use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
};

use anyhow::Result;

use crate::compiler::{Compiler, Event};

pub struct SouffleWriter<'s, 'o> {
    compiler: Compiler<'s>,
    out_dir: &'o Path,
}

impl<'s, 'o> SouffleWriter<'s, 'o> {
    pub fn new(compiler: Compiler<'s>, out_dir: &'o Path) -> Self {
        Self { compiler, out_dir }
    }

    pub fn run(&mut self) -> Result<()> {
        let mut text = create_writer(self.out_dir, "text.txt")?;
        let mut region_start_table = create_writer(self.out_dir, "region_start.facts")?;
        let mut region_end_table = create_writer(self.out_dir, "region_end.facts")?;
        let mut assertion_table = create_writer(self.out_dir, "assertion.facts")?;
        let mut parent_table = create_writer(self.out_dir, "parent.facts")?;
        let mut ancestor_table = create_writer(self.out_dir, "ancestor.facts")?;
        let mut atom_table = create_writer(self.out_dir, "atom.facts")?; // .input atom(rfc4180=true)
        let mut integer_table = create_writer(self.out_dir, "integer.facts")?;
        let mut real_table = create_writer(self.out_dir, "real.facts")?;
        let mut compound_table = create_writer(self.out_dir, "compound.facts")?;
        let mut argument_table = create_writer(self.out_dir, "argument.facts")?;

        while let Some(event) = self.compiler.next() {
            match event? {
                Event::Text(chunk) => text.write_all(chunk.as_bytes())?,
                Event::RegionStart(id, offset) => {
                    writeln!(region_start_table, "{}\t{offset}", id.value())?
                }
                Event::RegionEnd(id, offset) => {
                    writeln!(region_end_table, "{}\t{offset}", id.value())?
                }
                Event::Assertion(term, region) => {
                    writeln!(assertion_table, "{}\t{}", term.value(), region.value())?
                }
                Event::Parent(outer, inner) => {
                    writeln!(parent_table, "{}\t{}", outer.value(), inner.value())?
                }
                Event::Ancestor(outer, inner) => {
                    writeln!(ancestor_table, "{}\t{}", outer.value(), inner.value())?
                }
                Event::Atom(id, value) => {
                    write!(atom_table, "{},", id.value())?;
                    write_symbol(&mut atom_table, value)?;
                    atom_table.write_all(b"\n")?;
                }
                Event::Integer(id, value) => writeln!(integer_table, "{}\t{value}", id.value())?,
                Event::Real(id, value) => writeln!(real_table, "{}\t{value}", id.value())?,
                Event::Compound(id, functor) => {
                    writeln!(compound_table, "{}\t{}", id.value(), functor.value())?
                }
                Event::Argument(id, position, term) => writeln!(
                    argument_table,
                    "{}\t{position}\t{}",
                    id.value(),
                    term.value()
                )?,
            }
        }

        for sink in [
            &mut text,
            &mut region_start_table,
            &mut region_end_table,
            &mut assertion_table,
            &mut parent_table,
            &mut ancestor_table,
            &mut atom_table,
            &mut integer_table,
            &mut real_table,
            &mut compound_table,
            &mut argument_table,
        ] {
            Write::flush(sink)?;
        }
        Ok(())
    }
}

fn create_writer(out_dir: &Path, name: &str) -> Result<BufWriter<File>> {
    let path = out_dir.join(name);
    let file = File::create(&path)?;
    Ok(BufWriter::new(file))
}

fn write_symbol(out: &mut impl Write, value: &str) -> io::Result<()> {
    if !value.is_empty() && !value.contains([',', '\t', '\r', '\n', '"']) {
        return out.write_all(value.as_bytes());
    }
    out.write_all(b"\"")?;
    let mut start = 0;
    for (index, _) in value.match_indices('"') {
        out.write_all(&value.as_bytes()[start..index])?;
        out.write_all(b"\"\"")?;
        start = index + 1;
    }
    out.write_all(&value.as_bytes()[start..])?;
    out.write_all(b"\"")
}
