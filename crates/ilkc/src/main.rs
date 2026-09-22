mod compiler;
mod writer;

use std::{fs, io, path::PathBuf};

use anyhow::Result;
use clap::{Parser, ValueEnum};
use ilk::ProseParser;

use crate::{compiler::Compiler, writer::souffle::SouffleWriter};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    out_dir: PathBuf,
    #[arg(short, long, default_value_t = true, default_missing_value = "true",
        num_args = 0..=1, require_equals = true, action = clap::ArgAction::Set)]
    clean: bool,
    #[arg(short, long, value_enum, default_value_t = Format::Souffle)]
    format: Format,

    file: PathBuf,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Format {
    Souffle,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let source = fs::read_to_string(&cli.file)?;
    let parser = ProseParser::new(&source);
    let compiler = Compiler::new(parser);
    let mut writer = match cli.format {
        Format::Souffle => SouffleWriter::new(compiler, &cli.out_dir),
    };

    if cli.clean
        && let Err(e) = fs::remove_dir_all(&cli.out_dir)
        && e.kind() != io::ErrorKind::NotFound
    {
        return Err(e.into());
    }

    fs::create_dir_all(&cli.out_dir)?;
    writer.run()
}
