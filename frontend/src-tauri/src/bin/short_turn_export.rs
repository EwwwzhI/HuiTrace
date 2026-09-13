use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use app_lib::diarization::service::extract_short_candidate_vad_events;
use app_lib::diarization::short_turn::{
    MeetingSpeakerPrototypeStore, ShortTurnCandidateExtractor, ShortTurnRefiner,
};
use app_lib::diarization::types::AudioSource;
use clap::Parser;

#[derive(Debug, Parser)]
struct Args {
    /// Meeting audio to scan locally.
    #[arg(long)]
    audio: PathBuf,
    /// Local-only export directory (recommended: evaluation/short_turn_dataset/local).
    #[arg(long)]
    output: PathBuf,
    /// Context retained before and after each candidate.
    #[arg(long, default_value_t = 400)]
    context_ms: i64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    fs::create_dir_all(&args.output)
        .with_context(|| format!("create {}", args.output.display()))?;
    let vad = extract_short_candidate_vad_events(&args.audio, AudioSource::Imported)?;
    let candidates = ShortTurnCandidateExtractor::default().extract(&[], &[], &vad);
    let decoded = app_lib::audio::decoder::decode_audio_file(&args.audio)?;
    let samples = decoded.to_whisper_format();
    let manifest_path = args.output.join("manifest.template.jsonl");
    let mut manifest = BufWriter::new(File::create(&manifest_path)?);
    let refiner = ShortTurnRefiner::default();
    let prototypes = MeetingSpeakerPrototypeStore::default();

    for (index, candidate) in candidates.iter().enumerate() {
        let clip_start_ms = candidate.start_ms.saturating_sub(args.context_ms).max(0);
        let audio_end_ms = samples.len() as i64 * 1_000 / 16_000;
        let clip_end_ms = candidate
            .end_ms
            .saturating_add(args.context_ms)
            .min(audio_end_ms);
        let start_sample = (clip_start_ms as usize * 16_000 / 1_000).min(samples.len());
        let end_sample = (clip_end_ms as usize * 16_000 / 1_000).min(samples.len());
        let filename = format!("candidate_{:04}.wav", index + 1);
        let clip_path = args.output.join(&filename);
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&clip_path, spec)?;
        for sample in &samples[start_sample..end_sample] {
            writer.write_sample(*sample)?;
        }
        writer.finalize()?;

        let decision = refiner.refine(candidate, &prototypes);
        let local_start_ms = candidate.start_ms - clip_start_ms;
        let local_end_ms = candidate.end_ms - clip_start_ms;
        let duration_bucket = match candidate.duration_ms {
            100..300 => "100-300ms",
            300..500 => "300-500ms",
            500..800 => "500-800ms",
            _ => "800-1200ms",
        };
        let row = serde_json::json!({
            "meeting_id": null,
            "audio_path": filename,
            "start_ms": local_start_ms,
            "end_ms": local_end_ms,
            "event_start_ms": local_start_ms,
            "event_end_ms": local_end_ms,
            "source_start_ms": candidate.start_ms,
            "source_end_ms": candidate.end_ms,
            "duration_bucket": duration_bucket,
            "candidate_sources": candidate.candidate_sources,
            "predicted_kind": decision.kind,
            "predicted_speaker": decision.speaker_key,
            "ground_truth_kind": null,
            "ground_truth_speaker": null,
            "tags": [],
            "notes": ""
        });
        writeln!(manifest, "{}", serde_json::to_string(&row)?)?;
    }
    manifest.flush()?;
    eprintln!(
        "exported {} candidates and {}",
        candidates.len(),
        manifest_path.display()
    );
    Ok(())
}
