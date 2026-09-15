use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use app_lib::evaluation::utterance_reconstruction::run_benchmark_files;

fn main() -> Result<()> {
    let mut artifact = None;
    let mut ground_truth = None;
    let mut output = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--artifact" => artifact = args.next().map(PathBuf::from),
            "--ground-truth" => ground_truth = args.next().map(PathBuf::from),
            "--output" => output = args.next().map(PathBuf::from),
            "--help" | "-h" => {
                println!(
                    "utterance_reconstruction_benchmark --artifact ARTIFACT.json --ground-truth GT.json --output DIRECTORY"
                );
                return Ok(());
            }
            unknown => bail!("unknown argument: {unknown}"),
        }
    }
    let artifact = artifact.context("--artifact is required")?;
    let ground_truth = ground_truth.context("--ground-truth is required")?;
    let output = output.context("--output is required")?;
    let report = run_benchmark_files(&artifact, &ground_truth, &output)?;
    println!(
        "PASS meeting={} artifact={} report={}",
        report.meeting_id,
        report.artifact_id,
        output.display()
    );
    Ok(())
}
