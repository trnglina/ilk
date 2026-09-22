mod compiler;

use std::{fs, path::PathBuf};

use anyhow::Result;
use clap::{Parser, ValueEnum};
use ilk::ProseParser;

use crate::compiler::Compiler;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    out_dir: PathBuf,
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

    Ok(())
}
