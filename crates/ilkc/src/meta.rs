use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use ilk::{
    IntoTerm, OperatorClass, OperatorConfig, OperatorDefinition, ParseTerm, Term, TermParser,
    TermTable,
};

struct ParsedOperatorDefinition {
    name: String,
    precedence: u16,
    class: OperatorClass,
}

#[derive(Default)]
pub struct Definitions(Vec<ParsedOperatorDefinition>);

impl Definitions {
    pub fn config(&self) -> Result<Option<OperatorConfig<'_>>> {
        let Definitions(vec) = self;
        if vec.is_empty() {
            return Ok(None);
        }

        let definitions: Vec<_> = vec.iter().map(ParsedOperatorDefinition::borrowed).collect();
        Ok(Some(OperatorConfig::new(&definitions)?))
    }
}

impl ParsedOperatorDefinition {
    fn borrowed(&self) -> OperatorDefinition<'_> {
        OperatorDefinition {
            name: &self.name,
            precedence: self.precedence,
            class: self.class,
        }
    }
}

pub fn load(paths: &[PathBuf]) -> Result<Definitions> {
    let mut definitions = Definitions::default();
    for path in paths {
        let source = fs::read_to_string(path)?;
        let mut parser = TermParser::standalone(&source, None);
        let mut terms = TermTable::new();
        let mut stack = Vec::new();
        let mut expression_start = None;
        while let Some(event) = parser.next() {
            let event = event?;
            expression_start = expression_start.or(parser.current_fact_start_offset());
            match event {
                ParseTerm::Atom(atom) => stack.push(atom.into_term(&mut terms)),
                ParseTerm::Number(number) => stack.push(number.into_term(&mut terms)),
                ParseTerm::Compound(atom, arity) => {
                    let Term::Atom(functor) = atom.into_term(&mut terms) else {
                        unreachable!()
                    };
                    let start = stack.len() - arity;
                    let compound = terms.compound(functor, &stack[start..]);
                    stack.truncate(start);
                    stack.push(Term::Compound(compound));
                }
                ParseTerm::FactEnd => {
                    let root = stack.pop().expect("completed expression");
                    debug_assert!(stack.is_empty());
                    let definition = read_operator_definition(root, &terms)?;
                    definitions.0.push(definition);
                    terms = TermTable::new();
                }
            }
        }
    }
    Ok(definitions)
}

fn read_operator_definition(root: Term, terms: &TermTable) -> Result<ParsedOperatorDefinition> {
    let Term::Compound(compound) = root else {
        bail!("expected op(Precedence, Type, Name)");
    };
    let (functor, args) = terms.compound_value(compound);
    if terms.atom_value(functor) != "op" {
        bail!(
            "unsupported declaration {:?}; expected op/3",
            terms.atom_value(functor)
        );
    }
    let [
        Term::Integer(precedence),
        Term::Atom(class),
        Term::Atom(name),
    ] = args
    else {
        bail!("expected op(Precedence, Type, Name) with an integer and two atoms");
    };
    let precedence = u16::try_from(precedence.value())
        .context("operator precedence must be between 1 and 1200")?;
    let class = match terms.atom_value(*class) {
        "xfx" => OperatorClass::Xfx,
        "xfy" => OperatorClass::Xfy,
        "yfx" => OperatorClass::Yfx,
        "fy" => OperatorClass::Fy,
        other => bail!("unsupported operator type {other:?}; expected xfx, xfy, yfx, or fy"),
    };
    let definition = ParsedOperatorDefinition {
        name: terms.atom_value(*name).to_owned(),
        precedence,
        class,
    };
    OperatorConfig::new(&[definition.borrowed()])?;
    Ok(definition)
}
