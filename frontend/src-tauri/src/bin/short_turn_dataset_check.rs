use std::path::PathBuf;

use anyhow::Result;
use app_lib::evaluation::dataset::check_dataset;
use clap::Parser;

#[derive(Debug, Parser)]
struct Args {
    /// Dataset directory containing manifest.jsonl and local artifacts.
    #[arg(long)]
    dataset: PathBuf,
}

fn main() -> Result<()> {
    let report = check_dataset(&Args::parse().dataset)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
