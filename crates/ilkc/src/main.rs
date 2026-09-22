use std::{fs, path::PathBuf};

use anyhow::Result;
use clap::{Parser, ValueEnum};

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

    Ok(())
}
