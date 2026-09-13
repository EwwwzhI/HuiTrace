use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use app_lib::diarization::service::extract_short_candidate_vad_events;
use app_lib::diarization::short_turn::ShortTurnCandidateExtractor;
use app_lib::diarization::types::AudioSource;
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
    production_artifact: Option<PathBuf>,
    #[arg(long, default_value_t = 5_000)]
    window_ms: i64,
    #[arg(long, default_value_t = 4_000)]
    stride_ms: i64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.meeting_id.trim().is_empty() {
        bail!("--meeting-id must not be empty");
    }
    if args.window_ms <= 0 || args.stride_ms <= 0 || args.stride_ms > args.window_ms {
        bail!("window and stride must be positive, with stride <= window");
    }
    fs::create_dir_all(&args.output)
        .with_context(|| format!("create {}", args.output.display()))?;
    let vad = extract_short_candidate_vad_events(&args.audio, AudioSource::Imported)?;
    let candidates = ShortTurnCandidateExtractor::default().extract(&[], &[], &vad);
    let decoded = app_lib::audio::decoder::decode_audio_file(&args.audio)?;
    let samples = decoded.to_whisper_format();
    let audio_end_ms = samples.len() as i64 * 1_000 / 16_000;
    let blind_manifest_path = args.output.join("annotation_windows.blind.jsonl");
    let review_manifest_path = args.output.join("annotation_windows.review.jsonl");
    let mut blind_manifest = BufWriter::new(File::create(&blind_manifest_path)?);
    let mut review_manifest = BufWriter::new(File::create(&review_manifest_path)?);
    let artifact = args
        .production_artifact
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());

    let mut window_start = 0;
    let mut index = 1;
    while window_start < audio_end_ms {
        let window_end = (window_start + args.window_ms).min(audio_end_ms);
        let filename = format!("window_{index:04}.wav");
        write_clip(
            &args.output.join(&filename),
            &samples,
            window_start,
            window_end,
        )?;
        let suggestions = candidates
            .iter()
            .filter(|candidate| candidate.start_ms < window_end && candidate.end_ms > window_start)
            .map(|candidate| {
                serde_json::json!({
                    "start_ms": candidate.start_ms,
                    "end_ms": candidate.end_ms,
                    "candidate_sources": candidate.candidate_sources,
                    "vad_confidence": candidate.vad_confidence,
                    "label": "annotation_suggestion_not_ground_truth"
                })
            })
            .collect::<Vec<_>>();
        let common = serde_json::json!({
                "record_type": "annotation_window",
                "window_id": format!("{}-window-{index:04}", args.meeting_id),
                "meeting_id": args.meeting_id,
                "audio_path": filename,
                "source_start_ms": window_start,
                "source_end_ms": window_end,
                "production_artifact_path": artifact,
                "instructions": "Annotate every event on the source meeting timeline, including missed speech, noise, ordinary non-short controls, overlap, handoff, and uncertain cases. Write separate ground_truth_event rows to manifest.jsonl."
        });
        writeln!(blind_manifest, "{}", serde_json::to_string(&common)?)?;
        let mut review = common;
        review
            .as_object_mut()
            .expect("annotation window object")
            .insert(
                "candidate_suggestions".into(),
                serde_json::json!(suggestions),
            );
        writeln!(review_manifest, "{}", serde_json::to_string(&review)?)?;
        if window_end == audio_end_ms {
            break;
        }
        window_start += args.stride_ms;
        index += 1;
    }
    blind_manifest.flush()?;
    review_manifest.flush()?;
    eprintln!(
        "exported {index} blind windows to {} and suggestion-assisted review windows to {}; {} candidate suggestions are review hints only",
        blind_manifest_path.display(),
        review_manifest_path.display(),
        candidates.len()
    );
    Ok(())
}

fn write_clip(path: &PathBuf, samples: &[f32], start_ms: i64, end_ms: i64) -> Result<()> {
    let start = (start_ms as usize * 16_000 / 1_000).min(samples.len());
    let end = (end_ms as usize * 16_000 / 1_000).min(samples.len());
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for sample in &samples[start..end] {
        writer.write_sample(*sample)?;
    }
    writer.finalize()?;
    Ok(())
}
