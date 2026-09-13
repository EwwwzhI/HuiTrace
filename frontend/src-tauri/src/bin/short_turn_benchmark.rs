use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use app_lib::diarization::service::extract_short_candidate_vad_events;
use app_lib::diarization::short_turn::{
    compute_candidate_recall, compute_duration_bucket_metrics, compute_metrics,
    compute_speaker_acceptance_metrics, CandidateRecallObservation, MeetingSpeakerPrototypeStore,
    ShortTurnCandidateExtractor, ShortTurnCandidateSource, ShortTurnEvaluationObservation,
    ShortTurnRefiner, TranscriptCandidateInput,
};
use app_lib::diarization::types::{
    AssignmentMethod, AudioSource, SegmentKind, SpeakerSegment, TranscriptTiming,
};
use clap::Parser;
use serde::Deserialize;

#[derive(Debug, Parser)]
struct Args {
    /// Folder containing manifest.jsonl and audio paths referenced by it.
    #[arg(long)]
    dataset: PathBuf,
}

#[derive(Debug, Deserialize)]
struct ManifestRow {
    audio_path: PathBuf,
    start_ms: i64,
    end_ms: i64,
    duration_bucket: String,
    ground_truth_kind: SegmentKind,
    ground_truth_speaker: Option<String>,
    #[serde(default)]
    transcript_text: String,
    #[serde(default)]
    transcript_start_ms: Option<i64>,
    #[serde(default)]
    transcript_end_ms: Option<i64>,
    #[serde(default)]
    asr_confidence: Option<f64>,
    #[serde(default)]
    diarizer_turns: Vec<ManifestTurn>,
    #[serde(default)]
    accepted_speakers: Vec<String>,
    #[serde(default)]
    expected_visible_speakers: Vec<String>,
    #[serde(default)]
    notes: String,
}

#[derive(Debug, Deserialize)]
struct ManifestTurn {
    start_ms: i64,
    end_ms: i64,
    speaker_key: String,
    #[serde(default)]
    confidence: Option<f64>,
}

fn overlap_ms(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> i64 {
    a_end.min(b_end).saturating_sub(a_start.max(b_start)).max(0)
}

fn read_manifest(dataset: &Path) -> Result<Vec<ManifestRow>> {
    let path = dataset.join("manifest.jsonl");
    let reader =
        BufReader::new(File::open(&path).with_context(|| format!("open {}", path.display()))?);
    reader
        .lines()
        .enumerate()
        .filter_map(|(index, line)| match line {
            Ok(line) if line.trim().is_empty() => None,
            Ok(line) => Some(
                serde_json::from_str(&line)
                    .with_context(|| format!("parse {} line {}", path.display(), index + 1)),
            ),
            Err(error) => Some(Err(error.into())),
        })
        .collect()
}

fn main() -> Result<()> {
    let args = Args::parse();
    let rows = read_manifest(&args.dataset)?;
    let extractor = ShortTurnCandidateExtractor::default();
    let refiner = ShortTurnRefiner::default();
    let mut observations = Vec::new();
    let mut recall = Vec::new();
    let mut transcript_false_new = Vec::new();
    let mut visible_false_new = Vec::new();

    for (index, row) in rows.iter().enumerate() {
        let audio = if row.audio_path.is_absolute() {
            row.audio_path.clone()
        } else {
            args.dataset.join(&row.audio_path)
        };
        let vad = extract_short_candidate_vad_events(&audio, AudioSource::Imported)
            .with_context(|| format!("short VAD for {} ({})", audio.display(), row.notes))?;
        let transcripts = if row.transcript_text.is_empty() {
            Vec::new()
        } else {
            vec![TranscriptCandidateInput {
                timing: TranscriptTiming {
                    id: format!("manifest-{index}"),
                    start_ms: row.transcript_start_ms.unwrap_or(row.start_ms),
                    end_ms: row.transcript_end_ms.unwrap_or(row.end_ms),
                    audio_source: AudioSource::Imported,
                },
                text: row.transcript_text.clone(),
                asr_confidence: row.asr_confidence,
            }]
        };
        let speakers: Vec<_> = row
            .diarizer_turns
            .iter()
            .map(|turn| SpeakerSegment {
                start_ms: turn.start_ms,
                end_ms: turn.end_ms,
                speaker_key: turn.speaker_key.clone(),
                speaker_confidence: turn.confidence,
                audio_source: AudioSource::Imported,
                provisional: false,
                revision: 1,
                segment_kind: SegmentKind::Speech,
                assignment_method: AssignmentMethod::Diarization,
                overlap: false,
            })
            .collect();
        let candidates = extractor.extract(&transcripts, &speakers, &vad);
        let found = candidates
            .iter()
            .filter(|candidate| {
                overlap_ms(
                    candidate.start_ms,
                    candidate.end_ms,
                    row.start_ms,
                    row.end_ms,
                ) > 0
            })
            .max_by_key(|candidate| {
                overlap_ms(
                    candidate.start_ms,
                    candidate.end_ms,
                    row.start_ms,
                    row.end_ms,
                )
            });
        let found_sources = found
            .map(|candidate| candidate.candidate_sources.clone())
            .unwrap_or_default();
        recall.push(CandidateRecallObservation {
            expected_short_event: row.end_ms.saturating_sub(row.start_ms) <= 1_200,
            found_sources,
        });
        let (predicted_kind, predicted_speaker) = found
            .map(|candidate| {
                let decision = refiner.refine(
                    candidate,
                    &MeetingSpeakerPrototypeStore::new(row.accepted_speakers.clone()),
                );
                (decision.kind, decision.speaker_key)
            })
            .unwrap_or((SegmentKind::Unknown, None));
        transcript_false_new.push(
            predicted_speaker
                .as_ref()
                .is_some_and(|speaker| !row.accepted_speakers.contains(speaker)),
        );
        visible_false_new.push(row.accepted_speakers.iter().any(|speaker| {
            !row.expected_visible_speakers.is_empty()
                && !row.expected_visible_speakers.contains(speaker)
        }));
        let duration_ms = row.end_ms.saturating_sub(row.start_ms) as u64;
        let _declared_bucket = &row.duration_bucket;
        observations.push((
            duration_ms,
            ShortTurnEvaluationObservation {
                expected_kind: row.ground_truth_kind.clone(),
                expected_speaker: row.ground_truth_speaker.clone(),
                predicted_kind,
                predicted_speaker,
                predicted_is_new_speaker: false,
                manual_override_violated: false,
            },
        ));
    }

    let overall = compute_metrics(
        &observations
            .iter()
            .map(|(_, observation)| observation.clone())
            .collect::<Vec<_>>(),
    );
    let report = serde_json::json!({
        "dataset": args.dataset,
        "sample_count": rows.len(),
        "overall": overall,
        "duration_buckets": compute_duration_bucket_metrics(&observations),
        "candidate_recall_by_source": compute_candidate_recall(&recall),
        "speaker_acceptance": compute_speaker_acceptance_metrics(&transcript_false_new, &visible_false_new),
        "candidate_sources": [
            ShortTurnCandidateSource::Transcript,
            ShortTurnCandidateSource::DiarizerTurn,
            ShortTurnCandidateSource::VadEvent,
        ],
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
