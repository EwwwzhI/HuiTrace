use std::path::PathBuf;

use anyhow::Result;
use app_lib::evaluation::short_turn_export::{
    export_annotation_windows, ShortTurnExportRequest, DEFAULT_STRIDE_MS, DEFAULT_WINDOW_MS,
};
use clap::Parser;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    audio: PathBuf,
    #[arg(long)]
    meeting_id: String,
    /// Local-only output; use the gitignored dataset/local directory.
    #[arg(long)]
    output: PathBuf,
    /// JSON exported by a real app run. It should contain transcripts, all raw
    /// diarizer turns, VAD events, and accepted/visible speakers.
    #[arg(long)]
    production_artifact: PathBuf,
    #[arg(long, default_value_t = DEFAULT_WINDOW_MS)]
    window_ms: i64,
    #[arg(long, default_value_t = DEFAULT_STRIDE_MS)]
    stride_ms: i64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let result = export_annotation_windows(ShortTurnExportRequest {
        audio: args.audio,
        meeting_id: args.meeting_id,
        output: args.output,
        production_artifact: args.production_artifact,
        window_ms: args.window_ms,
        stride_ms: args.stride_ms,
        // Preserve the CLI's existing automation behavior. The interactive
        // Tauri preparation flow applies stricter no-overwrite guards.
        overwrite_existing: true,
    })?;
    eprintln!(
        "exported {} blind windows to {} and suggestion-assisted review windows to {}; {} candidate suggestions are review hints only",
        result.window_count,
        result.blind_manifest.display(),
        result.review_manifest.display(),
        result.candidate_count
    );
    Ok(())
}
