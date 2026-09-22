use std::{
    collections::{BTreeMap, HashMap},
    ops::Bound,
    vec,
};

use functor_derive::Functor;
use thiserror::Error;

use crate::{
    AtomHandle, CompoundHandle, IntoTerm, ParseTerm, ProseChunk, ProseError, ProseParser, Term,
    TermTable,
};

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum CompilerError {
    #[error("{source} at {offset}")]
    Parse { offset: usize, source: ProseError },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionId(u32);

impl RegionId {
    pub fn new(id: usize) -> Self {
        Self(u32::try_from(id).expect("id fits in u32"))
    }

    pub fn value(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TermId(u32);

impl TermId {
    pub fn new(id: usize) -> Self {
        Self(u32::try_from(id).expect("id fits in u32"))
    }

    pub fn value(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Functor)]
pub enum CompilerEvent<'s, AtomT> {
    Text(&'s str),
    RegionStart(RegionId, u32),
    RegionEnd(RegionId, u32),
    Assertion(TermId, RegionId),
    Parent(RegionId, RegionId),
    Ancestor(RegionId, RegionId),
    Atom(TermId, AtomT),
    Integer(TermId, i32),
    Real(TermId, f32),
    Compound(TermId, TermId),
    Argument(TermId, u32, TermId),
}

struct CompoundFrame {
    functor: AtomHandle,
    args: Vec<Term>,
}

enum Pending {
    Arguments {
        id: TermId,
        compound: CompoundHandle,
        position: usize,
    },
    Close {
        id: RegionId,
        children: vec::IntoIter<RegionId>,
        last_outer: Option<RegionId>,
    },
}

pub struct Compiler<'s> {
    parser: ProseParser<'s>,
    term_table: TermTable,
    term_ids: HashMap<Term, TermId>,
    compound_stack: Vec<CompoundFrame>,
    region_map: BTreeMap<RegionId, Vec<RegionId>>,
    current_fact: Option<Term>,
    current_region: Option<RegionId>,
    text_offset: u32,
    pending: Option<Pending>,
    done: bool,
}

impl<'s> Compiler<'s> {
    pub fn new(parser: ProseParser<'s>) -> Self {
        Self {
            parser,
            term_table: TermTable::new(),
            term_ids: HashMap::new(),
            compound_stack: Vec::new(),
            region_map: BTreeMap::new(),
            current_fact: None,
            current_region: None,
            text_offset: 0,
            pending: None,
            done: false,
        }
    }

    pub fn next(&mut self) -> Option<Result<CompilerEvent<'s, &str>, CompilerError>> {
        loop {
            if self.done {
                return None;
            }

            match self.step() {
                Ok(Some(event)) => {
                    return Some(Ok(event.fmap(|atom| self.term_table.atom_value(atom))));
                }
                Ok(None) => {}
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
            }
        }
    }

    fn step(&mut self) -> Result<Option<CompilerEvent<'s, AtomHandle>>, CompilerError> {
        if let Some(event) = self.drain_pending()? {
            return Ok(Some(event));
        }

        let Some(chunk) = self.parser.next() else {
            debug_assert!(self.region_map.is_empty());
            debug_assert!(self.compound_stack.is_empty() && self.current_fact.is_none());

            self.done = true;
            return Ok(None);
        };

        let chunk = chunk.map_err(|source| CompilerError::Parse {
            offset: self.parser.offset(),
            source,
        })?;

        let event = self.generate_event(chunk)?;

        Ok(event)
    }

    fn generate_event(
        &mut self,
        chunk: ProseChunk<'s>,
    ) -> Result<Option<CompilerEvent<'s, AtomHandle>>, CompilerError> {
        Ok(match chunk {
            ProseChunk::Text(text) => {
                self.text_offset =
                    self.text_offset + u32::try_from(text.len()).expect("text offset fits in u32");
                Some(CompilerEvent::Text(text))
            }
            ProseChunk::RegionStart { id } => {
                let id = RegionId::new(id);
                self.region_map.insert(id, Vec::default());
                self.current_region = Some(id);
                Some(CompilerEvent::RegionStart(id, self.text_offset))
            }
            ProseChunk::RegionEnd { id } => {
                let id = RegionId::new(id);
                let children = self.region_map.remove(&id).expect("well-formed region");
                self.pending = Some(Pending::Close {
                    id,
                    children: children.into_iter(),
                    last_outer: None,
                });
                Some(CompilerEvent::RegionEnd(id, self.text_offset))
            }
            ProseChunk::TermEvent(event) => match event {
                ParseTerm::CompoundStart(atom) => {
                    let term = atom.into_term(&mut self.term_table);
                    let Term::Atom(functor) = term else {
                        unreachable!()
                    };
                    self.compound_stack.push(CompoundFrame {
                        functor,
                        args: Vec::new(),
                    });
                    self.generate_term_event(term)?
                }
                ParseTerm::Atom(atom) => {
                    let term = atom.into_term(&mut self.term_table);
                    self.push_term(term);
                    self.generate_term_event(term)?
                }
                ParseTerm::Number(number) => {
                    let term = number.into_term(&mut self.term_table);
                    self.push_term(term);
                    self.generate_term_event(term)?
                }
                ParseTerm::CompoundEnd => {
                    let frame = self.compound_stack.pop().expect("well-formed compound");
                    let term = Term::Compound(self.term_table.compound(frame.functor, &frame.args));
                    self.push_term(term);
                    self.generate_term_event(term)?
                }
                ParseTerm::FactEnd => {
                    debug_assert!(self.compound_stack.is_empty());
                    let term = self.current_fact.take().expect("fact has a root");
                    let region = self.current_region.expect("fact has a region");
                    Some(CompilerEvent::Assertion(self.term_ids[&term], region))
                }
            },
        })
    }

    fn generate_term_event(
        &mut self,
        term: Term,
    ) -> Result<Option<CompilerEvent<'s, AtomHandle>>, CompilerError> {
        if self.term_ids.contains_key(&term) {
            return Ok(None);
        }

        let id = TermId::new(self.term_ids.len());

        self.term_ids.insert(term, id);

        Ok(Some(match term {
            Term::Atom(atom) => CompilerEvent::Atom(id, atom),
            Term::Integer(value) => CompilerEvent::Integer(id, value.value()),
            Term::Real(value) => CompilerEvent::Real(id, value.value()),
            Term::Compound(compound) => {
                let (functor, _) = self.term_table.compound_value(compound);
                let functor = self.term_ids[&Term::Atom(functor)];
                self.pending = Some(Pending::Arguments {
                    id,
                    compound,
                    position: 0,
                });
                CompilerEvent::Compound(id, functor)
            }
        }))
    }

    fn drain_pending(&mut self) -> Result<Option<CompilerEvent<'s, AtomHandle>>, CompilerError> {
        let Some(pending) = self.pending.as_mut() else {
            return Ok(None);
        };

        match pending {
            Pending::Arguments {
                id,
                compound,
                position,
            } => {
                let (_, args) = self.term_table.compound_value(*compound);
                if let Some(term) = args.get(*position) {
                    let index = u32::try_from(*position).expect("arg index fits in u32");
                    *position += 1;
                    return Ok(Some(CompilerEvent::Argument(
                        *id,
                        index,
                        self.term_ids[term],
                    )));
                }
            }
            Pending::Close {
                id,
                children,
                last_outer,
            } => {
                if let Some(child) = children.next() {
                    return Ok(Some(CompilerEvent::Parent(*id, child)));
                }
                let lower = last_outer.map_or(Bound::Unbounded, Bound::Excluded);
                if let Some((&outer, children)) = self
                    .region_map
                    .range_mut((lower, Bound::Excluded(*id)))
                    .next()
                {
                    while children.last().is_some_and(|child| child > id) {
                        children.pop();
                    }
                    children.push(*id);
                    *last_outer = Some(outer);
                    return Ok(Some(CompilerEvent::Ancestor(outer, *id)));
                }
            }
        }
        self.pending = None;
        Ok(None)
    }

    fn push_term(&mut self, term: Term) {
        if let Some(frame) = self.compound_stack.last_mut() {
            frame.args.push(term);
        } else {
            debug_assert!(self.current_fact.is_none());
            self.current_fact = Some(term);
        }
    }
}
