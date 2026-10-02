use std::{
    collections::{BTreeMap, HashMap},
    ops::Bound,
    vec,
};

use functor_derive::Functor;
use thiserror::Error;

use crate::{
    AtomHandle, CompoundHandle, IntoTerm, ParseTerm, ProseChunk, ProseError, ProseParser,
    SequenceHandle, Term, TermTable,
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
pub struct SourceRegionId(u32);

impl SourceRegionId {
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
    RegionStart(RegionId, u32, u32),
    RegionEnd(RegionId, u32, u32),
    Assertion(TermId, RegionId),

    SourceParent(SourceRegionId, SourceRegionId),
    SourceAncestor(SourceRegionId, SourceRegionId),
    Atom(TermId, AtomT),
    Integer(TermId, i32),
    Real(TermId, f32),
    Compound(TermId, TermId),
    CompoundArgument(TermId, u32, TermId),
    Sequence(TermId, u32),
    SequenceElement(TermId, u32, TermId),
}

enum Pending {
    Compound(Term),
    Arguments {
        id: TermId,
        compound: CompoundHandle,
        position: usize,
    },
    Elements {
        id: TermId,
        sequence: SequenceHandle,
        position: usize,
    },
    Close {
        id: SourceRegionId,
        children: vec::IntoIter<SourceRegionId>,
        last_outer: Option<SourceRegionId>,
    },
}

pub struct Compiler<'s> {
    parser: ProseParser<'s>,
    term_table: TermTable,
    term_ids: HashMap<Term, TermId>,
    term_stack: Vec<Term>,
    source_region_map: BTreeMap<SourceRegionId, Vec<SourceRegionId>>,
    current_source_region: Option<SourceRegionId>,
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
            term_stack: Vec::new(),
            source_region_map: BTreeMap::new(),
            current_source_region: None,
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
            debug_assert!(self.source_region_map.is_empty());
            debug_assert!(self.term_stack.is_empty());

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
            ProseChunk::RegionStart { id, source_offset } => {
                let source_id = SourceRegionId::new(id);
                self.source_region_map.insert(source_id, Vec::default());
                self.current_source_region = Some(source_id);
                Some(CompilerEvent::RegionStart(
                    RegionId::new(id),
                    self.text_offset,
                    u32::try_from(source_offset).expect("source offset fits in u32"),
                ))
            }
            ProseChunk::RegionEnd { id, source_offset } => {
                let source_id = SourceRegionId::new(id);
                let children = self
                    .source_region_map
                    .remove(&source_id)
                    .expect("well-formed source region");
                self.pending = Some(Pending::Close {
                    id: source_id,
                    children: children.into_iter(),
                    last_outer: None,
                });
                Some(CompilerEvent::RegionEnd(
                    RegionId::new(id),
                    self.text_offset,
                    u32::try_from(source_offset).expect("source offset fits in u32"),
                ))
            }
            ProseChunk::TermEvent(event) => match event {
                ParseTerm::Atom(atom) => {
                    let term = atom.into_term(&mut self.term_table);
                    self.term_stack.push(term);
                    self.generate_term_event(term)?
                }
                ParseTerm::Number(number) => {
                    let term = number.into_term(&mut self.term_table);
                    self.term_stack.push(term);
                    self.generate_term_event(term)?
                }
                ParseTerm::Compound(atom, arity) => {
                    let Term::Atom(functor) = atom.into_term(&mut self.term_table) else {
                        unreachable!()
                    };
                    let start = self.term_stack.len() - arity;
                    let term = Term::Compound(
                        self.term_table.compound(functor, &self.term_stack[start..]),
                    );
                    self.term_stack.truncate(start);
                    self.term_stack.push(term);
                    if let Some(event) = self.generate_term_event(Term::Atom(functor))? {
                        self.pending = Some(Pending::Compound(term));
                        Some(event)
                    } else {
                        self.generate_term_event(term)?
                    }
                }
                ParseTerm::Sequence(length) => {
                    let start = self.term_stack.len() - length;
                    let term = Term::Sequence(self.term_table.sequence(&self.term_stack[start..]));
                    self.term_stack.truncate(start);
                    self.term_stack.push(term);
                    self.generate_term_event(term)?
                }
                ParseTerm::FactEnd => {
                    let term = self.term_stack.pop().expect("fact has a root");
                    debug_assert!(self.term_stack.is_empty());
                    let region = self
                        .current_source_region
                        .expect("fact has a source region");
                    Some(CompilerEvent::Assertion(
                        self.term_ids[&term],
                        RegionId::new(region.value() as usize),
                    ))
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
            Term::Sequence(sequence) => {
                let length = u32::try_from(self.term_table.sequence_value(sequence).len())
                    .expect("sequence length fits in u32");
                self.pending = Some(Pending::Elements {
                    id,
                    sequence,
                    position: 0,
                });
                CompilerEvent::Sequence(id, length)
            }
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
        if let Some(Pending::Compound(term)) = self.pending {
            self.pending = None;
            return self.generate_term_event(term);
        }
        let Some(pending) = self.pending.as_mut() else {
            return Ok(None);
        };

        match pending {
            Pending::Compound(_) => unreachable!(),
            Pending::Arguments {
                id,
                compound,
                position,
            } => {
                let (_, args) = self.term_table.compound_value(*compound);
                if let Some(term) = args.get(*position) {
                    let index = u32::try_from(*position).expect("arg index fits in u32");
                    *position += 1;
                    return Ok(Some(CompilerEvent::CompoundArgument(
                        *id,
                        index,
                        self.term_ids[term],
                    )));
                }
            }
            Pending::Elements {
                id,
                sequence,
                position,
            } => {
                let elements = self.term_table.sequence_value(*sequence);
                if let Some(term) = elements.get(*position) {
                    let index = u32::try_from(*position).expect("element index fits in u32");
                    *position += 1;
                    return Ok(Some(CompilerEvent::SequenceElement(
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
                    return Ok(Some(CompilerEvent::SourceParent(*id, child)));
                }
                let lower = last_outer.map_or(Bound::Unbounded, Bound::Excluded);
                if let Some((&outer, children)) = self
                    .source_region_map
                    .range_mut((lower, Bound::Excluded(*id)))
                    .next()
                {
                    while children.last().is_some_and(|child| child > id) {
                        children.pop();
                    }
                    children.push(*id);
                    *last_outer = Some(outer);
                    return Ok(Some(CompilerEvent::SourceAncestor(outer, *id)));
                }
            }
        }
        self.pending = None;
        Ok(None)
    }
}
