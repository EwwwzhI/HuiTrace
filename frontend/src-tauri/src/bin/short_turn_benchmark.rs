use std::collections::{BTreeSet, HashMap};
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
    /// Replay artifacts captured by a real application run. This still does not
    /// execute ASR/diarization models and is not end-to-end.
    ProductionArtifactReplay,
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
    id: String,
    #[serde(default)]
    record_type: String,
    #[serde(default)]
    recall_eligible: bool,
    #[serde(default)]
    evidence_origin: String,
    #[serde(default)]
    meeting_id: String,
    #[serde(default)]
    audio_path: Option<PathBuf>,
    start_ms: i64,
    end_ms: i64,
    duration_bucket: String,
    #[serde(deserialize_with = "deserialize_ground_truth_kind")]
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

fn deserialize_ground_truth_kind<'de, D>(
    deserializer: D,
) -> std::result::Result<SegmentKind, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    match value.as_str() {
        "short_speech" | "speech" | "ordinary_speech_control" => Ok(SegmentKind::Speech),
        "backchannel" => Ok(SegmentKind::Backchannel),
        "noise" => Ok(SegmentKind::Noise),
        "non_speech_vocalization" => Ok(SegmentKind::NonSpeechVocalization),
        "unknown" => Ok(SegmentKind::Unknown),
        _ => Err(serde::de::Error::custom(format!(
            "invalid ground_truth_kind {value:?}"
        ))),
    }
}

fn duration_bucket(duration_ms: i64) -> Option<&'static str> {
    match duration_ms {
        100..=299 => Some("100-300ms"),
        300..=499 => Some("300-500ms"),
        500..=799 => Some("500-800ms"),
        800..=1_200 => Some("800-1200ms"),
        _ => None,
    }
}

fn validate_manifest(rows: &[ManifestRow], production_only: bool) -> Result<()> {
    let mut ids = BTreeSet::new();
    let mut meeting_diarizer: HashMap<&str, &Vec<ManifestTurn>> = HashMap::new();
    let mut meeting_vad: HashMap<&str, &Vec<ManifestVadEvent>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        let label = if row.id.is_empty() {
            format!("line {}", index + 1)
        } else {
            row.id.clone()
        };
        if row.id.trim().is_empty() || !ids.insert(row.id.as_str()) {
            bail!("{label}: every benchmark row needs a unique non-empty id");
        }
        if row.record_type != "ground_truth_event" {
            bail!("{label}: record_type must be ground_truth_event; annotation windows and candidate proposals are not benchmark samples");
        }
        if !row.recall_eligible {
            bail!("{label}: recall_eligible must be true; this manifest cannot demonstrate false negatives otherwise");
        }
        if row.meeting_id.trim().is_empty() {
            bail!("{label}: missing meeting_id");
        }
        if row.start_ms < 0 || row.end_ms <= row.start_ms {
            bail!("{label}: invalid timing {}..{}", row.start_ms, row.end_ms);
        }
        match (row.transcript_start_ms, row.transcript_end_ms) {
            (Some(start), Some(end)) if start >= 0 && end > start => {}
            (None, None) => {}
            _ => bail!("{label}: transcript timing must be a valid start/end pair"),
        }
        if !valid_confidence(row.asr_confidence) {
            bail!("{label}: ASR confidence must be null or within 0..=1");
        }
        let derived = duration_bucket(row.end_ms - row.start_ms).unwrap_or("non_short_control");
        if row.duration_bucket != derived {
            bail!(
                "{label}: duration_bucket {:?} disagrees with derived {derived}",
                row.duration_bucket
            );
        }
        if production_only && row.evidence_origin != "production_artifact" {
            bail!(
                "{label}: production-artifact-replay requires evidence_origin=production_artifact"
            );
        }
        if !matches!(
            row.evidence_origin.as_str(),
            "production_artifact" | "annotated_evidence_replay"
        ) {
            bail!("{label}: evidence_origin must identify production_artifact or annotated_evidence_replay");
        }
        for turn in &row.diarizer_turns {
            if turn.start_ms < 0
                || turn.end_ms <= turn.start_ms
                || turn.speaker_key.is_empty()
                || !valid_confidence(turn.confidence)
            {
                bail!("{label}: invalid raw diarizer turn");
            }
        }
        for event in &row.vad_events {
            if event.start_ms < 0
                || event.end_ms <= event.start_ms
                || !valid_confidence(event.confidence)
            {
                bail!("{label}: invalid VAD event");
            }
        }
        if let Some(previous) =
            meeting_diarizer.insert(row.meeting_id.as_str(), &row.diarizer_turns)
        {
            if previous != &row.diarizer_turns {
                bail!("{label}: raw diarizer evidence is inconsistent within the meeting");
            }
        }
        if let Some(previous) = meeting_vad.insert(row.meeting_id.as_str(), &row.vad_events) {
            if previous != &row.vad_events {
                bail!("{label}: VAD evidence is inconsistent within the meeting");
            }
        }
    }
    Ok(())
}

fn valid_confidence(value: Option<f64>) -> bool {
    match value {
        Some(value) => value.is_finite() && (0.0..=1.0).contains(&value),
        None => true,
    }
}

fn failure_summary(ids: &[String]) -> serde_json::Value {
    serde_json::json!({
        "count": ids.len(),
        "example_ids": ids.iter().take(5).collect::<Vec<_>>(),
    })
}

#[derive(Debug, PartialEq, Deserialize)]
struct ManifestTurn {
    start_ms: i64,
    end_ms: i64,
    speaker_key: String,
    #[serde(default)]
    confidence: Option<f64>,
}

#[derive(Debug, PartialEq, Deserialize)]
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
    validate_manifest(
        &rows,
        matches!(args.mode, BenchmarkMode::ProductionArtifactReplay),
    )?;
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
    let mut candidate_failures = Vec::new();
    let mut kind_failures = Vec::new();
    let mut speaker_failures = Vec::new();
    let mut acceptance_failures = Vec::new();
    let mut materialization_failures = Vec::new();
    let mut overlap_failures = Vec::new();

    for (index, row) in rows.iter().enumerate() {
        let labelled_duration_ms = row.end_ms.saturating_sub(row.start_ms);
        let is_short = labelled_duration_ms <= 1_200;
        let evaluation_kind = if is_short {
            row.ground_truth_kind.clone()
        } else {
            SegmentKind::Unknown
        };
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
        let expected_event = matches!(
            row.ground_truth_kind,
            SegmentKind::Speech | SegmentKind::Backchannel
        ) && is_short;
        if (expected_event && found.is_none()) || (!expected_event && found.is_some()) {
            candidate_failures.push(row.id.clone());
        }
        recall.push(CandidateRecallObservation {
            expected_short_event: expected_event,
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
        if found.is_some() && predicted_kind != evaluation_kind {
            kind_failures.push(row.id.clone());
        }
        if expected_event
            && found.is_some()
            && row.ground_truth_speaker.is_some()
            && predicted_speaker != row.ground_truth_speaker
        {
            speaker_failures.push(row.id.clone());
        }
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
        if *transcript_false_new.last().unwrap_or(&false)
            || *visible_false_new.last().unwrap_or(&false)
        {
            acceptance_failures.push(row.id.clone());
        }

        let timings = transcript_timing.iter().cloned().collect::<Vec<_>>();
        let materialized = found
            .zip(decision.as_ref())
            .and_then(|(candidate, decision)| {
                materializer
                    .materialize("benchmark", candidate, decision, &timings)
                    .event
            });
        let expected_materialized = row.expected_materialized.unwrap_or(matches!(
            evaluation_kind,
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
            expected_speaker: expected_materialized
                .then(|| row.ground_truth_speaker.clone())
                .flatten(),
            predicted_speaker: materialized.and_then(|event| event.speaker_key),
        });
        let latest_materialization = materialization_observations.last().expect("just pushed");
        let materialization_failed = latest_materialization.expected_visible
            != latest_materialization.predicted_visible
            || (latest_materialization.expected_visible
                && latest_materialization.expected_speaker.is_some()
                && latest_materialization.expected_speaker
                    != latest_materialization.predicted_speaker);
        if materialization_failed {
            materialization_failures.push(row.id.clone());
        }
        if (row
            .tags
            .iter()
            .any(|tag| tag == "overlap" || tag == "speaker_handoff"))
            && (found.is_none()
                || predicted_kind != evaluation_kind
                || predicted_speaker != row.ground_truth_speaker
                || materialization_failed)
        {
            overlap_failures.push(row.id.clone());
        }

        let duration_ms = labelled_duration_ms as u64;
        let _declared_bucket = &row.duration_bucket;
        let _local_audio_reference = &row.audio_path;
        let _notes = &row.notes;
        observations.push((
            duration_ms,
            ShortTurnEvaluationObservation {
                expected_kind: evaluation_kind,
                expected_speaker: expected_event
                    .then(|| row.ground_truth_speaker.clone())
                    .flatten(),
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
        .map(
            |row| match (&row.ground_truth_kind, row.end_ms - row.start_ms <= 1_200) {
                (SegmentKind::Speech, true) => "short_speech",
                (SegmentKind::Speech, false) => "ordinary_speech_control",
                _ => row.ground_truth_kind.as_str(),
            },
        )
        .collect();
    let floor_met = rows.len() >= 100
        && meeting_ids.len() >= 3
        && speaker_scenarios.len() >= 2
        && ["100-300ms", "300-500ms", "500-800ms", "800-1200ms"]
            .iter()
            .all(|bucket| buckets.contains(bucket))
        && ["backchannel", "short_speech", "noise"]
            .iter()
            .all(|kind| kinds.contains(kind))
        && ["overlap", "speaker_handoff"]
            .iter()
            .all(|tag| tags.contains(tag));
    let data_gate = if floor_met {
        "READY_FOR_ERROR_TAXONOMY_REVIEW"
    } else {
        "INSUFFICIENT_DATA_FOR_PHASE_2D"
    };
    let candidate_recall = compute_candidate_recall(&recall);
    let diagnosis = if !floor_met {
        "INSUFFICIENT_DATA_FOR_PHASE_2D"
    } else if !candidate_failures.is_empty() || candidate_recall.union < 0.90 {
        "IMPROVE_VAD_OR_CANDIDATE_EXTRACTION"
    } else if !kind_failures.is_empty()
        || overall.backchannel_recall < 0.85
        || overall.backchannel_precision < 0.85
    {
        "CONSIDER_AED_OR_ACOUSTIC_CLASSIFIER"
    } else if !speaker_failures.is_empty() || overall.speaker_attribution_accuracy < 0.85 {
        "CONSIDER_MEETING_LOCAL_SPEAKER_EMBEDDING"
    } else if !acceptance_failures.is_empty() {
        "FIX_SPEAKER_ACCEPTANCE"
    } else if !materialization_failures.is_empty() {
        "FIX_EMBEDDED_MATERIALIZATION"
    } else if !overlap_failures.is_empty() {
        "IMPROVE_OVERLAP_OR_SEGMENTATION"
    } else {
        "NO_NEW_MODEL_SIGNAL_FROM_CURRENT_ERRORS"
    };
    let taxonomy = serde_json::json!({
        "candidate_extraction": failure_summary(&candidate_failures),
        "event_kind_classification": failure_summary(&kind_failures),
        "speaker_attribution": failure_summary(&speaker_failures),
        "false_new_speaker_acceptance": failure_summary(&acceptance_failures),
        "embedded_materialization": failure_summary(&materialization_failures),
        "overlap_or_segmentation": failure_summary(&overlap_failures),
    });
    let report = serde_json::json!({
        "mode": match args.mode { BenchmarkMode::Evidence => "evidence_replay", BenchmarkMode::ProductionArtifactReplay => "production_artifact_replay", BenchmarkMode::Pipeline => unreachable!() },
        "pipeline_mode": "not_run_not_fully_supported",
        "dataset": args.dataset,
        "sample_count": rows.len(),
        "data_gate": data_gate,
        "diagnosis": diagnosis,
        "error_taxonomy": taxonomy,
        "dataset_coverage": {
            "meeting_count": meeting_ids.len(),
            "speaker_scenario_count": speaker_scenarios.len(),
            "duration_buckets": buckets,
            "kinds": kinds,
            "tags": tags,
        },
        "recommended_floor": {
            "annotated_event_or_control_samples": 100,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(extra: &str) -> ManifestRow {
        let mut root = serde_json::json!({
            "id": "event-1",
            "record_type": "ground_truth_event",
            "recall_eligible": true,
            "evidence_origin": "production_artifact",
            "meeting_id": "meeting-1",
            "start_ms": 1000,
            "end_ms": 1280,
            "duration_bucket": "100-300ms",
            "ground_truth_kind": "short_speech",
            "ground_truth_speaker": "speaker_01"
        });
        if !extra.is_empty() {
            let mutation: serde_json::Value = serde_json::from_str(extra).unwrap();
            for (key, mutation_value) in mutation.as_object().unwrap() {
                root.as_object_mut()
                    .unwrap()
                    .insert(key.clone(), mutation_value.clone());
            }
        }
        serde_json::from_value(root).unwrap()
    }

    #[test]
    fn manifest_semantics_are_derived_not_trusted() {
        assert!(validate_manifest(&[row("")], true).is_ok());
        assert!(validate_manifest(&[row(r#"{"duration_bucket":"300-500ms"}"#)], true).is_err());
        assert!(
            validate_manifest(&[row(r#"{"record_type":"candidate_proposal"}"#)], false).is_err()
        );
        assert!(validate_manifest(&[row(r#"{"meeting_id":""}"#)], false).is_err());
        assert!(validate_manifest(&[row(r#"{"end_ms":1000}"#)], false).is_err());
        assert!(validate_manifest(&[row(r#"{"recall_eligible":false}"#)], false).is_err());
        assert!(validate_manifest(
            &[
                row(r#"{"vad_events":[{"start_ms":990,"end_ms":1300}]}"#),
                row(r#"{"id":"event-2","vad_events":[]}"#),
            ],
            false,
        )
        .is_err());
    }

    #[test]
    fn suppressing_a_candidate_reduces_recall_without_removing_ground_truth() {
        let present = compute_candidate_recall(&[CandidateRecallObservation {
            expected_short_event: true,
            found_sources: vec![ShortTurnCandidateSource::VadEvent],
        }]);
        let suppressed = compute_candidate_recall(&[CandidateRecallObservation {
            expected_short_event: true,
            found_sources: vec![],
        }]);
        assert_eq!(present.sample_count, suppressed.sample_count);
        assert_eq!(present.union, 1.0);
        assert_eq!(suppressed.union, 0.0);
    }
}
