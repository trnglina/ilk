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
pub struct AssertionId(u32);

impl AssertionId {
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
    Assertion(AssertionId, TermId),
    AssertionRegion(AssertionId, u32, RegionId),
    Parent(RegionId, RegionId),
    Ancestor(RegionId, RegionId),
    Atom(TermId, AtomT),
    Integer(TermId, i32),
    Real(TermId, f32),
    Compound(TermId, TermId),
    CompoundArgument(TermId, u32, TermId),
    Sequence(TermId, u32),
    SequenceElement(TermId, u32, TermId),
}

struct Annotation {
    assertions: Vec<AssertionId>,
    region: RegionId,
    index: u32,
}

#[derive(Clone, Copy)]
enum AfterRegionClose {
    Read,
    ContinueAnnotation(usize),
}

enum CompilerState {
    Reading,
    EmittingAssertionRegion(AssertionId, u32, RegionId),
    EmittingAssociations {
        annotation: usize,
        position: usize,
    },
    EmittingCompound(Term),
    EmittingArguments {
        id: TermId,
        compound: CompoundHandle,
        position: usize,
    },
    EmittingElements {
        id: TermId,
        sequence: SequenceHandle,
        position: usize,
    },
    ClosingRegion {
        id: RegionId,
        children: vec::IntoIter<RegionId>,
        last_outer: Option<RegionId>,
        continuation: AfterRegionClose,
    },
    Done,
}

pub struct Compiler<'s> {
    parser: ProseParser<'s>,
    term_table: TermTable,
    term_ids: HashMap<Term, TermId>,
    term_stack: Vec<Term>,
    region_map: BTreeMap<RegionId, Vec<RegionId>>,
    annotations: HashMap<usize, Annotation>,
    current_annotation: Option<usize>,
    next_region_id: usize,
    next_assertion_id: usize,
    text_offset: u32,
    state: CompilerState,
}

impl<'s> Compiler<'s> {
    pub fn new(parser: ProseParser<'s>) -> Self {
        Self {
            parser,
            term_table: TermTable::new(),
            term_ids: HashMap::new(),
            term_stack: Vec::new(),
            region_map: BTreeMap::new(),
            annotations: HashMap::new(),
            current_annotation: None,
            next_region_id: 0,
            next_assertion_id: 0,
            text_offset: 0,
            state: CompilerState::Reading,
        }
    }

    pub fn next(&mut self) -> Option<Result<CompilerEvent<'s, &str>, CompilerError>> {
        loop {
            if matches!(self.state, CompilerState::Done) {
                return None;
            }

            match self.step() {
                Ok(Some(event)) => {
                    return Some(Ok(event.fmap(|atom| self.term_table.atom_value(atom))));
                }
                Ok(None) => {}
                Err(error) => {
                    self.state = CompilerState::Done;
                    return Some(Err(error));
                }
            }
        }
    }

    fn read_chunk(&mut self) -> Result<Option<CompilerEvent<'s, AtomHandle>>, CompilerError> {
        let Some(chunk) = self.parser.next() else {
            debug_assert!(self.region_map.is_empty());
            debug_assert!(self.annotations.is_empty());
            debug_assert!(self.term_stack.is_empty());

            self.state = CompilerState::Done;
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
            ProseChunk::AnnotationStart { id } => {
                let region = self.open_region();
                self.annotations.insert(
                    id,
                    Annotation {
                        assertions: Vec::new(),
                        region,
                        index: 0,
                    },
                );
                self.current_annotation = Some(id);
                Some(CompilerEvent::RegionStart(region, self.text_offset))
            }
            ProseChunk::AnnotationEnd { id } => {
                let annotation = self
                    .annotations
                    .remove(&id)
                    .expect("well-formed annotation");
                Some(self.close_region(annotation.region, AfterRegionClose::Read))
            }
            ProseChunk::AnnotationSplit { id } => {
                let region = self.annotations[&id].region;
                Some(self.close_region(region, AfterRegionClose::ContinueAnnotation(id)))
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
                        self.state = CompilerState::EmittingCompound(term);
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
                    let id = AssertionId::new(self.next_assertion_id);
                    self.next_assertion_id += 1;
                    let annotation = self
                        .annotations
                        .get_mut(&self.current_annotation.expect("fact has an annotation"))
                        .expect("open annotation");
                    annotation.assertions.push(id);
                    self.state = CompilerState::EmittingAssertionRegion(
                        id,
                        annotation.index,
                        annotation.region,
                    );
                    Some(CompilerEvent::Assertion(id, self.term_ids[&term]))
                }
            },
        })
    }

    fn open_region(&mut self) -> RegionId {
        let id = RegionId::new(self.next_region_id);
        self.next_region_id += 1;
        self.region_map.insert(id, Vec::new());
        id
    }

    fn close_region(
        &mut self,
        id: RegionId,
        continuation: AfterRegionClose,
    ) -> CompilerEvent<'s, AtomHandle> {
        let children = self.region_map.remove(&id).expect("well-formed region");
        self.state = CompilerState::ClosingRegion {
            id,
            children: children.into_iter(),
            last_outer: None,
            continuation,
        };
        CompilerEvent::RegionEnd(id, self.text_offset)
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
                self.state = CompilerState::EmittingElements {
                    id,
                    sequence,
                    position: 0,
                };
                CompilerEvent::Sequence(id, length)
            }
            Term::Compound(compound) => {
                let (functor, _) = self.term_table.compound_value(compound);
                let functor = self.term_ids[&Term::Atom(functor)];
                self.state = CompilerState::EmittingArguments {
                    id,
                    compound,
                    position: 0,
                };
                CompilerEvent::Compound(id, functor)
            }
        }))
    }

    fn step(&mut self) -> Result<Option<CompilerEvent<'s, AtomHandle>>, CompilerError> {
        match &mut self.state {
            CompilerState::Reading => return self.read_chunk(),
            CompilerState::Done => return Ok(None),
            CompilerState::EmittingAssertionRegion(id, index, region) => {
                let event = CompilerEvent::AssertionRegion(*id, *index, *region);
                self.state = CompilerState::Reading;
                return Ok(Some(event));
            }
            CompilerState::EmittingCompound(term) => {
                let term = *term;
                self.state = CompilerState::Reading;
                return self.generate_term_event(term);
            }
            CompilerState::EmittingAssociations {
                annotation,
                position,
            } => {
                let annotation = &self.annotations[annotation];
                if let Some(&id) = annotation.assertions.get(*position) {
                    *position += 1;
                    return Ok(Some(CompilerEvent::AssertionRegion(
                        id,
                        annotation.index,
                        annotation.region,
                    )));
                }
            }
            CompilerState::EmittingArguments {
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
            CompilerState::EmittingElements {
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
            CompilerState::ClosingRegion {
                id,
                children,
                last_outer,
                continuation,
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
                if let AfterRegionClose::ContinueAnnotation(annotation_id) = *continuation {
                    let region = self.open_region();
                    let annotation = self
                        .annotations
                        .get_mut(&annotation_id)
                        .expect("open annotation");
                    annotation.region = region;
                    annotation.index = annotation
                        .index
                        .checked_add(1)
                        .expect("region index fits in u32");
                    self.state = CompilerState::EmittingAssociations {
                        annotation: annotation_id,
                        position: 0,
                    };
                    return Ok(Some(CompilerEvent::RegionStart(region, self.text_offset)));
                }
            }
        }
        self.state = CompilerState::Reading;
        Ok(None)
    }
}
