use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use app_lib::diarization::short_turn::{
    candidate_matches_ground_truth, LexicalBackchannelDetector, MeetingSpeakerPrototypeStore,
    ShortTurnCandidate, ShortTurnCandidateExtractor, ShortTurnCandidateSource, ShortTurnDecision,
    ShortTurnRefiner, SpeakerAcceptanceTurn, TranscriptCandidateInput, VadEventCandidateInput,
};
use app_lib::diarization::short_turn_event::ShortTurnEvent;
use app_lib::diarization::types::{
    AssignmentMethod, AudioSource, SegmentKind, SpeakerSegment, TranscriptTiming,
};
use app_lib::evaluation::dataset::{coverage, CoveragePolicy, GateSample, GroundTruthKind};
use app_lib::evaluation::production_artifact::{
    validate_artifact, validate_production_config, ArtifactBackend, ArtifactDiarizerTurn,
    ArtifactTranscript, ArtifactVadEvent, MeetingProductionArtifact, ProductionConfigSnapshot,
    ProductionSafetyObservations, SourceAudioMetadata, ARTIFACT_SCHEMA_VERSION,
    PHASE_2C1_FROZEN_BASELINE_COMMIT,
};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum BenchmarkMode {
    Evidence,
    ProductionArtifactReplay,
    CounterfactualReplay,
    Pipeline,
}

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    dataset: PathBuf,
    #[arg(long, value_enum, default_value_t = BenchmarkMode::Evidence)]
    mode: BenchmarkMode,
    /// Complete experimental config used only by counterfactual replay.
    #[arg(long)]
    experiment_config: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
struct ManifestRow {
    #[serde(default, alias = "id")]
    ground_truth_event_id: String,
    #[serde(default)]
    record_type: String,
    #[serde(default)]
    recall_eligible: bool,
    #[serde(default)]
    evidence_origin: String,
    #[serde(default)]
    meeting_id: String,
    #[serde(default)]
    production_artifact_path: Option<PathBuf>,
    #[serde(default)]
    production_artifact_sha256: Option<String>,
    #[serde(default, rename = "audio_path")]
    _audio_path: Option<PathBuf>,
    start_ms: i64,
    end_ms: i64,
    duration_bucket: String,
    ground_truth_kind: GroundTruthKind,
    #[serde(default)]
    ground_truth_speaker: Option<String>,
    #[serde(default)]
    annotation_uncertain: bool,
    #[serde(default)]
    transcript_text: String,
    #[serde(default)]
    transcript_start_ms: Option<i64>,
    #[serde(default)]
    transcript_end_ms: Option<i64>,
    #[serde(default)]
    asr_confidence: Option<f64>,
    #[serde(default)]
    diarizer_turns: Vec<ArtifactDiarizerTurn>,
    #[serde(default)]
    vad_events: Vec<ArtifactVadEvent>,
    #[serde(default, alias = "accepted_speakers")]
    ground_truth_accepted_speakers: Vec<String>,
    #[serde(default)]
    expected_visible_speakers: Vec<String>,
    #[serde(default)]
    expected_materialized: Option<bool>,
    #[serde(default)]
    embedded: Option<bool>,
    #[serde(default, rename = "notes")]
    _notes: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ArtifactProvenance {
    artifact_id: String,
    transcription_run_id: String,
    meeting_id: String,
    path: String,
    computed_sha256: String,
    app_commit_sha: String,
    asr: ArtifactBackend,
    diarization: ArtifactBackend,
}

#[derive(Debug, Clone)]
struct LoadedMeeting {
    artifact: MeetingProductionArtifact,
    provenance: Option<ArtifactProvenance>,
}

#[derive(Debug, Clone)]
struct Prediction {
    candidate: ShortTurnCandidate,
    decision: ShortTurnDecision,
    event: Option<ShortTurnEvent>,
}

#[derive(Debug, Clone)]
struct MeetingRun {
    artifact: MeetingProductionArtifact,
    replay_config: ProductionConfigSnapshot,
    #[cfg(test)]
    prototype_speakers: BTreeSet<String>,
    candidates: Vec<ShortTurnCandidate>,
    predictions: Vec<Prediction>,
    predicted_accepted: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum RootCause {
    CandidateMiss,
    KindError,
    SpeakerAttributionError,
    SpeakerAcceptanceError,
    MaterializationError,
    SegmentationOverlapError,
    AnnotationUncertain,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
struct ClassMetrics {
    support: usize,
    predicted: usize,
    true_positive: usize,
    precision: f64,
    recall: f64,
    f1: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
struct SpeakerMetrics {
    denominator: usize,
    correct: usize,
    attribution_accuracy: f64,
    unattributed_rate: f64,
    wrong_existing_speaker_rate: f64,
}

#[derive(Debug, Clone, Serialize)]
struct TemporalMatchDiagnostics {
    matched_pairs: Vec<MatchedPairDiagnostic>,
    unmatched_ground_truth: Vec<UnmatchedGroundTruthDiagnostic>,
    unmatched_predictions: Vec<UnmatchedPredictionDiagnostic>,
    ambiguous_matches: Vec<AmbiguousMatchDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
struct MatchedPairDiagnostic {
    meeting_id: String,
    ground_truth_event_id: String,
    ground_truth_interval_ms: [i64; 2],
    prediction_interval_ms: [i64; 2],
    temporal_score: f64,
    iou: f64,
    ground_truth_coverage: f64,
    prediction_coverage: f64,
    center_distance_ms: i64,
    boundary_error_ms: i64,
    duration_difference_ms: i64,
    matching_ambiguous: bool,
}

#[derive(Debug, Clone, Serialize)]
struct UnmatchedGroundTruthDiagnostic {
    meeting_id: String,
    ground_truth_event_id: String,
    ground_truth_interval_ms: [i64; 2],
}

#[derive(Debug, Clone, Serialize)]
struct UnmatchedPredictionDiagnostic {
    meeting_id: String,
    prediction_interval_ms: [i64; 2],
}

#[derive(Debug, Clone, Serialize)]
struct AmbiguousMatchDiagnostic {
    meeting_id: String,
    interval_ms: [i64; 2],
    ground_truth_event_ids: Vec<String>,
    prediction_count: usize,
    event_count_correct: bool,
    ground_truth_speakers: Vec<String>,
    predicted_speakers: Vec<String>,
    speaker_set_correct: Option<bool>,
    ground_truth_kinds: Vec<String>,
    predicted_kinds: Vec<String>,
    kind_multiset_correct: bool,
}

struct MatchResult<'a> {
    predictions: HashMap<String, Option<&'a Prediction>>,
    ambiguous_ground_truth: HashSet<String>,
    diagnostics: TemporalMatchDiagnostics,
}

impl<'a> std::ops::Deref for MatchResult<'a> {
    type Target = HashMap<String, Option<&'a Prediction>>;

    fn deref(&self) -> &Self::Target {
        &self.predictions
    }
}

#[derive(Debug, Clone, Copy)]
struct TemporalGeometry {
    overlap_ms: i64,
    iou: f64,
    ground_truth_coverage: f64,
    prediction_coverage: f64,
    center_distance_ms: i64,
    boundary_error_ms: i64,
    duration_difference_ms: i64,
    score: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
struct ConfidenceBin {
    count: usize,
    correct: usize,
    accuracy: f64,
}

impl GateSample for ManifestRow {
    fn event_id(&self) -> &str {
        &self.ground_truth_event_id
    }

    fn meeting_id(&self) -> &str {
        &self.meeting_id
    }

    fn start_ms(&self) -> i64 {
        self.start_ms
    }

    fn end_ms(&self) -> i64 {
        self.end_ms
    }

    fn kind_label(&self) -> &'static str {
        self.ground_truth_kind
            .gate_label(self.end_ms - self.start_ms)
    }

    fn ground_truth_speaker(&self) -> Option<&str> {
        self.ground_truth_speaker.as_deref()
    }

    fn expected_visible_speakers(&self) -> &[String] {
        &self.expected_visible_speakers
    }

    fn annotation_uncertain(&self) -> bool {
        self.annotation_uncertain
    }

    fn tags(&self) -> &[String] {
        &self.tags
    }
}

fn duration_bucket(duration_ms: i64) -> &'static str {
    match duration_ms {
        100..=299 => "100-300ms",
        300..=499 => "300-500ms",
        500..=799 => "500-800ms",
        800..=1_200 => "800-1200ms",
        _ => "non_short_control",
    }
}

fn valid_confidence(value: Option<f64>) -> bool {
    value.map_or(true, |v| v.is_finite() && (0.0..=1.0).contains(&v))
}

fn is_true_short(row: &ManifestRow) -> bool {
    row.ground_truth_kind
        .is_true_short(row.end_ms - row.start_ms)
}

fn is_negative(row: &ManifestRow) -> bool {
    matches!(
        row.ground_truth_kind.segment_kind(),
        SegmentKind::Noise | SegmentKind::NonSpeechVocalization
    )
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

fn validate_manifest(rows: &[ManifestRow], production_only: bool) -> Result<()> {
    let mut ids = BTreeSet::new();
    let mut artifact_paths: HashMap<&str, &PathBuf> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        let label = if row.ground_truth_event_id.is_empty() {
            format!("line {}", index + 1)
        } else {
            row.ground_truth_event_id.clone()
        };
        if row.ground_truth_event_id.trim().is_empty()
            || !ids.insert(row.ground_truth_event_id.as_str())
        {
            bail!("{label}: every event needs a unique non-empty ground_truth_event_id");
        }
        if row.record_type != "ground_truth_event" || !row.recall_eligible {
            bail!("{label}: row must be a recall-eligible ground_truth_event");
        }
        if row.meeting_id.trim().is_empty() || row.start_ms < 0 || row.end_ms <= row.start_ms {
            bail!("{label}: invalid meeting or source-timeline timing");
        }
        if row.duration_bucket != duration_bucket(row.end_ms - row.start_ms) {
            bail!("{label}: duration_bucket disagrees with source timeline timing");
        }
        match (row.transcript_start_ms, row.transcript_end_ms) {
            (Some(start), Some(end)) if start >= 0 && end > start => {}
            (None, None) => {}
            _ => bail!("{label}: transcript timing must be a valid pair"),
        }
        if !valid_confidence(row.asr_confidence)
            || row.diarizer_turns.iter().any(|turn| {
                turn.start_ms < 0
                    || turn.end_ms <= turn.start_ms
                    || turn.speaker_key.trim().is_empty()
                    || !valid_confidence(turn.confidence)
            })
            || row.vad_events.iter().any(|event| {
                event.start_ms < 0
                    || event.end_ms <= event.start_ms
                    || !valid_confidence(event.confidence)
            })
        {
            bail!("{label}: invalid evidence timing or confidence");
        }
        if production_only {
            let path = row.production_artifact_path.as_ref().ok_or_else(|| {
                anyhow::anyhow!("{label}: production replay requires production_artifact_path")
            })?;
            if let Some(previous) = artifact_paths.insert(&row.meeting_id, path) {
                if previous != path {
                    bail!("{label}: a meeting must reference one immutable artifact");
                }
            }
        } else if !matches!(
            row.evidence_origin.as_str(),
            "annotated_evidence_replay" | "production_artifact"
        ) {
            bail!("{label}: unrecognized evidence replay origin");
        }
    }
    for (index, left) in rows.iter().enumerate() {
        for right in rows.iter().skip(index + 1) {
            if left.meeting_id == right.meeting_id
                && left.ground_truth_kind.segment_kind() == right.ground_truth_kind.segment_kind()
                && overlap_iou(left.start_ms, left.end_ms, right.start_ms, right.end_ms) >= 0.80
                && !(left.tags.iter().any(|tag| tag == "overlap")
                    && right.tags.iter().any(|tag| tag == "overlap")
                    && left.ground_truth_speaker.is_some()
                    && right.ground_truth_speaker.is_some()
                    && left.ground_truth_speaker != right.ground_truth_speaker)
            {
                bail!(
                    "{} and {} duplicate one source-timeline event",
                    left.ground_truth_event_id,
                    right.ground_truth_event_id
                );
            }
        }
    }
    Ok(())
}

fn resolved_path(dataset: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        dataset.join(path)
    }
}

fn load_production_artifacts(
    dataset: &Path,
    rows: &[ManifestRow],
) -> Result<BTreeMap<String, LoadedMeeting>> {
    let mut groups: BTreeMap<String, Vec<&ManifestRow>> = BTreeMap::new();
    for row in rows {
        groups.entry(row.meeting_id.clone()).or_default().push(row);
    }
    let mut loaded = BTreeMap::new();
    for (meeting_id, meeting_rows) in groups {
        let path = resolved_path(
            dataset,
            meeting_rows[0]
                .production_artifact_path
                .as_ref()
                .expect("validated"),
        );
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let computed_sha256 = format!("{:x}", Sha256::digest(&bytes));
        for row in &meeting_rows {
            if row
                .production_artifact_sha256
                .as_deref()
                .is_some_and(|expected| !expected.eq_ignore_ascii_case(&computed_sha256))
            {
                bail!(
                    "{}: production artifact SHA-256 mismatch",
                    row.ground_truth_event_id
                );
            }
        }
        let artifact: MeetingProductionArtifact =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        validate_artifact(&artifact, Some(&meeting_id))?;
        let provenance = ArtifactProvenance {
            artifact_id: artifact.artifact_id.clone(),
            transcription_run_id: artifact.transcription_run_id.clone(),
            meeting_id: meeting_id.clone(),
            path: path.display().to_string(),
            computed_sha256,
            app_commit_sha: artifact.app_commit_sha.clone(),
            asr: artifact.asr.clone(),
            diarization: artifact.diarization.clone(),
        };
        loaded.insert(
            meeting_id,
            LoadedMeeting {
                artifact,
                provenance: Some(provenance),
            },
        );
    }
    Ok(loaded)
}

fn build_evidence_meetings(rows: &[ManifestRow]) -> Result<BTreeMap<String, LoadedMeeting>> {
    let mut groups: BTreeMap<String, Vec<&ManifestRow>> = BTreeMap::new();
    for row in rows {
        groups.entry(row.meeting_id.clone()).or_default().push(row);
    }
    let mut loaded = BTreeMap::new();
    for (meeting_id, meeting_rows) in groups {
        let first = meeting_rows[0];
        if meeting_rows.iter().any(|row| {
            row.diarizer_turns != first.diarizer_turns || row.vad_events != first.vad_events
        }) {
            bail!("{meeting_id}: evidence replay requires consistent full-meeting diarizer/VAD evidence");
        }
        let transcripts = meeting_rows
            .iter()
            .filter(|row| !row.transcript_text.is_empty())
            .map(|row| ArtifactTranscript {
                id: format!("evidence-{}", row.ground_truth_event_id),
                start_ms: row.transcript_start_ms.unwrap_or(row.start_ms),
                end_ms: row.transcript_end_ms.unwrap_or(row.end_ms),
                text: row.transcript_text.clone(),
                asr_confidence: row.asr_confidence,
            })
            .collect();
        let duration_ms = meeting_rows
            .iter()
            .map(|row| row.end_ms)
            .chain(first.diarizer_turns.iter().map(|turn| turn.end_ms))
            .chain(first.vad_events.iter().map(|event| event.end_ms))
            .max()
            .unwrap_or(1);
        let artifact = MeetingProductionArtifact {
            schema_version: ARTIFACT_SCHEMA_VERSION,
            artifact_id: format!("annotated-evidence-{meeting_id}"),
            transcription_run_id: "not_applicable_evidence_replay".into(),
            meeting_id: meeting_id.clone(),
            source_audio: SourceAudioMetadata {
                path_hint: None,
                duration_ms,
                sha256: None,
            },
            created_at: "not_applicable_evidence_replay".into(),
            app_commit_sha: PHASE_2C1_FROZEN_BASELINE_COMMIT.into(),
            asr: ArtifactBackend {
                backend: "annotation_manifest".into(),
                model: "not_executed".into(),
                version_or_hash: None,
            },
            diarization: ArtifactBackend {
                backend: "annotation_manifest".into(),
                model: "not_executed".into(),
                version_or_hash: None,
            },
            production_config: ProductionConfigSnapshot::default(),
            transcripts,
            raw_diarizer_turns: first.diarizer_turns.clone(),
            vad_events: first.vad_events.clone(),
            accepted_speakers: first.ground_truth_accepted_speakers.clone(),
            visible_speakers: first.expected_visible_speakers.clone(),
            safety_observations: ProductionSafetyObservations::default(),
            production_metadata: serde_json::Value::Null,
        };
        loaded.insert(
            meeting_id,
            LoadedMeeting {
                artifact,
                provenance: None,
            },
        );
    }
    Ok(loaded)
}

fn run_meeting(
    loaded: LoadedMeeting,
    experiment_config: Option<&ProductionConfigSnapshot>,
) -> MeetingRun {
    let artifact = loaded.artifact;
    let config = experiment_config
        .cloned()
        .unwrap_or_else(|| artifact.production_config.clone());
    let transcripts = artifact
        .transcripts
        .iter()
        .map(|item| TranscriptCandidateInput {
            timing: TranscriptTiming {
                id: item.id.clone(),
                start_ms: item.start_ms,
                end_ms: item.end_ms,
                audio_source: AudioSource::Imported,
            },
            text: item.text.clone(),
            asr_confidence: item.asr_confidence,
        })
        .collect::<Vec<_>>();
    let speakers = artifact
        .raw_diarizer_turns
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
            overlap: turn.overlap,
        })
        .collect::<Vec<_>>();
    let vad = artifact
        .vad_events
        .iter()
        .map(|event| VadEventCandidateInput {
            start_ms: event.start_ms,
            end_ms: event.end_ms,
            confidence: event.confidence,
            audio_source: AudioSource::Imported,
        })
        .collect::<Vec<_>>();
    let acceptance_turns = artifact
        .raw_diarizer_turns
        .iter()
        .map(|turn| SpeakerAcceptanceTurn {
            start_ms: turn.start_ms,
            end_ms: turn.end_ms,
            speaker_key: turn.speaker_key.clone(),
            confidence: turn.confidence,
        })
        .collect::<Vec<_>>();
    let predicted_accepted = config
        .speaker_acceptance
        .accepted_speaker_keys(&acceptance_turns);
    let prototype_keys = if experiment_config.is_some() {
        predicted_accepted.iter().cloned().collect::<Vec<_>>()
    } else {
        artifact.accepted_speakers.clone()
    };
    #[cfg(test)]
    let prototype_speakers = prototype_keys.iter().cloned().collect();
    let prototypes = MeetingSpeakerPrototypeStore::new(prototype_keys);
    let candidates = ShortTurnCandidateExtractor {
        config: config.short_turn.clone(),
    }
    .extract(&transcripts, &speakers, &vad);
    let refiner = ShortTurnRefiner::new(config.short_turn.clone(), LexicalBackchannelDetector);
    let timings = transcripts
        .iter()
        .map(|item| item.timing.clone())
        .collect::<Vec<_>>();
    let predictions = candidates
        .iter()
        .cloned()
        .map(|candidate| {
            let decision = refiner.refine(&candidate, &prototypes);
            let event = config
                .materialization
                .materialize(&artifact.meeting_id, &candidate, &decision, &timings)
                .event;
            Prediction {
                candidate,
                decision,
                event,
            }
        })
        .collect();
    MeetingRun {
        artifact,
        replay_config: config,
        #[cfg(test)]
        prototype_speakers,
        candidates,
        predictions,
        predicted_accepted,
    }
}

fn temporal_geometry(row: &ManifestRow, prediction: &Prediction) -> TemporalGeometry {
    let prediction_duration = prediction
        .candidate
        .end_ms
        .saturating_sub(prediction.candidate.start_ms)
        .max(1);
    let ground_truth_duration = row.end_ms.saturating_sub(row.start_ms).max(1);
    let overlap = overlap_ms(
        prediction.candidate.start_ms,
        prediction.candidate.end_ms,
        row.start_ms,
        row.end_ms,
    );
    let union = prediction
        .candidate
        .end_ms
        .max(row.end_ms)
        .saturating_sub(prediction.candidate.start_ms.min(row.start_ms))
        .max(1);
    let iou = overlap as f64 / union as f64;
    let ground_truth_coverage = overlap as f64 / ground_truth_duration as f64;
    let prediction_coverage = overlap as f64 / prediction_duration as f64;
    let center_distance_ms = ((prediction.candidate.start_ms + prediction.candidate.end_ms)
        - (row.start_ms + row.end_ms))
        .abs()
        / 2;
    let boundary_error_ms = (prediction.candidate.start_ms - row.start_ms).abs()
        + (prediction.candidate.end_ms - row.end_ms).abs();
    let duration_difference_ms = (prediction_duration - ground_truth_duration).abs();
    let distance_scale = ground_truth_duration.max(prediction_duration) as f64;
    let score = 0.50 * iou + 0.30 * ground_truth_coverage + 0.20 * prediction_coverage
        - 0.05 * (center_distance_ms as f64 / distance_scale).min(1.0)
        - 0.05 * (boundary_error_ms as f64 / (2.0 * distance_scale)).min(1.0);
    TemporalGeometry {
        overlap_ms: overlap,
        iou,
        ground_truth_coverage,
        prediction_coverage,
        center_distance_ms,
        boundary_error_ms,
        duration_difference_ms,
        score,
    }
}

fn prediction_tie_key(prediction: &Prediction) -> String {
    // This key contains prediction output only. It makes exact geometry ties
    // deterministic without comparing either predicted field to a GT label.
    format!(
        "{:?}|{}|{:?}",
        prediction.decision.kind,
        prediction.decision.speaker_key.as_deref().unwrap_or(""),
        prediction.candidate.candidate_sources
    )
}

fn maximum_weight_assignment(weights: &[Vec<i64>]) -> Vec<Option<usize>> {
    fn solve(
        index: usize,
        mask: u64,
        weights: &[Vec<i64>],
        memo: &mut HashMap<(usize, u64), (i64, Vec<Option<usize>>)>,
    ) -> (i64, Vec<Option<usize>>) {
        if index == weights.len() {
            return (0, Vec::new());
        }
        if let Some(value) = memo.get(&(index, mask)) {
            return value.clone();
        }
        let (mut best_score, mut best_tail) = solve(index + 1, mask, weights, memo);
        best_tail.insert(0, None);
        for cluster in 0..weights[index].len() {
            if mask & (1 << cluster) != 0 {
                continue;
            }
            let (tail_score, mut tail) = solve(index + 1, mask | (1 << cluster), weights, memo);
            let score = weights[index][cluster] + tail_score;
            if score > best_score {
                best_score = score;
                tail.insert(0, Some(cluster));
                best_tail = tail;
            }
        }
        let result = (best_score, best_tail);
        memo.insert((index, mask), result.clone());
        result
    }
    solve(0, 0, weights, &mut HashMap::new()).1
}

#[derive(Debug, Serialize)]
struct SpeakerAlignmentCoverage {
    mapped_speakers: usize,
    total_gt_speakers: usize,
    unmapped_gt_speakers: Vec<String>,
    reference_intervals: usize,
    reference_duration_ms: i64,
    mapping: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
struct SpeakerAcceptanceMeetingStatus {
    scorable: bool,
    reason: Option<&'static str>,
    mapped_expected_speakers: usize,
    total_expected_speakers: usize,
    unmapped_expected_speakers: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SpeakerAcceptanceCoverage {
    meeting_count: usize,
    scorable_meetings: usize,
    unscorable_meetings: usize,
    false_new_speaker_rate: f64,
    missed_real_speaker_rate: f64,
    meetings: BTreeMap<String, SpeakerAcceptanceMeetingStatus>,
}

#[derive(Debug, Clone)]
struct SpeakerAcceptanceEvaluation {
    status: SpeakerAcceptanceMeetingStatus,
    failed: Option<bool>,
    false_new: bool,
    missed_real: bool,
}

fn expected_speaker_universes(rows: &[ManifestRow]) -> BTreeMap<String, BTreeSet<String>> {
    let mut expected = BTreeMap::<String, BTreeSet<String>>::new();
    for row in rows.iter().filter(|row| !row.annotation_uncertain) {
        expected.entry(row.meeting_id.clone()).or_default().extend(
            if row.expected_visible_speakers.is_empty() {
                row.ground_truth_accepted_speakers.iter().cloned()
            } else {
                row.expected_visible_speakers.iter().cloned()
            },
        );
    }
    expected
}

fn map_expected_speakers(
    original: &BTreeSet<String>,
    mapping: &BTreeMap<String, String>,
) -> Result<BTreeSet<String>, Vec<String>> {
    let unmapped = original
        .iter()
        .filter(|key| !mapping.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    if !unmapped.is_empty() {
        return Err(unmapped);
    }
    Ok(original.iter().map(|key| mapping[key].clone()).collect())
}

fn evaluate_speaker_acceptance(
    original: &BTreeSet<String>,
    mapping: &BTreeMap<String, String>,
    predicted: &BTreeSet<String>,
) -> SpeakerAcceptanceEvaluation {
    if original.is_empty() {
        return SpeakerAcceptanceEvaluation {
            status: SpeakerAcceptanceMeetingStatus {
                scorable: false,
                reason: Some("no_expected_speakers"),
                mapped_expected_speakers: 0,
                total_expected_speakers: 0,
                unmapped_expected_speakers: Vec::new(),
            },
            failed: None,
            false_new: false,
            missed_real: false,
        };
    }
    match map_expected_speakers(original, mapping) {
        Ok(expected) => {
            let false_new = predicted.iter().any(|key| !expected.contains(key));
            let missed_real = expected.iter().any(|key| !predicted.contains(key));
            SpeakerAcceptanceEvaluation {
                status: SpeakerAcceptanceMeetingStatus {
                    scorable: true,
                    reason: None,
                    mapped_expected_speakers: expected.len(),
                    total_expected_speakers: original.len(),
                    unmapped_expected_speakers: Vec::new(),
                },
                failed: Some(false_new || missed_real),
                false_new,
                missed_real,
            }
        }
        Err(unmapped) => SpeakerAcceptanceEvaluation {
            status: SpeakerAcceptanceMeetingStatus {
                scorable: false,
                reason: Some("partial_speaker_alignment"),
                mapped_expected_speakers: original.len() - unmapped.len(),
                total_expected_speakers: original.len(),
                unmapped_expected_speakers: unmapped,
            },
            failed: None,
            false_new: false,
            missed_real: false,
        },
    }
}

fn is_speaker_reference(row: &ManifestRow) -> bool {
    row.ground_truth_kind.is_speaker_reference(
        row.end_ms - row.start_ms,
        row.ground_truth_speaker.as_deref(),
        row.annotation_uncertain,
        row.tags.iter().any(|tag| tag == "overlap"),
    )
}

fn align_ground_truth_speakers(
    rows: &mut [ManifestRow],
    runs: &BTreeMap<String, MeetingRun>,
) -> BTreeMap<String, SpeakerAlignmentCoverage> {
    let mut coverage = BTreeMap::new();
    for (meeting_id, run) in runs {
        let all_gt = rows
            .iter()
            .filter(|row| &row.meeting_id == meeting_id)
            .filter_map(|row| row.ground_truth_speaker.clone())
            .collect::<BTreeSet<_>>();
        let references = rows
            .iter()
            .filter(|row| &row.meeting_id == meeting_id && is_speaker_reference(row))
            .collect::<Vec<_>>();
        let gt = references
            .iter()
            .filter_map(|row| row.ground_truth_speaker.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let production = run
            .artifact
            .raw_diarizer_turns
            .iter()
            .map(|turn| turn.speaker_key.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let weights = gt
            .iter()
            .map(|speaker| {
                production
                    .iter()
                    .map(|cluster| {
                        references
                            .iter()
                            .filter(|row| row.ground_truth_speaker.as_ref() == Some(speaker))
                            .map(|row| {
                                run.artifact
                                    .raw_diarizer_turns
                                    .iter()
                                    .filter(|turn| &turn.speaker_key == cluster)
                                    .map(|turn| {
                                        (row.end_ms.min(turn.end_ms)
                                            - row.start_ms.max(turn.start_ms))
                                        .max(0)
                                    })
                                    .sum::<i64>()
                            })
                            .sum::<i64>()
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        // Preserve the existing bounded assignment; unavailable mappings never fall back to GT events.
        let assignment = if production.len() <= 20 {
            maximum_weight_assignment(&weights)
        } else {
            vec![None; gt.len()]
        };
        let mapping = gt
            .iter()
            .enumerate()
            .filter_map(|(index, key)| {
                assignment[index]
                    .filter(|cluster| weights[index][*cluster] > 0)
                    .map(|cluster| (key.clone(), production[cluster].clone()))
            })
            .collect::<BTreeMap<_, _>>();
        coverage.insert(
            meeting_id.clone(),
            SpeakerAlignmentCoverage {
                mapped_speakers: mapping.len(),
                total_gt_speakers: all_gt.len(),
                unmapped_gt_speakers: all_gt
                    .into_iter()
                    .filter(|key| !mapping.contains_key(key))
                    .collect(),
                reference_intervals: references.len(),
                reference_duration_ms: references.iter().map(|row| row.end_ms - row.start_ms).sum(),
                mapping: mapping.clone(),
            },
        );
        // Freeze the reference-only mapping before any ShortTurn comparison.
        for row in rows.iter_mut().filter(|row| &row.meeting_id == meeting_id) {
            row.ground_truth_speaker = row
                .ground_truth_speaker
                .as_ref()
                .and_then(|key| mapping.get(key))
                .cloned();
            for key in &mut row.ground_truth_accepted_speakers {
                if let Some(mapped) = mapping.get(key) {
                    *key = mapped.clone();
                }
            }
            for key in &mut row.expected_visible_speakers {
                if let Some(mapped) = mapping.get(key) {
                    *key = mapped.clone();
                }
            }
        }
    }
    coverage
}

fn match_predictions<'a>(
    rows: &[ManifestRow],
    runs: &'a BTreeMap<String, MeetingRun>,
) -> MatchResult<'a> {
    let mut matched = rows
        .iter()
        .map(|row| (row.ground_truth_event_id.clone(), None))
        .collect::<HashMap<_, _>>();
    let mut ambiguous_ground_truth = HashSet::new();
    let mut matched_pairs = Vec::new();
    let mut unmatched_ground_truth = Vec::new();
    let mut unmatched_predictions = Vec::new();
    let mut ambiguous_matches = Vec::new();
    for (meeting_id, run) in runs {
        let mut meeting_rows = rows
            .iter()
            .filter(|row| &row.meeting_id == meeting_id)
            .collect::<Vec<_>>();
        meeting_rows
            .sort_by_key(|row| (row.start_ms, row.end_ms, row.ground_truth_event_id.as_str()));
        let mut prediction_order = (0..run.predictions.len()).collect::<Vec<_>>();
        prediction_order.sort_by_key(|index| {
            let prediction = &run.predictions[*index];
            (
                prediction.candidate.start_ms,
                prediction.candidate.end_ms,
                prediction_tie_key(prediction),
            )
        });

        let mut gt_intervals: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
        for (row_index, row) in meeting_rows.iter().enumerate() {
            gt_intervals
                .entry((row.start_ms, row.end_ms))
                .or_default()
                .push(row_index);
        }
        let mut prediction_intervals: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
        for prediction_index in &prediction_order {
            let prediction = &run.predictions[*prediction_index];
            prediction_intervals
                .entry((prediction.candidate.start_ms, prediction.candidate.end_ms))
                .or_default()
                .push(*prediction_index);
        }
        for (interval, gt_indices) in gt_intervals.iter().filter(|(_, values)| values.len() > 1) {
            let prediction_indices = prediction_intervals
                .get(interval)
                .cloned()
                .unwrap_or_default();
            if prediction_indices.len() > 1 {
                let gt_ids = gt_indices
                    .iter()
                    .map(|index| meeting_rows[*index].ground_truth_event_id.clone())
                    .collect::<Vec<_>>();
                ambiguous_ground_truth.extend(gt_ids.iter().cloned());
                let ground_truth_speakers = gt_indices
                    .iter()
                    .filter_map(|index| meeting_rows[*index].ground_truth_speaker.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let predicted_speakers = prediction_indices
                    .iter()
                    .filter_map(|index| run.predictions[*index].decision.speaker_key.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let speaker_set_correct = (!ground_truth_speakers.is_empty()
                    && gt_indices
                        .iter()
                        .all(|index| meeting_rows[*index].ground_truth_speaker.is_some()))
                .then_some(ground_truth_speakers == predicted_speakers);
                let mut ground_truth_kinds = gt_indices
                    .iter()
                    .map(|index| {
                        kind_label(&meeting_rows[*index].ground_truth_kind.segment_kind())
                            .to_string()
                    })
                    .collect::<Vec<_>>();
                ground_truth_kinds.sort();
                let mut predicted_kinds = prediction_indices
                    .iter()
                    .map(|index| kind_label(&run.predictions[*index].decision.kind).to_string())
                    .collect::<Vec<_>>();
                predicted_kinds.sort();
                let kind_multiset_correct = ground_truth_kinds == predicted_kinds;
                ambiguous_matches.push(AmbiguousMatchDiagnostic {
                    meeting_id: meeting_id.clone(),
                    interval_ms: [interval.0, interval.1],
                    ground_truth_event_ids: gt_ids,
                    prediction_count: prediction_indices.len(),
                    event_count_correct: gt_indices.len() == prediction_indices.len(),
                    ground_truth_speakers,
                    predicted_speakers,
                    speaker_set_correct,
                    ground_truth_kinds,
                    predicted_kinds,
                    kind_multiset_correct,
                });
            }
        }

        let mut edges = Vec::new();
        for (row_index, row) in meeting_rows.iter().enumerate() {
            for prediction_index in &prediction_order {
                let prediction = &run.predictions[*prediction_index];
                if candidate_matches_ground_truth(
                    prediction.candidate.start_ms,
                    prediction.candidate.end_ms,
                    row.start_ms,
                    row.end_ms,
                    &run.replay_config.candidate_match,
                ) {
                    let geometry = temporal_geometry(row, prediction);
                    edges.push((row_index, *prediction_index, geometry));
                }
            }
        }
        edges.sort_by(|left, right| {
            let left_score = (left.2.score * 1_000_000.0).round() as i64;
            let right_score = (right.2.score * 1_000_000.0).round() as i64;
            right_score
                .cmp(&left_score)
                .then_with(|| right.2.overlap_ms.cmp(&left.2.overlap_ms))
                .then_with(|| left.2.center_distance_ms.cmp(&right.2.center_distance_ms))
                .then_with(|| left.2.boundary_error_ms.cmp(&right.2.boundary_error_ms))
                .then_with(|| {
                    left.2
                        .duration_difference_ms
                        .cmp(&right.2.duration_difference_ms)
                })
                .then_with(|| {
                    meeting_rows[left.0]
                        .ground_truth_event_id
                        .cmp(&meeting_rows[right.0].ground_truth_event_id)
                })
                .then_with(|| {
                    prediction_tie_key(&run.predictions[left.1])
                        .cmp(&prediction_tie_key(&run.predictions[right.1]))
                })
        });
        let mut used_rows = HashSet::new();
        let mut used_predictions = HashSet::new();
        for (row_index, prediction_index, geometry) in edges {
            if used_rows.insert(row_index) && used_predictions.insert(prediction_index) {
                let row = meeting_rows[row_index];
                let prediction = &run.predictions[prediction_index];
                matched.insert(row.ground_truth_event_id.clone(), Some(prediction));
                matched_pairs.push(MatchedPairDiagnostic {
                    meeting_id: meeting_id.clone(),
                    ground_truth_event_id: row.ground_truth_event_id.clone(),
                    ground_truth_interval_ms: [row.start_ms, row.end_ms],
                    prediction_interval_ms: [
                        prediction.candidate.start_ms,
                        prediction.candidate.end_ms,
                    ],
                    temporal_score: geometry.score,
                    iou: geometry.iou,
                    ground_truth_coverage: geometry.ground_truth_coverage,
                    prediction_coverage: geometry.prediction_coverage,
                    center_distance_ms: geometry.center_distance_ms,
                    boundary_error_ms: geometry.boundary_error_ms,
                    duration_difference_ms: geometry.duration_difference_ms,
                    matching_ambiguous: ambiguous_ground_truth.contains(&row.ground_truth_event_id),
                });
            }
        }
        unmatched_ground_truth.extend(
            meeting_rows
                .iter()
                .enumerate()
                .filter(|(index, _)| !used_rows.contains(index))
                .map(|(_, row)| UnmatchedGroundTruthDiagnostic {
                    meeting_id: meeting_id.clone(),
                    ground_truth_event_id: row.ground_truth_event_id.clone(),
                    ground_truth_interval_ms: [row.start_ms, row.end_ms],
                }),
        );
        unmatched_predictions.extend(
            prediction_order
                .iter()
                .filter(|index| !used_predictions.contains(index))
                .map(|index| {
                    let prediction = &run.predictions[*index];
                    UnmatchedPredictionDiagnostic {
                        meeting_id: meeting_id.clone(),
                        prediction_interval_ms: [
                            prediction.candidate.start_ms,
                            prediction.candidate.end_ms,
                        ],
                    }
                }),
        );
    }
    matched_pairs.sort_by(|left, right| {
        (&left.meeting_id, &left.ground_truth_event_id)
            .cmp(&(&right.meeting_id, &right.ground_truth_event_id))
    });
    unmatched_ground_truth.sort_by(|left, right| {
        (&left.meeting_id, &left.ground_truth_event_id)
            .cmp(&(&right.meeting_id, &right.ground_truth_event_id))
    });
    unmatched_predictions.sort_by_key(|item| {
        (
            item.meeting_id.clone(),
            item.prediction_interval_ms[0],
            item.prediction_interval_ms[1],
        )
    });
    MatchResult {
        predictions: matched,
        ambiguous_ground_truth,
        diagnostics: TemporalMatchDiagnostics {
            matched_pairs,
            unmatched_ground_truth,
            unmatched_predictions,
            ambiguous_matches,
        },
    }
}

fn root_cause_label(cause: RootCause) -> &'static str {
    match cause {
        RootCause::CandidateMiss => "candidate_miss",
        RootCause::KindError => "kind_error",
        RootCause::SpeakerAttributionError => "speaker_attribution_error",
        RootCause::SpeakerAcceptanceError => "speaker_acceptance_error",
        RootCause::MaterializationError => "materialization_error",
        RootCause::SegmentationOverlapError => "segmentation_overlap_error",
        RootCause::AnnotationUncertain => "annotation_uncertain",
    }
}

fn root_cause(
    row: &ManifestRow,
    prediction: Option<&Prediction>,
    acceptance_failed: Option<bool>,
    matching_ambiguous: bool,
) -> Option<RootCause> {
    if row.annotation_uncertain {
        return Some(RootCause::AnnotationUncertain);
    }
    if is_true_short(row) && prediction.is_none() {
        return Some(RootCause::CandidateMiss);
    }
    if matching_ambiguous {
        return None;
    }
    let predicted_kind = prediction
        .map(|item| item.decision.kind.clone())
        .unwrap_or(SegmentKind::Unknown);
    if row
        .tags
        .iter()
        .any(|tag| tag == "overlap" || tag == "speaker_handoff")
        && prediction.is_some_and(|item| item.candidate.true_speaker_overlap)
        && predicted_kind != row.ground_truth_kind.segment_kind()
    {
        return Some(RootCause::SegmentationOverlapError);
    }
    if prediction.is_some() && predicted_kind != row.ground_truth_kind.segment_kind() {
        return Some(RootCause::KindError);
    }
    if !matching_ambiguous
        && is_true_short(row)
        && row.ground_truth_speaker.is_some()
        && prediction.and_then(|item| item.decision.speaker_key.as_ref())
            != row.ground_truth_speaker.as_ref()
    {
        return Some(RootCause::SpeakerAttributionError);
    }
    if acceptance_failed == Some(true) {
        return Some(RootCause::SpeakerAcceptanceError);
    }
    let expected_visible = row
        .expected_materialized
        .unwrap_or_else(|| is_true_short(row));
    let predicted_visible = prediction
        .and_then(|item| item.event.as_ref())
        .is_some_and(|event| event.user_visible);
    if expected_visible != predicted_visible {
        return Some(RootCause::MaterializationError);
    }
    None
}

fn ratio(n: usize, d: usize) -> f64 {
    if d == 0 {
        0.0
    } else {
        n as f64 / d as f64
    }
}

fn kind_label(kind: &SegmentKind) -> &'static str {
    match kind {
        SegmentKind::Speech => "short_speech",
        SegmentKind::Backchannel => "backchannel",
        SegmentKind::Noise => "noise",
        SegmentKind::NonSpeechVocalization => "non_speech_vocalization",
        SegmentKind::Unknown => "unknown",
    }
}

fn class_metrics<'a>(
    rows: &[&ManifestRow],
    predictions: &HashMap<String, Option<&'a Prediction>>,
) -> (
    BTreeMap<String, ClassMetrics>,
    BTreeMap<String, BTreeMap<String, usize>>,
) {
    let classes = [
        SegmentKind::Speech,
        SegmentKind::Backchannel,
        SegmentKind::Noise,
        SegmentKind::NonSpeechVocalization,
    ];
    let predicted_kind = |row: &ManifestRow| {
        predictions
            .get(&row.ground_truth_event_id)
            .and_then(|v| *v)
            .map(|p| p.decision.kind.clone())
            .unwrap_or(SegmentKind::Unknown)
    };
    let mut confusion: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for row in rows {
        *confusion
            .entry(kind_label(&row.ground_truth_kind.segment_kind()).into())
            .or_default()
            .entry(kind_label(&predicted_kind(row)).into())
            .or_default() += 1;
    }
    let mut metrics = BTreeMap::new();
    for class in classes {
        let support = rows
            .iter()
            .filter(|row| row.ground_truth_kind.segment_kind() == class)
            .count();
        let predicted = rows
            .iter()
            .filter(|row| predicted_kind(row) == class)
            .count();
        let tp = rows
            .iter()
            .filter(|row| {
                row.ground_truth_kind.segment_kind() == class && predicted_kind(row) == class
            })
            .count();
        let precision = ratio(tp, predicted);
        let recall = ratio(tp, support);
        metrics.insert(
            kind_label(&class).into(),
            ClassMetrics {
                support,
                predicted,
                true_positive: tp,
                precision,
                recall,
                f1: if precision + recall == 0.0 {
                    0.0
                } else {
                    2.0 * precision * recall / (precision + recall)
                },
            },
        );
    }
    (metrics, confusion)
}

fn speaker_metrics<'a>(
    rows: &[&ManifestRow],
    predictions: &HashMap<String, Option<&'a Prediction>>,
    ambiguous_ground_truth: &HashSet<String>,
) -> SpeakerMetrics {
    let scored = rows
        .iter()
        .filter(|row| {
            is_true_short(row)
                && row.ground_truth_speaker.is_some()
                && !ambiguous_ground_truth.contains(&row.ground_truth_event_id)
        })
        .collect::<Vec<_>>();
    let predicted_speaker = |row: &ManifestRow| {
        predictions
            .get(&row.ground_truth_event_id)
            .and_then(|v| *v)
            .and_then(|p| p.decision.speaker_key.as_ref())
    };
    let correct = scored
        .iter()
        .filter(|row| predicted_speaker(row) == row.ground_truth_speaker.as_ref())
        .count();
    let unattributed = scored
        .iter()
        .filter(|row| predicted_speaker(row).is_none())
        .count();
    let wrong = scored.len().saturating_sub(correct + unattributed);
    SpeakerMetrics {
        denominator: scored.len(),
        correct,
        attribution_accuracy: ratio(correct, scored.len()),
        unattributed_rate: ratio(unattributed, scored.len()),
        wrong_existing_speaker_rate: ratio(wrong, scored.len()),
    }
}

fn speaker_error_breakdown<'a>(
    rows: &[&ManifestRow],
    predictions: &HashMap<String, Option<&'a Prediction>>,
    ambiguous_ground_truth: &HashSet<String>,
) -> BTreeMap<&'static str, usize> {
    let mut result = BTreeMap::from([
        ("wrong_existing_speaker", 0),
        ("unattributed", 0),
        ("continuity_error", 0),
        ("direct_diarizer_error", 0),
        ("overlap_ambiguity", 0),
    ]);
    for row in rows.iter().filter(|row| {
        is_true_short(row)
            && row.ground_truth_speaker.is_some()
            && !ambiguous_ground_truth.contains(&row.ground_truth_event_id)
    }) {
        let prediction = predictions
            .get(&row.ground_truth_event_id)
            .and_then(|value| *value);
        if prediction.and_then(|item| item.decision.speaker_key.as_ref())
            == row.ground_truth_speaker.as_ref()
        {
            continue;
        }
        let subtype = match prediction {
            None => "unattributed",
            Some(item) if item.decision.speaker_key.is_none() => "unattributed",
            Some(item)
                if item.candidate.true_speaker_overlap
                    || row.tags.iter().any(|tag| tag == "overlap") =>
            {
                "overlap_ambiguity"
            }
            Some(item) if item.decision.evidence.reason.contains("continuity") => {
                "continuity_error"
            }
            Some(item) if item.candidate.diarization_speaker.is_some() => "direct_diarizer_error",
            Some(_) => "wrong_existing_speaker",
        };
        *result.get_mut(subtype).expect("known speaker subtype") += 1;
    }
    result
}

fn confidence_report(samples: impl IntoIterator<Item = (Option<f64>, bool)>) -> serde_json::Value {
    let mut bins = vec![ConfidenceBin::default(); 5];
    for (confidence, correct) in samples {
        if let Some(value) = confidence {
            let index = ((value.clamp(0.0, 0.999_999) * 5.0) as usize).min(4);
            bins[index].count += 1;
            bins[index].correct += usize::from(correct);
        }
    }
    for bin in &mut bins {
        bin.accuracy = ratio(bin.correct, bin.count);
    }
    let nonempty = bins.iter().filter(|bin| bin.count > 0).collect::<Vec<_>>();
    let monotonic = nonempty
        .windows(2)
        .all(|pair| pair[0].accuracy <= pair[1].accuracy + f64::EPSILON);
    serde_json::json!({
        "bins": {"0.0-0.2": bins[0], "0.2-0.4": bins[1], "0.4-0.6": bins[2], "0.6-0.8": bins[3], "0.8-1.0": bins[4]},
        "status": if nonempty.len() < 2 { "INSUFFICIENT_CONFIDENCE_DATA" } else if monotonic { "BASIC_MONOTONICITY_OBSERVED_NOT_CALIBRATED" } else { "CONFIDENCE_NOT_RELIABLE" }
    })
}

fn failure_summary(ids: &[String]) -> serde_json::Value {
    serde_json::json!({"count": ids.len(), "example_ids": ids.iter().take(5).collect::<Vec<_>>()})
}

fn main() -> Result<()> {
    let args = Args::parse();
    if matches!(args.mode, BenchmarkMode::Pipeline) {
        bail!("Pipeline Mode remains unsupported until the desktop application service layer can be reused directly");
    }
    let mut rows = read_manifest(&args.dataset)?;
    let production_mode = matches!(
        args.mode,
        BenchmarkMode::ProductionArtifactReplay | BenchmarkMode::CounterfactualReplay
    );
    let experiment_config = match (&args.mode, &args.experiment_config) {
        (BenchmarkMode::CounterfactualReplay, Some(path)) => Some(
            serde_json::from_slice::<ProductionConfigSnapshot>(
                &fs::read(path)
                    .with_context(|| format!("read counterfactual config {}", path.display()))?,
            )
            .context("parse complete counterfactual production config")?,
        ),
        (BenchmarkMode::CounterfactualReplay, None) => {
            bail!("counterfactual replay requires --experiment-config")
        }
        (_, Some(_)) => bail!("--experiment-config is only valid with counterfactual replay"),
        _ => None,
    };
    if let Some(config) = experiment_config.as_ref() {
        validate_production_config(config).context("validate complete counterfactual config")?;
    }
    validate_manifest(&rows, production_mode)?;
    let loaded = if production_mode {
        load_production_artifacts(&args.dataset, &rows)?
    } else {
        build_evidence_meetings(&rows)?
    };
    let provenance = loaded
        .values()
        .filter_map(|item| item.provenance.clone())
        .collect::<Vec<_>>();
    let runs = loaded
        .into_iter()
        .map(|(id, meeting)| (id, run_meeting(meeting, experiment_config.as_ref())))
        .collect::<BTreeMap<_, _>>();
    let policy = CoveragePolicy::default();
    let coverage = coverage(&rows, &policy);
    // Preserve the acceptance GT namespace before alignment mutates speaker identities.
    let original_expected_speakers = expected_speaker_universes(&rows);
    let speaker_alignment = align_ground_truth_speakers(&mut rows, &runs);
    let acceptance_evaluations = runs
        .iter()
        .map(|(meeting_id, run)| {
            let original = original_expected_speakers
                .get(meeting_id)
                .cloned()
                .unwrap_or_default();
            let mapping = &speaker_alignment[meeting_id].mapping;
            (
                meeting_id.clone(),
                evaluate_speaker_acceptance(&original, mapping, &run.predicted_accepted),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let matching = match_predictions(&rows, &runs);
    let matched = &matching.predictions;
    let scorable = rows
        .iter()
        .filter(|row| !row.annotation_uncertain)
        .collect::<Vec<_>>();
    let true_short = scorable
        .iter()
        .copied()
        .filter(|row| is_true_short(row))
        .collect::<Vec<_>>();
    let source_recall = |source: Option<ShortTurnCandidateSource>, subset: &[&ManifestRow]| {
        ratio(
            subset
                .iter()
                .filter(|row| {
                    matched
                        .get(&row.ground_truth_event_id)
                        .and_then(|v| *v)
                        .is_some_and(|p| {
                            source.map_or(true, |s| p.candidate.candidate_sources.contains(&s))
                        })
                })
                .count(),
            subset.len(),
        )
    };
    let recall_by_bucket = ["100-300ms", "300-500ms", "500-800ms", "800-1200ms"].into_iter().map(|bucket| {
        let subset = true_short.iter().copied().filter(|row| row.duration_bucket == bucket).collect::<Vec<_>>();
        (bucket, serde_json::json!({"sample_count": subset.len(), "union": source_recall(None, &subset)}))
    }).collect::<BTreeMap<_, _>>();
    let candidate_recall = serde_json::json!({
        "sample_count": true_short.len(), "transcript": source_recall(Some(ShortTurnCandidateSource::Transcript), &true_short),
        "diarizer_turn": source_recall(Some(ShortTurnCandidateSource::DiarizerTurn), &true_short),
        "vad": source_recall(Some(ShortTurnCandidateSource::VadEvent), &true_short), "union": source_recall(None, &true_short),
        "duration_buckets": recall_by_bucket
    });
    let negatives = scorable
        .iter()
        .copied()
        .filter(|row| is_negative(row))
        .collect::<Vec<_>>();
    let negative_with_candidate = negatives
        .iter()
        .filter(|row| matched[&row.ground_truth_event_id].is_some())
        .count();
    let downstream_rejected = negatives
        .iter()
        .filter(|row| {
            matched[&row.ground_truth_event_id].is_some_and(|p| {
                matches!(
                    p.decision.kind,
                    SegmentKind::Noise | SegmentKind::NonSpeechVocalization | SegmentKind::Unknown
                ) && !p.event.as_ref().is_some_and(|event| event.user_visible)
            })
        })
        .count();
    let final_false_accepts = negatives
        .iter()
        .filter(|row| {
            matched[&row.ground_truth_event_id]
                .and_then(|p| p.event.as_ref())
                .is_some_and(|event| event.user_visible)
        })
        .count();
    let embedded_false_accepts = negatives
        .iter()
        .filter(|row| {
            row.embedded.unwrap_or(false)
                && matched[&row.ground_truth_event_id]
                    .and_then(|p| p.event.as_ref())
                    .is_some_and(|event| event.user_visible)
        })
        .count();
    let total_candidates: usize = runs.values().map(|run| run.candidates.len()).sum();
    let total_minutes: f64 = runs
        .values()
        .map(|run| run.artifact.source_audio.duration_ms.max(0) as f64 / 60_000.0)
        .sum();
    let kind_rows = scorable
        .iter()
        .copied()
        .filter(|row| {
            row.end_ms - row.start_ms <= 1_200
                && !matching
                    .ambiguous_ground_truth
                    .contains(&row.ground_truth_event_id)
        })
        .collect::<Vec<_>>();
    let (kind_metrics, confusion) = class_metrics(&kind_rows, &matched);
    let speaker_overall = speaker_metrics(&scorable, matched, &matching.ambiguous_ground_truth);
    let speaker_buckets = ["100-300ms", "300-500ms", "500-800ms", "800-1200ms"]
        .into_iter()
        .map(|bucket| {
            let subset = scorable
                .iter()
                .copied()
                .filter(|row| row.duration_bucket == bucket)
                .collect::<Vec<_>>();
            (
                bucket,
                speaker_metrics(&subset, matched, &matching.ambiguous_ground_truth),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let speaker_contexts = ["overlap", "non_overlap", "speaker_handoff", "embedded"]
        .into_iter()
        .map(|tag| {
            let subset = scorable
                .iter()
                .copied()
                .filter(|row| {
                    if tag == "embedded" {
                        row.embedded.unwrap_or(false)
                    } else if tag == "non_overlap" {
                        !row.tags.iter().any(|value| value == "overlap")
                    } else {
                        row.tags.iter().any(|v| v == tag)
                    }
                })
                .collect::<Vec<_>>();
            (
                tag,
                speaker_metrics(&subset, matched, &matching.ambiguous_ground_truth),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let speaker_error_subtypes =
        speaker_error_breakdown(&scorable, matched, &matching.ambiguous_ground_truth);
    let ambiguous_speaker_sets = matching
        .diagnostics
        .ambiguous_matches
        .iter()
        .filter(|group| group.speaker_set_correct.is_some())
        .count();
    let correct_ambiguous_speaker_sets = matching
        .diagnostics
        .ambiguous_matches
        .iter()
        .filter(|group| group.speaker_set_correct == Some(true))
        .count();
    let ambiguous_kind_sets = matching.diagnostics.ambiguous_matches.len();
    let correct_ambiguous_kind_sets = matching
        .diagnostics
        .ambiguous_matches
        .iter()
        .filter(|group| group.kind_multiset_correct)
        .count();
    let scorable_acceptance_meetings = acceptance_evaluations
        .values()
        .filter(|evaluation| evaluation.status.scorable)
        .count();
    let false_new_meetings = acceptance_evaluations
        .values()
        .filter(|evaluation| evaluation.false_new)
        .count();
    let missed_real_meetings = acceptance_evaluations
        .values()
        .filter(|evaluation| evaluation.missed_real)
        .count();
    let expected_visible = scorable
        .iter()
        .filter(|row| {
            row.expected_materialized
                .unwrap_or_else(|| is_true_short(row))
        })
        .count();
    let predicted_visible = scorable
        .iter()
        .filter(|row| {
            matched[&row.ground_truth_event_id]
                .and_then(|p| p.event.as_ref())
                .is_some_and(|event| event.user_visible)
        })
        .count();
    let visible_tp = scorable
        .iter()
        .filter(|row| {
            row.expected_materialized
                .unwrap_or_else(|| is_true_short(row))
                && matched[&row.ground_truth_event_id]
                    .and_then(|p| p.event.as_ref())
                    .is_some_and(|event| event.user_visible)
        })
        .count();
    let embedded_scored = scorable
        .iter()
        .filter(|row| {
            row.embedded.unwrap_or(false)
                && row.ground_truth_speaker.is_some()
                && row
                    .expected_materialized
                    .unwrap_or_else(|| is_true_short(row))
        })
        .count();
    let embedded_correct = scorable
        .iter()
        .filter(|row| {
            row.embedded.unwrap_or(false)
                && row.ground_truth_speaker.is_some()
                && matched[&row.ground_truth_event_id]
                    .and_then(|p| p.event.as_ref())
                    .and_then(|event| event.speaker_key.as_ref())
                    == row.ground_truth_speaker.as_ref()
        })
        .count();
    let event_ids = runs
        .values()
        .flat_map(|run| {
            run.predictions
                .iter()
                .filter_map(|p| p.event.as_ref().map(|e| e.id.as_str()))
        })
        .collect::<Vec<_>>();
    let unique_events = event_ids.iter().copied().collect::<HashSet<_>>().len();
    let long_corruption = runs
        .values()
        .map(|run| {
            run.artifact
                .safety_observations
                .long_transcript_speaker_corruption_count
        })
        .collect::<Option<Vec<_>>>()
        .map(|values| values.into_iter().sum::<usize>());
    let manual_violations = runs
        .values()
        .map(|run| {
            run.artifact
                .safety_observations
                .manual_override_violation_count
        })
        .collect::<Option<Vec<_>>>()
        .map(|values| values.into_iter().sum::<usize>());
    let mut taxonomy: BTreeMap<RootCause, Vec<String>> = BTreeMap::new();
    let mut overgeneration = Vec::new();
    for row in &rows {
        let acceptance_failed = acceptance_evaluations[&row.meeting_id].failed;
        let prediction = matched[&row.ground_truth_event_id];
        if is_negative(row) && prediction.is_some() {
            overgeneration.push(row.ground_truth_event_id.clone());
        }
        if let Some(cause) = root_cause(
            row,
            prediction,
            acceptance_failed,
            matching
                .ambiguous_ground_truth
                .contains(&row.ground_truth_event_id),
        ) {
            taxonomy
                .entry(cause)
                .or_default()
                .push(row.ground_truth_event_id.clone());
        }
    }
    let taxonomy_json: BTreeMap<String, serde_json::Value> = taxonomy
        .iter()
        .map(|(cause, ids)| (root_cause_label(*cause).to_string(), failure_summary(ids)))
        .chain(std::iter::once((
            "candidate_overgeneration_secondary".into(),
            failure_summary(&overgeneration),
        )))
        .collect::<BTreeMap<_, _>>();
    let count = |cause| taxonomy.get(&cause).map_or(0, Vec::len);
    let union_recall = source_recall(None, &true_short);
    let decision = if !coverage.missing_requirements.is_empty() {
        "INSUFFICIENT_REPRESENTATIVE_DATA"
    } else if count(RootCause::CandidateMiss) > 0 && union_recall < 0.90 {
        "IMPROVE_CANDIDATE_EXTRACTION"
    } else if count(RootCause::KindError) > count(RootCause::CandidateMiss)
        && count(RootCause::KindError) >= count(RootCause::SpeakerAttributionError)
    {
        "ADD_ACOUSTIC_EVENT_CLASSIFIER"
    } else if count(RootCause::SegmentationOverlapError) > 0
        && count(RootCause::SegmentationOverlapError) >= count(RootCause::SpeakerAttributionError)
    {
        "IMPROVE_SEGMENTATION_OVERLAP"
    } else if count(RootCause::SpeakerAttributionError) > 0 {
        "ADD_MEETING_LOCAL_SPEAKER_MATCHING"
    } else if count(RootCause::SpeakerAcceptanceError) > 0 {
        "FIX_SPEAKER_ACCEPTANCE"
    } else if count(RootCause::MaterializationError) > 0 {
        "FIX_MATERIALIZATION"
    } else {
        "KEEP_MODEL_FREE"
    };
    let confidence = serde_json::json!({
        "diarization": confidence_report(scorable.iter().filter(|row| !matching.ambiguous_ground_truth.contains(&row.ground_truth_event_id)).map(|row| { let p = matched[&row.ground_truth_event_id]; (p.and_then(|v| v.candidate.diarization_confidence), p.is_some_and(|v| v.decision.kind == row.ground_truth_kind.segment_kind())) })),
        "effective_speaker": confidence_report(scorable.iter().filter(|row| row.ground_truth_speaker.is_some() && !matching.ambiguous_ground_truth.contains(&row.ground_truth_event_id)).map(|row| { let p = matched[&row.ground_truth_event_id]; (p.map(|v| v.decision.evidence.effective_speaker_confidence), p.and_then(|v| v.decision.speaker_key.as_ref()) == row.ground_truth_speaker.as_ref()) })),
        "kind": confidence_report(scorable.iter().filter(|row| !matching.ambiguous_ground_truth.contains(&row.ground_truth_event_id)).map(|row| { let p = matched[&row.ground_truth_event_id]; (p.map(|v| v.decision.kind_confidence), p.is_some_and(|v| v.decision.kind == row.ground_truth_kind.segment_kind())) })),
        "claim": "reliability bins only; not calibrated"
    });
    let speaker_acceptance = SpeakerAcceptanceCoverage {
        meeting_count: runs.len(),
        scorable_meetings: scorable_acceptance_meetings,
        unscorable_meetings: runs.len() - scorable_acceptance_meetings,
        false_new_speaker_rate: ratio(false_new_meetings, scorable_acceptance_meetings),
        missed_real_speaker_rate: ratio(missed_real_meetings, scorable_acceptance_meetings),
        meetings: acceptance_evaluations
            .iter()
            .map(|(meeting_id, evaluation)| (meeting_id.clone(), evaluation.status.clone()))
            .collect(),
    };
    let report = serde_json::json!({
        "mode": match args.mode { BenchmarkMode::Evidence => "annotated_evidence_replay", BenchmarkMode::ProductionArtifactReplay => "frozen_production_replay", BenchmarkMode::CounterfactualReplay => "counterfactual_replay", BenchmarkMode::Pipeline => unreachable!() },
        "pipeline_mode": "unsupported_not_run",
        "phase_2c1_frozen_baseline": {"name": "PHASE_2C1_FROZEN_BASELINE", "commit_sha": PHASE_2C1_FROZEN_BASELINE_COMMIT, "production_config": ProductionConfigSnapshot::default()},
        "artifact_schema_version": ARTIFACT_SCHEMA_VERSION, "production_artifacts": provenance,
        "meeting_replay_count": runs.len(), "sample_count": rows.len(), "coverage_policy": policy, "dataset_coverage": coverage,
        "data_gate": if decision == "INSUFFICIENT_REPRESENTATIVE_DATA" { "INSUFFICIENT_REPRESENTATIVE_DATA" } else { "REPRESENTATIVE_DATA_READY" },
        "architecture_decision": decision,
        "candidate_detection": {"recall_by_source": candidate_recall, "overgeneration": {
            "candidate_proposals": total_candidates, "candidate_proposals_per_minute": if total_minutes > 0.0 { total_candidates as f64 / total_minutes } else { 0.0 },
            "negative_controls_with_candidate": negative_with_candidate, "negative_controls_with_candidate_rate": ratio(negative_with_candidate, negatives.len()),
            "candidates_rejected_downstream": downstream_rejected, "final_negative_false_accepts": final_false_accepts
        }},
        "matching_diagnostics": matching.diagnostics,
        "kind_classification": {"per_class": kind_metrics, "confusion_matrix": confusion,
            "ambiguous_temporal_sets": {"denominator": ambiguous_kind_sets, "correct": correct_ambiguous_kind_sets, "multiset_accuracy": ratio(correct_ambiguous_kind_sets, ambiguous_kind_sets), "individual_pairs_excluded": matching.ambiguous_ground_truth.len()}},
        "speaker_alignment": speaker_alignment,
        "speaker_attribution": {"overall": speaker_overall, "duration_buckets": speaker_buckets, "contexts": speaker_contexts, "error_subtypes": speaker_error_subtypes,
            "ambiguous_temporal_sets": {"denominator": ambiguous_speaker_sets, "correct": correct_ambiguous_speaker_sets, "set_accuracy": ratio(correct_ambiguous_speaker_sets, ambiguous_speaker_sets), "individual_pairs_excluded": matching.ambiguous_ground_truth.len()}},
        "speaker_acceptance": speaker_acceptance,
        "materialization": {"visible_precision": ratio(visible_tp, predicted_visible), "visible_recall": ratio(visible_tp, expected_visible),
            "embedded_speaker_accuracy": ratio(embedded_correct, embedded_scored), "duplicate_render_rate": ratio(event_ids.len().saturating_sub(unique_events), event_ids.len()),
            "false_embedded_event_rate": ratio(embedded_false_accepts, negatives.iter().filter(|row| row.embedded.unwrap_or(false)).count()),
            "long_transcript_speaker_corruption_count": long_corruption, "manual_override_violation_count": manual_violations,
            "safety_observation_source": if production_mode { "production_artifact" } else { "not_exercised_by_annotated_evidence_replay" }},
        "confidence_reliability": confidence, "error_taxonomy": taxonomy_json,
        "model_free_sensitivity": {"status": if matches!(args.mode, BenchmarkMode::CounterfactualReplay) { "COUNTERFACTUAL_REPLAY_EXECUTED" } else { "NOT_RUN_WITHOUT_MEETING_LEVEL_DEVELOPMENT_FINAL_SPLIT" }, "meeting_leakage_guard": "required", "pareto_frontier": [], "original_artifact_immutable": true},
        "model_spike": {"executed": false, "reason": if decision == "INSUFFICIENT_REPRESENTATIVE_DATA" { "representative data gate failed; model selection forbidden" } else { "benchmark diagnosis requires review first" }}
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn overlap_ms(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> i64 {
    a_end.min(b_end).saturating_sub(a_start.max(b_start)).max(0)
}

fn overlap_iou(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> f64 {
    ratio(
        overlap_ms(a_start, a_end, b_start, b_end) as usize,
        (a_end.max(b_end) - a_start.min(b_start)).max(1) as usize,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn row(kind: &str) -> ManifestRow {
        serde_json::from_value(serde_json::json!({"ground_truth_event_id":"event-1","record_type":"ground_truth_event","recall_eligible":true,
            "evidence_origin":"annotated_evidence_replay","meeting_id":"meeting-1","start_ms":1000,"end_ms":1280,
            "duration_bucket":"100-300ms","ground_truth_kind":kind,"ground_truth_speaker":"speaker_01"})).unwrap()
    }

    fn prediction(kind: SegmentKind) -> Prediction {
        Prediction {
            candidate: ShortTurnCandidate {
                start_ms: 990,
                end_ms: 1290,
                duration_ms: 300,
                candidate_sources: vec![ShortTurnCandidateSource::VadEvent],
                transcript_ids: vec![],
                text: String::new(),
                asr_confidence: None,
                diarization_speaker: None,
                diarization_confidence: None,
                diarization_coverage_ratio: None,
                direct_diarization_turns: vec![],
                vad_confidence: None,
                audio_source: AudioSource::Imported,
                overlaps_existing_turn: false,
                true_speaker_overlap: false,
                previous_speaker: None,
                next_speaker: None,
                previous_gap_ms: None,
                next_gap_ms: None,
            },
            decision: ShortTurnDecision {
                kind,
                kind_confidence: 0.8,
                speaker_key: None,
                speaker_confidence: None,
                evidence: app_lib::diarization::short_turn::ShortTurnEvidence {
                    normalized_text: String::new(),
                    lexical_backchannel: false,
                    asr_confidence_present: false,
                    diarization_confidence_present: false,
                    base_speaker_confidence: 0.0,
                    duration_weight: 0.35,
                    effective_speaker_confidence: 0.0,
                    reason: "test".into(),
                },
                allow_new_speaker: false,
            },
            event: None,
        }
    }

    fn row_at(
        id: &str,
        start_ms: i64,
        end_ms: i64,
        kind: SegmentKind,
        speaker: Option<&str>,
    ) -> ManifestRow {
        let mut value = row("short_speech");
        value.ground_truth_event_id = id.into();
        value.start_ms = start_ms;
        value.end_ms = end_ms;
        value.duration_bucket = duration_bucket(end_ms - start_ms).into();
        value.ground_truth_kind = match kind {
            SegmentKind::Speech => GroundTruthKind::ShortSpeech,
            SegmentKind::Backchannel => GroundTruthKind::Backchannel,
            SegmentKind::Noise => GroundTruthKind::Noise,
            SegmentKind::NonSpeechVocalization => GroundTruthKind::NonSpeechVocalization,
            SegmentKind::Unknown => GroundTruthKind::Unknown,
        };
        value.ground_truth_speaker = speaker.map(str::to_owned);
        value
    }

    fn prediction_at(
        start_ms: i64,
        end_ms: i64,
        kind: SegmentKind,
        speaker: Option<&str>,
    ) -> Prediction {
        let mut value = prediction(kind);
        value.candidate.start_ms = start_ms;
        value.candidate.end_ms = end_ms;
        value.candidate.duration_ms = (end_ms - start_ms) as u64;
        value.decision.speaker_key = speaker.map(str::to_owned);
        value
    }

    fn production_artifact() -> MeetingProductionArtifact {
        MeetingProductionArtifact {
            schema_version: ARTIFACT_SCHEMA_VERSION,
            artifact_id: "artifact-1".into(),
            transcription_run_id: "transcription-run-1".into(),
            meeting_id: "meeting-1".into(),
            source_audio: SourceAudioMetadata {
                path_hint: None,
                duration_ms: 5_000,
                sha256: None,
            },
            created_at: "2026-09-14T00:00:00Z".into(),
            app_commit_sha: PHASE_2C1_FROZEN_BASELINE_COMMIT.into(),
            asr: ArtifactBackend {
                backend: "whisper.cpp".into(),
                model: "large-v3".into(),
                version_or_hash: None,
            },
            diarization: ArtifactBackend {
                backend: "sherpa-onnx".into(),
                model: "pyannote-3.0+campplus".into(),
                version_or_hash: None,
            },
            production_config: ProductionConfigSnapshot::default(),
            transcripts: vec![],
            raw_diarizer_turns: vec![],
            vad_events: vec![ArtifactVadEvent {
                start_ms: 990,
                end_ms: 1_290,
                confidence: None,
            }],
            accepted_speakers: vec![],
            visible_speakers: vec![],
            safety_observations: ProductionSafetyObservations::default(),
            production_metadata: serde_json::Value::Null,
        }
    }

    fn run_with_predictions(predictions: Vec<Prediction>) -> MeetingRun {
        MeetingRun {
            artifact: production_artifact(),
            replay_config: ProductionConfigSnapshot::default(),
            prototype_speakers: BTreeSet::new(),
            candidates: predictions
                .iter()
                .map(|prediction| prediction.candidate.clone())
                .collect(),
            predictions,
            predicted_accepted: BTreeSet::new(),
        }
    }

    fn artifact_dataset(rows: &mut [ManifestRow]) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("huitrace-phase2d-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("meeting-1.production.json"),
            serde_json::to_vec(&production_artifact()).unwrap(),
        )
        .unwrap();
        for row in rows {
            row.production_artifact_path = Some("meeting-1.production.json".into());
        }
        directory
    }

    #[test]
    fn rejected_noise_candidate_is_not_candidate_failure() {
        let row = row("noise");
        let prediction = prediction(SegmentKind::Noise);
        assert_eq!(
            root_cause(&row, Some(&prediction), Some(false), false),
            None
        );
    }

    #[test]
    fn missing_true_short_candidate_is_candidate_miss() {
        assert_eq!(
            root_cause(&row("short_speech"), None, Some(false), false),
            Some(RootCause::CandidateMiss)
        );
    }

    #[test]
    fn noise_accepted_as_speech_is_kind_error() {
        let row = row("noise");
        let prediction = prediction(SegmentKind::Speech);
        assert_eq!(
            root_cause(&row, Some(&prediction), Some(false), false),
            Some(RootCause::KindError)
        );
    }

    #[test]
    fn speaker_acceptance_full_alignment_preserves_existing_scoring() {
        let expected = BTreeSet::from(["gt_A".into(), "gt_B".into()]);
        let mapping = BTreeMap::from([
            ("gt_A".into(), "speaker_01".into()),
            ("gt_B".into(), "speaker_02".into()),
        ]);
        let matching = evaluate_speaker_acceptance(
            &expected,
            &mapping,
            &BTreeSet::from(["speaker_01".into(), "speaker_02".into()]),
        );
        assert!(matching.status.scorable);
        assert_eq!(matching.failed, Some(false));
        let json = serde_json::to_value(&matching.status).unwrap();
        assert_eq!(json["scorable"], true);
        assert!(json["reason"].is_null());
        assert_eq!(json["mapped_expected_speakers"], 2);
        assert_eq!(json["total_expected_speakers"], 2);

        let missing = evaluate_speaker_acceptance(
            &expected,
            &mapping,
            &BTreeSet::from(["speaker_01".into()]),
        );
        assert!(missing.status.scorable);
        assert_eq!(missing.failed, Some(true));
        let row = row("noise");
        let prediction = prediction(SegmentKind::Noise);
        assert_eq!(
            root_cause(&row, Some(&prediction), missing.failed, false),
            Some(RootCause::SpeakerAcceptanceError)
        );
    }

    #[test]
    fn partial_alignment_is_unscorable_even_with_an_extra_cluster() {
        let expected = BTreeSet::from(["gt_A".into(), "gt_B".into(), "gt_C".into()]);
        let mapping = BTreeMap::from([
            ("gt_A".into(), "speaker_01".into()),
            ("gt_B".into(), "speaker_02".into()),
        ]);
        assert_eq!(
            map_expected_speakers(&expected, &mapping),
            Err(vec!["gt_C".into()])
        );
        let evaluation = evaluate_speaker_acceptance(
            &expected,
            &mapping,
            &BTreeSet::from([
                "speaker_01".into(),
                "speaker_02".into(),
                "speaker_03".into(),
            ]),
        );
        assert!(!evaluation.status.scorable);
        assert_eq!(evaluation.failed, None);
        assert_eq!(evaluation.status.reason, Some("partial_speaker_alignment"));
        assert_eq!(evaluation.status.unmapped_expected_speakers, vec!["gt_C"]);
        let json = serde_json::to_value(&evaluation.status).unwrap();
        assert_eq!(json["scorable"], false);
        assert_eq!(json["reason"], "partial_speaker_alignment");
        assert_eq!(json["mapped_expected_speakers"], 2);
        assert_eq!(json["total_expected_speakers"], 3);
        assert_eq!(json["unmapped_expected_speakers"][0], "gt_C");

        let row = row("noise");
        let prediction = prediction(SegmentKind::Noise);
        assert_ne!(
            root_cause(&row, Some(&prediction), evaluation.failed, false),
            Some(RootCause::SpeakerAcceptanceError)
        );
    }

    #[test]
    fn unscorable_acceptance_does_not_hide_materialization_errors() {
        let mut row = row("noise");
        row.expected_materialized = Some(true);
        assert_eq!(
            root_cause(&row, Some(&prediction(SegmentKind::Noise)), None, false),
            Some(RootCause::MaterializationError)
        );
    }

    #[test]
    fn no_expected_speakers_are_reported_unscorable() {
        let evaluation =
            evaluate_speaker_acceptance(&BTreeSet::new(), &BTreeMap::new(), &BTreeSet::new());
        assert!(!evaluation.status.scorable);
        assert_eq!(evaluation.status.reason, Some("no_expected_speakers"));
        assert_eq!(evaluation.failed, None);
    }

    #[test]
    fn origin_string_cannot_replace_a_production_artifact() {
        let mut value = row("short_speech");
        value.evidence_origin = "production_artifact".into();
        assert!(validate_manifest(&[value], true).is_err());
    }

    #[test]
    fn overlapping_windows_cannot_duplicate_ground_truth() {
        let first = row("backchannel");
        let mut duplicate = first.clone();
        duplicate.ground_truth_event_id = "event-2".into();
        duplicate.start_ms += 10;
        duplicate.end_ms += 10;
        assert!(validate_manifest(&[first, duplicate], false).is_err());
    }

    #[test]
    fn insufficient_categories_fail_the_data_gate() {
        assert!(
            !coverage(&[row("short_speech")], &CoveragePolicy::default())
                .missing_requirements
                .is_empty()
        );
    }

    #[test]
    fn unknown_ground_truth_speaker_is_not_scored() {
        let mut value = row("short_speech");
        value.ground_truth_speaker = None;
        let predictions = HashMap::from([(value.ground_truth_event_id.clone(), None)]);
        assert_eq!(
            speaker_metrics(&[&value], &predictions, &HashSet::new()).denominator,
            0
        );
    }

    #[test]
    fn production_replay_loads_the_real_artifact_once_per_meeting() {
        let first = row("short_speech");
        let mut second = row("noise");
        second.ground_truth_event_id = "event-2".into();
        second.start_ms = 2_000;
        second.end_ms = 2_280;
        let mut rows = vec![first, second];
        let dataset = artifact_dataset(&mut rows);
        validate_manifest(&rows, true).unwrap();
        let loaded = load_production_artifacts(&dataset, &rows).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["meeting-1"].artifact.artifact_id, "artifact-1");
        fs::remove_dir_all(dataset).unwrap();
    }

    #[test]
    fn production_predictions_do_not_depend_on_the_current_gt_row() {
        let mut original = vec![row("short_speech")];
        let dataset = artifact_dataset(&mut original);
        let first = run_meeting(
            load_production_artifacts(&dataset, &original)
                .unwrap()
                .remove("meeting-1")
                .unwrap(),
            None,
        );
        original[0].transcript_text = "ground truth mutation".into();
        original[0].start_ms = 3_000;
        original[0].end_ms = 3_280;
        let second = run_meeting(
            load_production_artifacts(&dataset, &original)
                .unwrap()
                .remove("meeting-1")
                .unwrap(),
            None,
        );
        assert_eq!(first.candidates, second.candidates);
        fs::remove_dir_all(dataset).unwrap();
    }

    #[test]
    fn frozen_replay_uses_recorded_speaker_state_and_counterfactual_recomputes_it() {
        let mut artifact = production_artifact();
        artifact.raw_diarizer_turns.push(ArtifactDiarizerTurn {
            start_ms: 0,
            end_ms: 5_000,
            speaker_key: "speaker_01".into(),
            confidence: Some(0.95),
            overlap: false,
        });
        artifact.accepted_speakers.clear();
        let original = artifact.clone();

        let frozen = run_meeting(
            LoadedMeeting {
                artifact: artifact.clone(),
                provenance: None,
            },
            None,
        );
        assert!(frozen.predicted_accepted.contains("speaker_01"));
        assert!(frozen.prototype_speakers.is_empty());

        let counterfactual = run_meeting(
            LoadedMeeting {
                artifact,
                provenance: None,
            },
            Some(&ProductionConfigSnapshot::default()),
        );
        assert!(counterfactual.prototype_speakers.contains("speaker_01"));
        assert_eq!(original.accepted_speakers, Vec::<String>::new());
        assert_eq!(
            original.production_config,
            ProductionConfigSnapshot::default()
        );
    }

    fn matched_interval_map(result: &MatchResult<'_>) -> BTreeMap<String, [i64; 2]> {
        result
            .diagnostics
            .matched_pairs
            .iter()
            .map(|pair| {
                (
                    pair.ground_truth_event_id.clone(),
                    pair.prediction_interval_ms,
                )
            })
            .collect()
    }

    #[test]
    fn adversarial_matching_is_temporal_one_to_one_and_marks_ambiguity() {
        // Adjacent events and speaker handoffs retain their own prediction.
        let mut rows = vec![
            row_at("left", 1_000, 1_200, SegmentKind::Backchannel, Some("a")),
            row_at("right", 1_200, 1_400, SegmentKind::Backchannel, Some("b")),
        ];
        rows[0].tags.push("speaker_handoff".into());
        rows[1].tags.push("speaker_handoff".into());
        let run = run_with_predictions(vec![
            prediction_at(1_195, 1_405, SegmentKind::Backchannel, Some("b")),
            prediction_at(995, 1_205, SegmentKind::Backchannel, Some("a")),
        ]);
        let runs = BTreeMap::from([("meeting-1".into(), run)]);
        let matched = match_predictions(&rows, &runs);
        assert_eq!(
            matched["left"].unwrap().decision.speaker_key.as_deref(),
            Some("a")
        );
        assert_eq!(
            matched["right"].unwrap().decision.speaker_key.as_deref(),
            Some("b")
        );

        // One wide candidate cannot satisfy two annotations.
        let rows = vec![
            row_at("first", 1_000, 1_200, SegmentKind::Speech, None),
            row_at("second", 1_200, 1_400, SegmentKind::Speech, None),
        ];
        let run = run_with_predictions(vec![prediction_at(950, 1_350, SegmentKind::Speech, None)]);
        let runs = BTreeMap::from([("meeting-1".into(), run)]);
        let matched = match_predictions(&rows, &runs);
        assert_eq!(matched.values().filter(|value| value.is_some()).count(), 1);
        assert!(matched["first"].is_some());
        assert!(matched["second"].is_none());

        // Two candidates cannot satisfy one annotation; the tighter boundary wins.
        let rows = vec![row_at("single", 1_000, 1_200, SegmentKind::Speech, None)];
        let run = run_with_predictions(vec![
            prediction_at(900, 1_300, SegmentKind::Speech, None),
            prediction_at(1_000, 1_200, SegmentKind::Speech, None),
        ]);
        let runs = BTreeMap::from([("meeting-1".into(), run)]);
        let matched = match_predictions(&rows, &runs);
        assert_eq!(matched["single"].unwrap().candidate.start_ms, 1_000);

        // Same-time overlap is not secretly resolved with speaker labels.
        let mut rows = vec![
            row_at("speaker-a", 2_000, 2_300, SegmentKind::Speech, Some("a")),
            row_at("speaker-b", 2_000, 2_300, SegmentKind::Speech, Some("b")),
        ];
        rows[0].tags.push("overlap".into());
        rows[1].tags.push("overlap".into());
        let run = run_with_predictions(vec![
            prediction_at(2_000, 2_300, SegmentKind::Speech, Some("b")),
            prediction_at(2_000, 2_300, SegmentKind::Speech, Some("a")),
        ]);
        let runs = BTreeMap::from([("meeting-1".into(), run)]);
        let matched = match_predictions(&rows, &runs);
        assert_eq!(matched.ambiguous_ground_truth.len(), 2);
        assert_eq!(matched.diagnostics.ambiguous_matches.len(), 1);
        assert!(matched.diagnostics.ambiguous_matches[0].event_count_correct);
        assert_eq!(
            matched.diagnostics.ambiguous_matches[0].speaker_set_correct,
            Some(true)
        );

        // Nearby noise and speech pair by geometry even when their labels cross.
        let rows = vec![
            row_at("speech", 3_000, 3_250, SegmentKind::Speech, None),
            row_at("noise", 3_050, 3_300, SegmentKind::Noise, None),
        ];
        let run = run_with_predictions(vec![
            prediction_at(3_000, 3_250, SegmentKind::Noise, None),
            prediction_at(3_050, 3_300, SegmentKind::Speech, None),
        ]);
        let runs = BTreeMap::from([("meeting-1".into(), run)]);
        let matched = match_predictions(&rows, &runs);
        assert_eq!(matched["speech"].unwrap().decision.kind, SegmentKind::Noise);
        assert_eq!(matched["noise"].unwrap().decision.kind, SegmentKind::Speech);
    }

    #[test]
    fn matching_is_invariant_to_ground_truth_label_mutation_and_input_order() {
        let rows = vec![
            row_at("left", 1_000, 1_200, SegmentKind::Speech, Some("a")),
            row_at("right", 1_200, 1_400, SegmentKind::Noise, Some("b")),
        ];
        let predictions = vec![
            prediction_at(1_195, 1_405, SegmentKind::Speech, Some("b")),
            prediction_at(995, 1_205, SegmentKind::Noise, Some("a")),
        ];
        let original_runs = BTreeMap::from([(
            "meeting-1".into(),
            run_with_predictions(predictions.clone()),
        )]);
        let original = match_predictions(&rows, &original_runs);

        let mut mutated = rows.clone();
        mutated[0].ground_truth_kind = GroundTruthKind::Backchannel;
        mutated[0].ground_truth_speaker = Some("different".into());
        mutated[1].ground_truth_kind = GroundTruthKind::ShortSpeech;
        mutated[1].ground_truth_speaker = Some("also-different".into());
        mutated.reverse();
        let reordered_runs = BTreeMap::from([(
            "meeting-1".into(),
            run_with_predictions(predictions.into_iter().rev().collect()),
        )]);
        let after_mutation = match_predictions(&mutated, &reordered_runs);

        assert_eq!(
            matched_interval_map(&original),
            matched_interval_map(&after_mutation)
        );
        assert_ne!(
            class_metrics(&rows.iter().collect::<Vec<_>>(), &original.predictions).0,
            class_metrics(
                &mutated.iter().collect::<Vec<_>>(),
                &after_mutation.predictions
            )
            .0
        );
    }

    #[test]
    fn maximum_weight_alignment_recovers_swapped_speaker_clusters() {
        assert_eq!(
            maximum_weight_assignment(&[vec![0, 2_000], vec![1_500, 0]]),
            vec![Some(1), Some(0)]
        );
    }

    fn reference(id: &str, start: i64, end: i64, speaker: &str) -> ManifestRow {
        let mut value = row_at(id, start, end, SegmentKind::Speech, Some(speaker));
        value.ground_truth_kind = GroundTruthKind::OrdinarySpeechControl;
        value
    }

    fn alignment_run(turns: &[(i64, i64, &str)]) -> BTreeMap<String, MeetingRun> {
        let mut run = run_with_predictions(vec![]);
        run.artifact.raw_diarizer_turns = turns
            .iter()
            .map(|(start, end, speaker)| ArtifactDiarizerTurn {
                start_ms: *start,
                end_ms: *end,
                speaker_key: (*speaker).into(),
                confidence: None,
                overlap: false,
            })
            .collect();
        BTreeMap::from([("meeting-1".into(), run)])
    }

    #[test]
    fn reference_alignment_recovers_swapped_clusters_and_scores_short_turns() {
        let mut rows = vec![
            reference("a", 0, 2_000, "gt_A"),
            reference("b", 2_000, 4_000, "gt_B"),
            row_at(
                "short-a",
                4_000,
                4_300,
                SegmentKind::Backchannel,
                Some("gt_A"),
            ),
            row_at(
                "short-b",
                4_500,
                4_800,
                SegmentKind::Backchannel,
                Some("gt_B"),
            ),
        ];
        let report = align_ground_truth_speakers(
            &mut rows,
            &alignment_run(&[(0, 2_000, "speaker_02"), (2_000, 4_000, "speaker_01")]),
        );
        assert_eq!(report["meeting-1"].mapping["gt_A"], "speaker_02");
        assert_eq!(report["meeting-1"].mapping["gt_B"], "speaker_01");
        let a = prediction_at(4_000, 4_300, SegmentKind::Backchannel, Some("speaker_02"));
        let b = prediction_at(4_500, 4_800, SegmentKind::Backchannel, Some("speaker_01"));
        let metrics = speaker_metrics(
            &rows.iter().collect::<Vec<_>>(),
            &HashMap::from([("short-a".into(), Some(&a)), ("short-b".into(), Some(&b))]),
            &HashSet::new(),
        );
        assert_eq!(metrics.denominator, 2);
        assert_eq!(metrics.correct, 2);
    }

    #[test]
    fn synthetic_workflow_e_wrong_short_turn_cannot_change_mapping() {
        let mut rows = vec![
            reference("a", 0, 2_000, "gt_A"),
            reference("b", 2_000, 4_000, "gt_B"),
            row_at(
                "wrong",
                4_000,
                4_300,
                SegmentKind::Backchannel,
                Some("gt_A"),
            ),
        ];
        let runs = alignment_run(&[
            (0, 2_000, "speaker_01"),
            (2_000, 4_000, "speaker_02"),
            (4_000, 10_000, "speaker_02"),
        ]);
        // These disjoint wrong short turns would outweigh the references if they
        // leaked into the matrix. Their aggregate must have exactly zero influence.
        for i in 1..20 {
            rows.push(row_at(
                &format!("wrong-{i}"),
                4_000 + i * 300,
                4_300 + i * 300,
                SegmentKind::Backchannel,
                Some("gt_A"),
            ));
        }
        let report = align_ground_truth_speakers(&mut rows, &runs);
        assert_eq!(report["meeting-1"].mapping["gt_A"], "speaker_01");
        assert_eq!(report["meeting-1"].reference_intervals, 2);
        let wrong = prediction_at(4_000, 4_300, SegmentKind::Backchannel, Some("speaker_02"));
        assert_eq!(
            root_cause(&rows[2], Some(&wrong), Some(false), false),
            Some(RootCause::SpeakerAttributionError)
        );
        let metrics = speaker_metrics(
            &[&rows[2]],
            &HashMap::from([("wrong".into(), Some(&wrong))]),
            &HashSet::new(),
        );
        assert_eq!(metrics.denominator, 1);
        assert_eq!(metrics.correct, 0);
    }

    #[test]
    fn alignment_never_falls_back_to_short_uncertain_overlap_or_legacy_speech() {
        for label in [
            "short_speech",
            "backchannel",
            "noise",
            "non_speech_vocalization",
            "speech",
        ] {
            let mut value = row(label);
            value.end_ms = 5_000; // Duration never upgrades a label into a reference.
            let report = align_ground_truth_speakers(
                std::slice::from_mut(&mut value),
                &alignment_run(&[(0, 6_000, "speaker_01")]),
            );
            assert_eq!(report["meeting-1"].mapped_speakers, 0);
            assert_eq!(report["meeting-1"].reference_intervals, 0);
            assert!(value.ground_truth_speaker.is_none());
        }
        for (uncertain, overlap, duration) in [
            (true, false, 2_000),
            (false, true, 2_000),
            (false, false, 1_200),
        ] {
            let mut value = reference("a", 0, duration, "gt_A");
            value.annotation_uncertain = uncertain;
            if overlap {
                value.tags.push("overlap".into());
            }
            assert!(!is_speaker_reference(&value));
            let report = align_ground_truth_speakers(
                std::slice::from_mut(&mut value),
                &alignment_run(&[(0, 6_000, "speaker_01")]),
            );
            assert_eq!(report["meeting-1"].mapped_speakers, 0);
        }
        let mut value = reference("a", 0, 1_201, "gt_A");
        value.tags = vec!["speaker_handoff".into(), "embedded".into()];
        assert!(is_speaker_reference(&value));
        value.ground_truth_speaker = None;
        assert!(!is_speaker_reference(&value));
    }

    #[test]
    fn multiple_references_accumulate_and_unmapped_speaker_is_excluded() {
        let mut rows = vec![
            reference("a1", 0, 2_000, "gt_A"),
            reference("a2", 2_000, 4_000, "gt_A"),
            reference("a3", 4_000, 6_000, "gt_A"),
            reference("b", 6_000, 8_000, "gt_B"),
            row_at(
                "short-a",
                8_000,
                8_300,
                SegmentKind::Backchannel,
                Some("gt_A"),
            ),
            row_at(
                "short-b",
                8_300,
                8_600,
                SegmentKind::Backchannel,
                Some("gt_B"),
            ),
            row_at(
                "short-c",
                8_600,
                8_900,
                SegmentKind::Backchannel,
                Some("gt_C"),
            ),
        ];
        let report = align_ground_truth_speakers(
            &mut rows,
            &alignment_run(&[
                (0, 2_000, "speaker_02"),
                (2_000, 6_000, "speaker_01"),
                (6_000, 8_000, "speaker_02"),
                (8_600, 8_900, "speaker_03"),
            ]),
        );
        let coverage = &report["meeting-1"];
        assert_eq!(coverage.mapping["gt_A"], "speaker_01");
        assert_eq!(coverage.mapped_speakers, 2);
        assert_eq!(coverage.total_gt_speakers, 3);
        assert_eq!(coverage.unmapped_gt_speakers, vec!["gt_C"]);
        assert_eq!(coverage.reference_duration_ms, 8_000);
        assert_eq!(coverage.reference_intervals, 4);
        let metrics = speaker_metrics(
            &rows.iter().collect::<Vec<_>>(),
            &HashMap::new(),
            &HashSet::new(),
        );
        assert_eq!(metrics.denominator, 2);
        let json = serde_json::to_value(report).unwrap();
        assert_eq!(json["meeting-1"]["unmapped_gt_speakers"][0], "gt_C");
    }

    #[test]
    fn manifest_preserves_distinct_speech_labels() {
        for label in ["short_speech", "ordinary_speech_control", "speech"] {
            let value = row(label);
            assert_eq!(value.ground_truth_kind.as_label(), label);
            assert_eq!(value.ground_truth_kind.segment_kind(), SegmentKind::Speech);
        }
    }
}
