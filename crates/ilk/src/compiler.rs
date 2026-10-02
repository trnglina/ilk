use std::{
    collections::{BTreeMap, HashMap},
    vec,
};

use functor_derive::Functor;
use thiserror::Error;

use crate::intern::InternTable;

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
    Regions {
        text: Option<(RegionId, u32, u32)>,
        source: Option<(SourceRegionId, u32, u32)>,
    },
    /// Term, extracted-text region, original-source region.
    Assertion(TermId, RegionId, SourceRegionId),

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
        text: RegionId,
        source: SourceRegionId,
        assertions: vec::IntoIter<TermId>,
        children: vec::IntoIter<(usize, SourceRegionId)>,
        descendants: vec::IntoIter<SourceRegionId>,
    },
}

struct OpenRegion {
    text_start: u32,
    source_start: u32,
    assertions: Vec<TermId>,
    children: Vec<(usize, SourceRegionId)>,
    descendants: Vec<SourceRegionId>,
}

pub struct Compiler<'s> {
    parser: ProseParser<'s>,
    term_table: TermTable,
    term_ids: HashMap<Term, TermId>,
    term_stack: Vec<Term>,
    source_region_map: BTreeMap<usize, OpenRegion>,
    current_source_region: Option<usize>,
    text_regions: InternTable<u32>,
    source_regions: InternTable<u32>,
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
            text_regions: InternTable::default(),
            source_regions: InternTable::default(),
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
                self.source_region_map.insert(
                    id,
                    OpenRegion {
                        text_start: self.text_offset,
                        source_start: u32::try_from(source_offset)
                            .expect("source offset fits in u32"),
                        assertions: Vec::new(),
                        children: Vec::new(),
                        descendants: Vec::new(),
                    },
                );
                self.current_source_region = Some(id);
                None
            }
            ProseChunk::RegionEnd { id, source_offset } => {
                let region = self
                    .source_region_map
                    .remove(&id)
                    .expect("well-formed source region");
                let source_end = u32::try_from(source_offset).expect("source offset fits in u32");
                let (text_id, new_text) = self
                    .text_regions
                    .intern_with_status(&[region.text_start, self.text_offset]);
                let (source_id, new_source) = self
                    .source_regions
                    .intern_with_status(&[region.source_start, source_end]);
                let text_id = RegionId::new(text_id);
                let source_id = SourceRegionId::new(source_id);
                for (_, outer) in self.source_region_map.range_mut(..id) {
                    while outer.children.last().is_some_and(|&(child, _)| child > id) {
                        outer.children.pop();
                    }
                    outer.children.push((id, source_id));
                    outer.descendants.push(source_id);
                }
                self.pending = Some(Pending::Close {
                    text: text_id,
                    source: source_id,
                    assertions: region.assertions.into_iter(),
                    children: region.children.into_iter(),
                    descendants: region.descendants.into_iter(),
                });
                Some(CompilerEvent::Regions {
                    text: new_text.then_some((text_id, region.text_start, self.text_offset)),
                    source: new_source.then_some((source_id, region.source_start, source_end)),
                })
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
                    self.source_region_map
                        .get_mut(&region)
                        .expect("open source region")
                        .assertions
                        .push(self.term_ids[&term]);
                    None
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
                text,
                source,
                assertions,
                children,
                descendants,
            } => {
                if let Some(term) = assertions.next() {
                    return Ok(Some(CompilerEvent::Assertion(term, *text, *source)));
                }
                if let Some((_, child)) = children.next() {
                    return Ok(Some(CompilerEvent::SourceParent(*source, child)));
                }
                if let Some(descendant) = descendants.next() {
                    return Ok(Some(CompilerEvent::SourceAncestor(*source, descendant)));
                }
            }
        }
        self.pending = None;
        Ok(None)
    }
}
