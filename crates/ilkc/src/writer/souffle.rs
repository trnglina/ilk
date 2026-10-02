use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
};

use anyhow::Result;
use ilk::{Compiler, CompilerEvent};

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
        let mut source_region_start_table =
            create_writer(self.out_dir, "source_region_start.facts")?;
        let mut source_region_end_table = create_writer(self.out_dir, "source_region_end.facts")?;
        let mut assertion_table = create_writer(self.out_dir, "assertion.facts")?;
        let mut parent_table = create_writer(self.out_dir, "source_parent.facts")?;
        let mut ancestor_table = create_writer(self.out_dir, "source_ancestor.facts")?;
        let mut atom_table = create_writer(self.out_dir, "atom.facts")?; // .input atom(rfc4180=true)
        let mut integer_table = create_writer(self.out_dir, "integer.facts")?;
        let mut real_table = create_writer(self.out_dir, "real.facts")?;
        let mut compound_table = create_writer(self.out_dir, "compound.facts")?;
        let mut compound_argument_table = create_writer(self.out_dir, "compound_argument.facts")?;

        let mut sequence_table = create_writer(self.out_dir, "sequence.facts")?;
        let mut sequence_element_table = create_writer(self.out_dir, "sequence_element.facts")?;

        while let Some(event) = self.compiler.next() {
            match event? {
                CompilerEvent::Text(chunk) => text.write_all(chunk.as_bytes())?,
                CompilerEvent::RegionStart(id, text_offset, source_offset) => {
                    writeln!(region_start_table, "{}\t{text_offset}", id.value())?;
                    writeln!(source_region_start_table, "{}\t{source_offset}", id.value())?;
                }
                CompilerEvent::RegionEnd(id, text_offset, source_offset) => {
                    writeln!(region_end_table, "{}\t{text_offset}", id.value())?;
                    writeln!(source_region_end_table, "{}\t{source_offset}", id.value())?;
                }
                CompilerEvent::Assertion(term, region) => {
                    writeln!(assertion_table, "{}\t{}", term.value(), region.value())?
                }
                CompilerEvent::SourceParent(outer, inner) => {
                    writeln!(parent_table, "{}\t{}", outer.value(), inner.value())?
                }
                CompilerEvent::SourceAncestor(outer, inner) => {
                    writeln!(ancestor_table, "{}\t{}", outer.value(), inner.value())?
                }
                CompilerEvent::Atom(id, value) => {
                    write!(atom_table, "{},", id.value())?;
                    write_symbol(&mut atom_table, value)?;
                    atom_table.write_all(b"\n")?;
                }
                CompilerEvent::Integer(id, value) => {
                    writeln!(integer_table, "{}\t{value}", id.value())?
                }
                CompilerEvent::Real(id, value) => writeln!(real_table, "{}\t{value}", id.value())?,
                CompilerEvent::Compound(id, functor) => {
                    writeln!(compound_table, "{}\t{}", id.value(), functor.value())?
                }
                CompilerEvent::Sequence(id, length) => {
                    writeln!(sequence_table, "{}\t{length}", id.value())?
                }
                CompilerEvent::SequenceElement(id, position, term) => writeln!(
                    sequence_element_table,
                    "{}\t{position}\t{}",
                    id.value(),
                    term.value()
                )?,
                CompilerEvent::CompoundArgument(id, position, term) => writeln!(
                    compound_argument_table,
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
            &mut source_region_start_table,
            &mut source_region_end_table,
            &mut assertion_table,
            &mut parent_table,
            &mut ancestor_table,
            &mut atom_table,
            &mut integer_table,
            &mut real_table,
            &mut compound_table,
            &mut compound_argument_table,
            &mut sequence_table,
            &mut sequence_element_table,
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
