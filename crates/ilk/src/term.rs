use std::hash::Hash;

use crate::intern::InternTable;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IntegerTerm(i32);

impl IntegerTerm {
    pub fn new(value: i32) -> Self {
        Self(value)
    }

    pub fn value(&self) -> i32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RealTerm(f32);

impl RealTerm {
    pub fn new(value: f32) -> Self {
        assert!(value.is_finite(), "reals must be finite");
        Self(if value == 0.0 { 0.0f32 } else { value })
    }

    pub fn value(&self) -> f32 {
        self.0
    }
}

impl Eq for RealTerm {}

impl Hash for RealTerm {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AtomHandle(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CompoundHandle(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Term {
    Integer(IntegerTerm),
    Real(RealTerm),
    Atom(AtomHandle),
    Compound(CompoundHandle),
}

#[derive(Default)]
pub struct TermTable {
    atoms: InternTable<u8>,
    terms: InternTable<Term>,
}

impl TermTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn atom(&mut self, atom: &str) -> AtomHandle {
        AtomHandle(self.atoms.intern(atom.as_bytes()))
    }

    pub fn compound(&mut self, functor: AtomHandle, args: &[Term]) -> CompoundHandle {
        assert!(!args.is_empty(), "a compound must have arguments");
        let mut body = Vec::with_capacity(args.len() + 1);
        body.push(Term::Atom(functor));
        body.extend_from_slice(args);
        CompoundHandle(self.terms.intern(&body))
    }

    pub fn atom_value(&self, atom: AtomHandle) -> &str {
        let bytes = self.atoms.resolve(atom.0);
        unsafe { str::from_utf8_unchecked(bytes) }
    }

    pub fn compound_value(&self, compound: CompoundHandle) -> (AtomHandle, &[Term]) {
        let body = self.terms.resolve(compound.0);
        let Term::Atom(functor) = body[0] else {
            panic!("invalid stored compound")
        };

        (functor, &body[1..])
    }
}

pub trait IntoTerm {
    fn into_term(self, terms: &mut TermTable) -> Term;
}
