use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use app_lib::diarization::short_turn::{
    candidate_matches_ground_truth, compute_candidate_recall, compute_duration_bucket_metrics,
    compute_materialization_metrics, compute_metrics, compute_speaker_acceptance_metrics,
    CandidateMatchConfig, CandidateRecallObservation, MaterializationObservation,
    MeetingSpeakerPrototypeStore, ShortTurnCandidateExtractor, ShortTurnCandidateSource,
    ShortTurnEvaluationObservation, ShortTurnRefiner, SpeakerAcceptancePolicy,
    SpeakerAcceptanceTurn, TranscriptCandidateInput, VadEventCandidateInput,
};
use app_lib::diarization::short_turn_event::ShortTurnMaterializationPolicy;
use app_lib::diarization::types::{
    AssignmentMethod, AudioSource, SegmentKind, SpeakerSegment, TranscriptTiming,
};
use clap::{Parser, ValueEnum};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum BenchmarkMode {
    /// Deterministic threshold/rule regression from manifest-provided evidence.
    Evidence,
    /// Reserved for full ASR + diarization + extraction execution.
    Pipeline,
}

#[derive(Debug, Parser)]
struct Args {
    /// Folder containing manifest.jsonl and referenced local evidence.
    #[arg(long)]
    dataset: PathBuf,
    #[arg(long, value_enum, default_value_t = BenchmarkMode::Evidence)]
    mode: BenchmarkMode,
}

#[derive(Debug, Deserialize)]
struct ManifestRow {
    #[serde(default)]
    meeting_id: String,
    #[serde(default)]
    audio_path: Option<PathBuf>,
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
    vad_events: Vec<ManifestVadEvent>,
    #[serde(default, alias = "accepted_speakers")]
    ground_truth_accepted_speakers: Vec<String>,
    #[serde(default)]
    expected_visible_speakers: Vec<String>,
    #[serde(default)]
    expected_materialized: Option<bool>,
    #[serde(default)]
    embedded: Option<bool>,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ManifestTurn {
    start_ms: i64,
    end_ms: i64,
    speaker_key: String,
    #[serde(default)]
    confidence: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct ManifestVadEvent {
    start_ms: i64,
    end_ms: i64,
    #[serde(default)]
    confidence: Option<f64>,
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
    if matches!(args.mode, BenchmarkMode::Pipeline) {
        bail!(
            "Pipeline Mode is not yet wired to the application's model lifecycle; use Evidence Mode. Refusing to label manifest evidence as end-to-end."
        );
    }
    let rows = read_manifest(&args.dataset)?;
    let extractor = ShortTurnCandidateExtractor::default();
    let refiner = ShortTurnRefiner::default();
    let acceptance = SpeakerAcceptancePolicy::default();
    let matcher = CandidateMatchConfig::default();
    let materializer = ShortTurnMaterializationPolicy::default();
    let mut observations = Vec::new();
    let mut recall = Vec::new();
    let mut transcript_false_new = Vec::new();
    let mut visible_false_new = Vec::new();
    let mut materialization_observations = Vec::new();

    for (index, row) in rows.iter().enumerate() {
        let transcript_timing = (!row.transcript_text.is_empty()).then(|| TranscriptTiming {
            id: format!("manifest-{index}"),
            start_ms: row.transcript_start_ms.unwrap_or(row.start_ms),
            end_ms: row.transcript_end_ms.unwrap_or(row.end_ms),
            audio_source: AudioSource::Imported,
        });
        let transcripts = transcript_timing
            .iter()
            .map(|timing| TranscriptCandidateInput {
                timing: timing.clone(),
                text: row.transcript_text.clone(),
                asr_confidence: row.asr_confidence,
            })
            .collect::<Vec<_>>();
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
        let acceptance_turns = row
            .diarizer_turns
            .iter()
            .map(|turn| SpeakerAcceptanceTurn {
                start_ms: turn.start_ms,
                end_ms: turn.end_ms,
                speaker_key: turn.speaker_key.clone(),
                confidence: turn.confidence,
            })
            .collect::<Vec<_>>();
        let predicted_accepted = acceptance.accepted_speaker_keys(&acceptance_turns);
        let vad = row
            .vad_events
            .iter()
            .map(|event| VadEventCandidateInput {
                start_ms: event.start_ms,
                end_ms: event.end_ms,
                confidence: event.confidence,
                audio_source: AudioSource::Imported,
            })
            .collect::<Vec<_>>();
        let candidates = extractor.extract(&transcripts, &speakers, &vad);
        let found = candidates
            .iter()
            .filter(|candidate| {
                candidate_matches_ground_truth(
                    candidate.start_ms,
                    candidate.end_ms,
                    row.start_ms,
                    row.end_ms,
                    &matcher,
                )
            })
            .max_by_key(|candidate| {
                candidate
                    .end_ms
                    .min(row.end_ms)
                    .saturating_sub(candidate.start_ms.max(row.start_ms))
            });
        recall.push(CandidateRecallObservation {
            expected_short_event: row.end_ms.saturating_sub(row.start_ms) <= 1_200,
            found_sources: found
                .map(|candidate| candidate.candidate_sources.clone())
                .unwrap_or_default(),
        });
        let prototypes = MeetingSpeakerPrototypeStore::new(predicted_accepted.iter().cloned());
        let decision = found.map(|candidate| refiner.refine(candidate, &prototypes));
        let predicted_kind = decision
            .as_ref()
            .map(|decision| decision.kind.clone())
            .unwrap_or(SegmentKind::Unknown);
        let predicted_speaker = decision
            .as_ref()
            .and_then(|decision| decision.speaker_key.clone());
        transcript_false_new.push(
            predicted_speaker
                .as_ref()
                .is_some_and(|speaker| !predicted_accepted.contains(speaker)),
        );
        let expected_visible = if row.expected_visible_speakers.is_empty() {
            &row.ground_truth_accepted_speakers
        } else {
            &row.expected_visible_speakers
        };
        visible_false_new.push(
            predicted_accepted
                .iter()
                .any(|speaker| !expected_visible.contains(speaker)),
        );

        let timings = transcript_timing.iter().cloned().collect::<Vec<_>>();
        let materialized = found
            .zip(decision.as_ref())
            .and_then(|(candidate, decision)| {
                materializer
                    .materialize("benchmark", candidate, decision, &timings)
                    .event
            });
        let expected_materialized = row.expected_materialized.unwrap_or(matches!(
            row.ground_truth_kind,
            SegmentKind::Speech | SegmentKind::Backchannel
        ));
        let embedded = row.embedded.unwrap_or_else(|| {
            transcript_timing
                .as_ref()
                .is_some_and(|timing| timing.end_ms.saturating_sub(timing.start_ms) > 1_200)
        });
        materialization_observations.push(MaterializationObservation {
            expected_visible: expected_materialized,
            predicted_visible: materialized
                .as_ref()
                .is_some_and(|event| event.user_visible),
            embedded,
            expected_speaker: row.ground_truth_speaker.clone(),
            predicted_speaker: materialized.and_then(|event| event.speaker_key),
        });

        let duration_ms = row.end_ms.saturating_sub(row.start_ms) as u64;
        let _declared_bucket = &row.duration_bucket;
        let _local_audio_reference = &row.audio_path;
        let _notes = &row.notes;
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
    let meeting_ids: BTreeSet<_> = rows
        .iter()
        .map(|row| row.meeting_id.trim())
        .filter(|meeting| !meeting.is_empty())
        .collect();
    let speaker_scenarios: BTreeSet<_> = rows
        .iter()
        .filter_map(|row| {
            row.ground_truth_speaker
                .as_deref()
                .map(|speaker| format!("{}:{speaker}", row.meeting_id))
        })
        .collect();
    let buckets: BTreeSet<_> = rows
        .iter()
        .map(|row| row.duration_bucket.as_str())
        .collect();
    let tags: BTreeSet<_> = rows
        .iter()
        .flat_map(|row| row.tags.iter().map(String::as_str))
        .collect();
    let kinds: BTreeSet<_> = rows
        .iter()
        .map(|row| row.ground_truth_kind.as_str())
        .collect();
    let floor_met = rows.len() >= 100
        && meeting_ids.len() >= 3
        && speaker_scenarios.len() >= 2
        && ["100-300ms", "300-500ms", "500-800ms", "800-1200ms"]
            .iter()
            .all(|bucket| buckets.contains(bucket))
        && ["backchannel", "speech", "noise"]
            .iter()
            .all(|kind| kinds.contains(kind))
        && ["overlap", "speaker_handoff"]
            .iter()
            .all(|tag| tags.contains(tag));
    let data_gate = if floor_met {
        "READY_FOR_ERROR_TAXONOMY_REVIEW"
    } else {
        "INSUFFICIENT_DATA"
    };
    let candidate_recall = compute_candidate_recall(&recall);
    let diagnosis = if !floor_met {
        "INSUFFICIENT_DATA"
    } else if candidate_recall.union < 0.90 {
        "IMPROVE_VAD_OR_CANDIDATE_EXTRACTION"
    } else if overall.backchannel_recall < 0.85 || overall.backchannel_precision < 0.85 {
        "CONSIDER_AED_OR_ACOUSTIC_CLASSIFIER"
    } else if overall.speaker_attribution_accuracy < 0.85 {
        "CONSIDER_MEETING_LOCAL_SPEAKER_EMBEDDING"
    } else {
        "NO_NEW_MODEL_SIGNAL_FROM_CURRENT_ERRORS"
    };
    let report = serde_json::json!({
        "mode": "evidence",
        "pipeline_mode": "not_run_not_fully_supported",
        "dataset": args.dataset,
        "sample_count": rows.len(),
        "data_gate": data_gate,
        "diagnosis": diagnosis,
        "dataset_coverage": {
            "meeting_count": meeting_ids.len(),
            "speaker_scenario_count": speaker_scenarios.len(),
            "duration_buckets": buckets,
            "kinds": kinds,
            "tags": tags,
        },
        "recommended_floor": {
            "short_events": 100,
            "meetings": 3,
            "coverage": ["100-300ms", "300-500ms", "500-800ms", "800-1200ms", "backchannel", "short_speech", "noise", "overlap", "speaker_handoff"]
        },
        "candidate_match_config": matcher,
        "overall": overall,
        "duration_buckets": compute_duration_bucket_metrics(&observations),
        "candidate_recall_by_source": candidate_recall,
        "speaker_acceptance": compute_speaker_acceptance_metrics(&transcript_false_new, &visible_false_new),
        "materialization": compute_materialization_metrics(&materialization_observations),
        "candidate_sources": [
            ShortTurnCandidateSource::Transcript,
            ShortTurnCandidateSource::DiarizerTurn,
            ShortTurnCandidateSource::VadEvent,
        ],
        "phase_2d_decision_matrix": {
            "low_candidate_recall": "improve VAD/extraction; do not add speaker embeddings",
            "good_recall_poor_kind": "consider AED/acoustic classification",
            "good_recall_kind_poor_speaker": "consider meeting-local speaker embeddings",
            "overlap_or_segmentation_errors": "consider Sortformer/MSDD-style segmentation work"
        },
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
