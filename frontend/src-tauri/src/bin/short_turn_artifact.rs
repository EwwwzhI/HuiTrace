use std::path::PathBuf;

use anyhow::Result;
use app_lib::evaluation::production_artifact::read_and_validate_artifact;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate one schema-v1 production artifact.
    Validate { path: PathBuf },
}

fn main() -> Result<()> {
    match Args::parse().command {
        Command::Validate { path } => {
            let artifact = read_and_validate_artifact(&path, None)?;
            println!(
                "valid schema={} meeting_id={} artifact_id={}",
                artifact.schema_version, artifact.meeting_id, artifact.artifact_id
            );
        }
    }
    Ok(())
}
