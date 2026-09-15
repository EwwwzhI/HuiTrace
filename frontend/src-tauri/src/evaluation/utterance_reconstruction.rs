//! Frozen Ground Truth and benchmark gate for utterance reconstruction.
//!
//! This module never runs ASR, VAD, diarization, or ShortTurn inference. It
//! serializes already-persisted evidence and replays only the deterministic
//! utterance reconstruction functions.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::audio::transcription::{TimingSource, TranscriptTiming};
use crate::database::models::Transcript;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::short_turn_event::ShortTurnEventsRepository;
use crate::database::repositories::speaker_turn::{SpeakerTurn, SpeakerTurnsRepository};
use crate::diarization::short_turn_event::ShortTurnEvent;
use crate::evaluation::production_artifact::{
    app_commit_sha, read_and_validate_artifact, ArtifactBackend, ARTIFACT_SCHEMA_VERSION,
};
use crate::utterance_reconstruction::{
    reconstruct_v2_with_config, ReconstructedUtterance, ReconstructionResult, SpeakerAttribution,
    UtteranceReconstructionConfig,
};
use crate::{context, state::AppState};

pub const RECONSTRUCTION_ARTIFACT_SCHEMA_VERSION: u32 = 1;
pub const UTTERANCE_GROUND_TRUTH_SCHEMA_VERSION: u32 = 1;
pub const RECONSTRUCTION_REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UtteranceGroundTruthManifestRow {
    pub meeting_id: String,
    pub reconstruction_artifact_id: String,
    pub artifact_sha256: String,
    pub source_audio_sha256: String,
    pub annotation_version: String,
    pub dataset_split: DatasetSplit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenFileIdentity {
    pub sha256: String,
    #[serde(default)]
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceProductionArtifactIdentity {
    pub artifact_id: String,
    pub sha256: String,
    pub schema_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtteranceReconstructionArtifact {
    pub schema_version: u32,
    pub artifact_id: String,
    pub meeting_id: String,
    pub created_at: String,
    pub app_commit_sha: String,
    pub source_production_artifact: SourceProductionArtifactIdentity,
    pub source_media: FrozenFileIdentity,
    pub transcription_run_id: String,
    pub asr: ArtifactBackend,
    pub transcripts: Vec<Transcript>,
    pub speaker_turns: Vec<SpeakerTurn>,
    pub short_turn_events: Vec<ShortTurnEvent>,
    pub v1: ReconstructionResult,
    pub v2: ReconstructionResult,
    /// SHA-256 of deterministic JSON with this field set to an empty string.
    pub integrity_sha256: String,
}

impl UtteranceReconstructionArtifact {
    pub fn seal(mut self) -> Result<Self> {
        self.integrity_sha256.clear();
        self.integrity_sha256 = payload_hash(&self)?;
        Ok(self)
    }

    pub fn validate(&self, expected_meeting_id: Option<&str>) -> Result<()> {
        if self.schema_version != RECONSTRUCTION_ARTIFACT_SCHEMA_VERSION {
            bail!("unsupported reconstruction artifact schema");
        }
        require_id("artifact_id", &self.artifact_id)?;
        require_id("meeting_id", &self.meeting_id)?;
        require_id("transcription_run_id", &self.transcription_run_id)?;
        require_id("app_commit_sha", &self.app_commit_sha)?;
        require_id(
            "source production artifact id",
            &self.source_production_artifact.artifact_id,
        )?;
        validate_sha256(
            "source production artifact",
            &self.source_production_artifact.sha256,
        )?;
        validate_sha256("source media", &self.source_media.sha256)?;
        validate_sha256("artifact integrity", &self.integrity_sha256)?;
        if self.source_production_artifact.schema_version != ARTIFACT_SCHEMA_VERSION {
            bail!("reconstruction artifact must reference Production Artifact schema v2");
        }
        if let Some(expected) = expected_meeting_id {
            if self.meeting_id != expected {
                bail!(
                    "reconstruction artifact belongs to meeting '{}', expected '{}'",
                    self.meeting_id,
                    expected
                );
            }
        }
        if self.v1.meeting_id != self.meeting_id || self.v2.meeting_id != self.meeting_id {
            bail!("reconstruction outputs belong to another meeting");
        }
        for (label, result) in [("V1", &self.v1), ("V2", &self.v2)] {
            require_id(
                &format!("{label} algorithm_version"),
                &result.algorithm_version,
            )?;
            require_id(&format!("{label} config_version"), &result.config_version)?;
            validate_sha256(&format!("{label} config"), &result.config_hash)?;
            if payload_hash(&result.config)? != result.config_hash {
                bail!("{label} config snapshot does not match its hash");
            }
        }
        if self
            .transcripts
            .iter()
            .any(|row| row.meeting_id != self.meeting_id)
            || self
                .short_turn_events
                .iter()
                .any(|event| event.meeting_id != self.meeting_id)
        {
            bail!("frozen evidence belongs to another meeting");
        }
        for transcript in &self.transcripts {
            let Some(json) = transcript.asr_timing_json.as_deref() else {
                continue;
            };
            let timing: TranscriptTiming =
                serde_json::from_str(json).context("parse frozen transcript timing")?;
            if timing.tokens.iter().any(|token| {
                token.start_ms < 0
                    || token.end_ms.is_some_and(|end| end < token.start_ms)
                    || (self.asr.backend.to_ascii_lowercase().contains("parakeet")
                        && token.timing_source != TimingSource::NativeTokenEmission)
            }) {
                bail!("frozen transcript contains invalid or mislabelled ASR timing");
            }
        }
        let mut unsigned = self.clone();
        unsigned.integrity_sha256.clear();
        if payload_hash(&unsigned)? != self.integrity_sha256 {
            bail!("reconstruction artifact integrity check failed");
        }
        Ok(())
    }

    pub fn deterministic_json(&self) -> Result<Vec<u8>> {
        self.validate(Some(&self.meeting_id))?;
        serde_json::to_vec_pretty(self).context("serialize reconstruction artifact")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetSplit {
    Calibration,
    Evaluation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioBucket {
    CleanSingleSpeaker,
    SameSpeakerContinuity,
    SameSpeakerBoundary,
    AsrFragmentation,
    VadFragmentation,
    SpeakerHandoff,
    Backchannel,
    ShortSpeech,
    TrueOverlap,
    NoisySpeech,
    Chinese,
    English,
    ChineseEnglishMixed,
    TimingUnavailableFallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GtSpeaker {
    pub key: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GtSpeakerInterval {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub gt_speaker_key: String,
    #[serde(default)]
    pub overlap: bool,
    #[serde(default)]
    pub uncertain: bool,
    #[serde(default)]
    pub status: GtAnnotationStatus,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GtAnnotationStatus {
    #[default]
    Pending,
    ConfirmedBlind,
    Reviewed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GtBoundaryKind {
    Utterance,
    SpeakerHandoff,
    ShortSpeech,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GtUtteranceBoundary {
    pub id: String,
    pub timestamp_ms: i64,
    pub boundary_kind: GtBoundaryKind,
    #[serde(default)]
    pub uncertain: bool,
    #[serde(default)]
    pub status: GtAnnotationStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GtUtterance {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub gt_speaker_key: String,
    #[serde(default)]
    pub boundary_start_uncertain: bool,
    #[serde(default)]
    pub boundary_end_uncertain: bool,
    #[serde(default)]
    pub contains_backchannel: bool,
    #[serde(default)]
    pub overlap: bool,
    #[serde(default)]
    pub buckets: BTreeSet<ScenarioBucket>,
    #[serde(default)]
    pub status: GtAnnotationStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UtteranceGroundTruth {
    pub schema_version: u32,
    pub annotation_version: String,
    pub meeting_id: String,
    pub reconstruction_artifact_id: String,
    pub reconstruction_artifact_sha256: String,
    pub source_audio_sha256: String,
    pub dataset_split: DatasetSplit,
    pub blind_complete: bool,
    pub review_complete: bool,
    pub qa_passed: bool,
    pub speakers: Vec<GtSpeaker>,
    pub speaker_intervals: Vec<GtSpeakerInterval>,
    pub utterances: Vec<GtUtterance>,
    pub boundaries: Vec<GtUtteranceBoundary>,
}

impl UtteranceGroundTruth {
    pub fn validate(&self, artifact: &UtteranceReconstructionArtifact) -> Result<()> {
        self.validate_structure(artifact)?;
        if !self.blind_complete || !self.review_complete || !self.qa_passed {
            bail!("pending annotation blocks benchmark/export");
        }
        if self.speakers.is_empty()
            || self.speaker_intervals.is_empty()
            || self.utterances.is_empty()
        {
            bail!(
                "completed ground truth must contain speakers, speaker intervals, and utterances"
            );
        }
        if self
            .speaker_intervals
            .iter()
            .any(|item| item.status != GtAnnotationStatus::Reviewed)
            || self
                .utterances
                .iter()
                .any(|item| item.status != GtAnnotationStatus::Reviewed)
            || self
                .boundaries
                .iter()
                .any(|item| item.status != GtAnnotationStatus::Reviewed)
        {
            bail!("pending annotation blocks benchmark/export");
        }
        Ok(())
    }

    fn validate_structure(&self, artifact: &UtteranceReconstructionArtifact) -> Result<()> {
        artifact.validate(Some(&self.meeting_id))?;
        if self.schema_version != UTTERANCE_GROUND_TRUTH_SCHEMA_VERSION {
            bail!("unsupported utterance ground-truth schema");
        }
        require_id("annotation_version", &self.annotation_version)?;
        if self.reconstruction_artifact_id != artifact.artifact_id
            || self.reconstruction_artifact_sha256 != artifact.integrity_sha256
            || self.source_audio_sha256 != artifact.source_media.sha256
        {
            bail!("ground truth is bound to a different frozen artifact or source media");
        }
        if self.review_complete && !self.blind_complete {
            bail!("Review cannot complete before Blind");
        }
        if self.qa_passed && !self.review_complete {
            bail!("QA cannot pass before Review");
        }
        let speakers = self
            .speakers
            .iter()
            .map(|speaker| speaker.key.as_str())
            .collect::<HashSet<_>>();
        if speakers.len() != self.speakers.len()
            || speakers.iter().any(|key| !key.starts_with("gt_speaker_"))
        {
            bail!("ground-truth speaker map is invalid or unstable");
        }
        let duration = artifact.source_media.duration_ms.unwrap_or(i64::MAX);
        let valid_interval = |start: i64, end: i64| start >= 0 && end > start && end <= duration;
        let mut ids = HashSet::new();
        for interval in &self.speaker_intervals {
            if !ids.insert(interval.id.as_str())
                || !valid_interval(interval.start_ms, interval.end_ms)
                || !speakers.contains(interval.gt_speaker_key.as_str())
            {
                bail!("invalid ground-truth speaker interval");
            }
        }
        for utterance in &self.utterances {
            if !ids.insert(utterance.id.as_str())
                || !valid_interval(utterance.start_ms, utterance.end_ms)
                || !speakers.contains(utterance.gt_speaker_key.as_str())
            {
                bail!("invalid ground-truth utterance");
            }
        }
        for boundary in &self.boundaries {
            if !ids.insert(boundary.id.as_str())
                || boundary.timestamp_ms < 0
                || boundary.timestamp_ms > duration
            {
                bail!("invalid ground-truth boundary");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationViewMode {
    Blind,
    Review,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconstructionReviewEvidence {
    pub raw_transcripts: Vec<Transcript>,
    pub speaker_turns: Vec<SpeakerTurn>,
    pub short_turn_events: Vec<ShortTurnEvent>,
    pub v1: ReconstructionResult,
    pub v2: ReconstructionResult,
    pub system_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtteranceAnnotationView {
    pub meeting_id: String,
    pub source_media: FrozenFileIdentity,
    pub ground_truth: UtteranceGroundTruth,
    pub review_evidence: Option<ReconstructionReviewEvidence>,
}

pub fn annotation_view(
    mode: AnnotationViewMode,
    artifact: &UtteranceReconstructionArtifact,
    ground_truth: UtteranceGroundTruth,
) -> Result<UtteranceAnnotationView> {
    artifact.validate(Some(&ground_truth.meeting_id))?;
    let review_evidence = match mode {
        AnnotationViewMode::Blind => None,
        AnnotationViewMode::Review => {
            if !ground_truth.blind_complete {
                bail!("review is locked until Blind annotation is complete");
            }
            Some(ReconstructionReviewEvidence {
                raw_transcripts: artifact.transcripts.clone(),
                speaker_turns: artifact.speaker_turns.clone(),
                short_turn_events: artifact.short_turn_events.clone(),
                v1: artifact.v1.clone(),
                v2: artifact.v2.clone(),
                system_label: "SYSTEM SUGGESTION — NOT GROUND TRUTH".into(),
            })
        }
    };
    Ok(UtteranceAnnotationView {
        meeting_id: artifact.meeting_id.clone(),
        source_media: artifact.source_media.clone(),
        ground_truth,
        review_evidence,
    })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SelectiveSpeakerMetrics {
    pub eligible: usize,
    pub assigned: usize,
    pub correct: usize,
    pub coverage: f64,
    pub assigned_only_accuracy: f64,
    pub selective_accuracy: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BoundaryMetrics {
    pub reference: usize,
    pub predicted: usize,
    pub matched: usize,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    pub over_segmentation_rate: f64,
    pub under_segmentation_rate: f64,
    /// User-facing aliases: an unmatched predicted boundary fragments speech.
    pub false_split_rate: f64,
    /// An unmatched reference boundary merges distinct utterances.
    pub false_merge_rate: f64,
    pub error_mae_ms: Option<f64>,
    pub error_median_ms: Option<f64>,
    pub error_p90_ms: Option<f64>,
    pub error_p95_ms: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SpecialCaseMetrics {
    pub backchannel_main_utterance_preservation_rate: f64,
    pub short_speech_boundary_recall: f64,
    pub overlap_mixed_or_abstention_recall: f64,
    pub overlap_false_single_speaker_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlgorithmMetrics {
    pub speaker_attribution: SelectiveSpeakerMetrics,
    pub handoff_250ms: BoundaryMetrics,
    pub handoff_500ms: BoundaryMetrics,
    pub utterance_boundary_250ms: BoundaryMetrics,
    pub utterance_boundary_500ms: BoundaryMetrics,
    pub special_cases: SpecialCaseMetrics,
    pub lexical_preservation: f64,
    /// Selective accuracy counts abstentions as incorrect, unlike assigned-only accuracy.
    pub speaker_attribution_accuracy: f64,
    /// Recall of reference speaker handoffs within the documented 500 ms collar.
    pub speaker_boundary_accuracy: f64,
    /// Signed prediction count minus selected Ground Truth utterance count.
    pub utterance_count_error: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorTaxonomy {
    TimingError,
    DiarizationError,
    AlignmentThresholdError,
    TrueOverlap,
    BackchannelError,
    ShortSpeechError,
    BoundaryOverSegmentation,
    BoundaryUnderSegmentation,
    LexicalGroupingError,
    TimingUnavailable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SweepPoint {
    pub assignment_min_overlap_ratio: f64,
    pub assignment_min_margin: f64,
    pub alignment_tolerance_ms: i64,
    pub true_overlap_min_ms: i64,
    pub coverage: f64,
    pub assigned_only_accuracy: f64,
    pub ambiguous_rate: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EmissionHandoffMetrics {
    pub matched_handoffs: usize,
    pub median_ms: Option<f64>,
    pub p90_ms: Option<f64>,
    pub p95_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorstCase {
    pub meeting_id: String,
    pub category: ErrorTaxonomy,
    pub start_ms: i64,
    pub end_ms: i64,
    pub source_transcript_ids: Vec<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UtteranceReconstructionReport {
    pub schema_version: u32,
    pub dataset_split: DatasetSplit,
    pub meeting_id: String,
    pub artifact_id: String,
    pub artifact_sha256: String,
    pub app_commit_sha: String,
    pub source_production_artifact: SourceProductionArtifactIdentity,
    pub source_media: FrozenFileIdentity,
    pub transcription_run_id: String,
    pub asr: ArtifactBackend,
    pub v1_algorithm_version: String,
    pub v1_config_hash: String,
    pub v1_config: UtteranceReconstructionConfig,
    pub v1_self_diagnostics: crate::utterance_reconstruction::ReconstructionMetrics,
    pub v2_algorithm_version: String,
    pub v2_config_hash: String,
    pub v2_config: UtteranceReconstructionConfig,
    pub v2_self_diagnostics: crate::utterance_reconstruction::ReconstructionMetrics,
    pub v1: AlgorithmMetrics,
    pub v2: AlgorithmMetrics,
    pub per_bucket_v1: BTreeMap<ScenarioBucket, AlgorithmMetrics>,
    pub per_bucket_v2: BTreeMap<ScenarioBucket, AlgorithmMetrics>,
    pub parameter_sweep: Vec<SweepPoint>,
    pub parakeet_emission_to_handoff_error: EmissionHandoffMetrics,
    pub errors: BTreeMap<String, usize>,
    pub worst_cases: Vec<WorstCase>,
    pub diagnostic_metrics_are_not_accuracy: bool,
    pub known_limitations: Vec<String>,
}

pub fn benchmark(
    artifact: &UtteranceReconstructionArtifact,
    ground_truth: &UtteranceGroundTruth,
) -> Result<UtteranceReconstructionReport> {
    ground_truth.validate(artifact)?;
    if !lexically_preserved(artifact, &artifact.v1) || !lexically_preserved(artifact, &artifact.v2)
    {
        bail!("LEXICAL_GROUPING_ERROR: lexical preservation must be 100%");
    }
    let mapping = align_speakers(&ground_truth.speaker_intervals, &artifact.speaker_turns);
    let v1 = score_algorithm(artifact, ground_truth, &artifact.v1, &mapping, None);
    let v2 = score_algorithm(artifact, ground_truth, &artifact.v2, &mapping, None);
    let mut per_bucket_v1 = BTreeMap::new();
    let mut per_bucket_v2 = BTreeMap::new();
    for bucket in all_buckets(ground_truth) {
        per_bucket_v1.insert(
            bucket,
            score_algorithm(artifact, ground_truth, &artifact.v1, &mapping, Some(bucket)),
        );
        per_bucket_v2.insert(
            bucket,
            score_algorithm(artifact, ground_truth, &artifact.v2, &mapping, Some(bucket)),
        );
    }
    let parameter_sweep = parameter_sweep(artifact, ground_truth, &mapping);
    let mut errors = BTreeMap::new();
    if v2.utterance_boundary_500ms.over_segmentation_rate > 0.0 {
        errors.insert(
            format!("{:?}", ErrorTaxonomy::BoundaryOverSegmentation),
            v2.utterance_boundary_500ms.predicted - v2.utterance_boundary_500ms.matched,
        );
    }
    if v2.utterance_boundary_500ms.under_segmentation_rate > 0.0 {
        errors.insert(
            format!("{:?}", ErrorTaxonomy::BoundaryUnderSegmentation),
            v2.utterance_boundary_500ms.reference - v2.utterance_boundary_500ms.matched,
        );
    }
    if artifact.v2.metrics.fallback_to_v1_count > 0 {
        errors.insert(
            format!("{:?}", ErrorTaxonomy::TimingUnavailable),
            artifact.v2.metrics.fallback_to_v1_count,
        );
    }
    Ok(UtteranceReconstructionReport {
        schema_version: RECONSTRUCTION_REPORT_SCHEMA_VERSION,
        dataset_split: ground_truth.dataset_split,
        meeting_id: artifact.meeting_id.clone(),
        artifact_id: artifact.artifact_id.clone(),
        artifact_sha256: artifact.integrity_sha256.clone(),
        app_commit_sha: artifact.app_commit_sha.clone(),
        source_production_artifact: artifact.source_production_artifact.clone(),
        source_media: artifact.source_media.clone(),
        transcription_run_id: artifact.transcription_run_id.clone(),
        asr: artifact.asr.clone(),
        v1_algorithm_version: artifact.v1.algorithm_version.clone(),
        v1_config_hash: artifact.v1.config_hash.clone(),
        v1_config: artifact.v1.config.clone(),
        v1_self_diagnostics: artifact.v1.metrics.clone(),
        v2_algorithm_version: artifact.v2.algorithm_version.clone(),
        v2_config_hash: artifact.v2.config_hash.clone(),
        v2_config: artifact.v2.config.clone(),
        v2_self_diagnostics: artifact.v2.metrics.clone(),
        v1,
        v2,
        per_bucket_v1,
        per_bucket_v2,
        parameter_sweep,
        parakeet_emission_to_handoff_error: emission_handoff_metrics(artifact, ground_truth),
        errors,
        worst_cases: worst_cases(artifact, ground_truth, &mapping),
        diagnostic_metrics_are_not_accuracy: true,
        known_limitations: vec![
            "True multi-speaker overlap text streams are not reconstructed".into(),
            "Whisper remains text-only without word-level timing".into(),
            "ASR lexical correctness is outside this benchmark".into(),
        ],
    })
}

fn emission_handoff_metrics(
    artifact: &UtteranceReconstructionArtifact,
    ground_truth: &UtteranceGroundTruth,
) -> EmissionHandoffMetrics {
    if !artifact
        .asr
        .backend
        .to_ascii_lowercase()
        .contains("parakeet")
    {
        return EmissionHandoffMetrics::default();
    }
    let mut diagnostics = artifact.v2.alignment_diagnostics.iter().collect::<Vec<_>>();
    diagnostics.sort_by_key(|item| (item.start_ms, item.end_ms));
    let transitions = diagnostics
        .windows(2)
        .filter_map(|pair| {
            let left = pair[0].speaker_key.as_deref()?;
            let right = pair[1].speaker_key.as_deref()?;
            (left != right).then_some(pair[1].start_ms)
        })
        .collect::<Vec<_>>();
    let handoffs = ground_truth
        .boundaries
        .iter()
        .filter(|item| !item.uncertain && item.boundary_kind == GtBoundaryKind::SpeakerHandoff)
        .map(|boundary| boundary.timestamp_ms)
        .collect::<Vec<_>>();
    let mut errors = match_boundary_errors(&handoffs, &transitions, None);
    errors.sort_by(f64::total_cmp);
    EmissionHandoffMetrics {
        matched_handoffs: errors.len(),
        median_ms: percentile(&errors, 0.50),
        p90_ms: percentile(&errors, 0.90),
        p95_ms: percentile(&errors, 0.95),
    }
}

pub async fn build_artifact_from_persisted_meeting(
    state: &AppState,
    meeting_id: &str,
    source_production_artifact_path: &Path,
    source_media_path: &Path,
) -> Result<UtteranceReconstructionArtifact> {
    let ctx = context::current();
    let production = read_and_validate_artifact(source_production_artifact_path, Some(meeting_id))?;
    let production_sha = file_hash(source_production_artifact_path)?;
    let media_sha = file_hash(source_media_path)?;
    if let Some(expected) = production.source_audio.sha256.as_deref() {
        let expected = normalize_sha(expected);
        if expected != media_sha {
            bail!("source media SHA-256 does not match Production Artifact");
        }
    }
    let pool = state.db_manager.pool();
    let transcripts =
        MeetingsRepository::get_all_meeting_transcripts(pool, &ctx, meeting_id).await?;
    let speaker_turns =
        SpeakerTurnsRepository::list_accepted_turns_for_meeting(pool, &ctx, meeting_id).await?;
    let short_turn_events =
        ShortTurnEventsRepository::list_for_meeting(pool, &ctx, meeting_id).await?;
    if transcripts.len() != production.transcripts.len()
        || transcripts
            .iter()
            .zip(&production.transcripts)
            .any(|(live, frozen)| {
                live.id != frozen.id
                    || live.transcript != frozen.text
                    || seconds_ms(live.audio_start_time) != Some(frozen.start_ms)
                    || seconds_ms(live.audio_end_time) != Some(frozen.end_ms)
            })
    {
        bail!("persisted transcripts no longer match the source Production Artifact");
    }
    let config = UtteranceReconstructionConfig::default();
    let v1 = crate::utterance_reconstruction::reconstruct_v1_with_config(
        meeting_id,
        &transcripts,
        &speaker_turns,
        &short_turn_events,
        &config,
    );
    let v2 = reconstruct_v2_with_config(
        meeting_id,
        &transcripts,
        &speaker_turns,
        &short_turn_events,
        &config,
    );
    UtteranceReconstructionArtifact {
        schema_version: RECONSTRUCTION_ARTIFACT_SCHEMA_VERSION,
        artifact_id: uuid::Uuid::new_v4().to_string(),
        meeting_id: meeting_id.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        app_commit_sha: app_commit_sha().to_string(),
        source_production_artifact: SourceProductionArtifactIdentity {
            artifact_id: production.artifact_id,
            sha256: production_sha,
            schema_version: production.schema_version,
        },
        source_media: FrozenFileIdentity {
            sha256: media_sha,
            duration_ms: Some(production.source_audio.duration_ms),
        },
        transcription_run_id: production.transcription_run_id,
        asr: production.asr,
        transcripts,
        speaker_turns,
        short_turn_events,
        v1,
        v2,
        integrity_sha256: String::new(),
    }
    .seal()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReconstructionArtifactRequest {
    pub meeting_id: String,
    pub source_production_artifact_path: PathBuf,
    pub source_media_path: PathBuf,
    pub output_path: PathBuf,
}

#[tauri::command]
pub async fn api_export_utterance_reconstruction_artifact(
    state: tauri::State<'_, AppState>,
    request: ExportReconstructionArtifactRequest,
) -> Result<String, String> {
    let artifact = build_artifact_from_persisted_meeting(
        &state,
        &request.meeting_id,
        &request.source_production_artifact_path,
        &request.source_media_path,
    )
    .await
    .map_err(|error| format!("build reconstruction artifact: {error:#}"))?;
    atomic_write(
        &request.output_path,
        &artifact.deterministic_json().map_err(|e| e.to_string())?,
    )
    .map_err(|error| format!("write reconstruction artifact: {error:#}"))?;
    Ok(artifact.integrity_sha256)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunBenchmarkRequest {
    pub artifact_path: PathBuf,
    pub ground_truth_path: PathBuf,
    pub output_directory: PathBuf,
}

#[tauri::command]
pub fn api_run_utterance_reconstruction_benchmark(
    request: RunBenchmarkRequest,
) -> Result<UtteranceReconstructionReport, String> {
    run_benchmark_files(
        &request.artifact_path,
        &request.ground_truth_path,
        &request.output_directory,
    )
    .map_err(|error| format!("run reconstruction benchmark: {error:#}"))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveGroundTruthRequest {
    pub artifact_path: PathBuf,
    pub ground_truth_path: PathBuf,
    pub ground_truth: UtteranceGroundTruth,
}

#[tauri::command]
pub fn api_save_utterance_ground_truth(request: SaveGroundTruthRequest) -> Result<(), String> {
    let artifact: UtteranceReconstructionArtifact = serde_json::from_slice(
        &std::fs::read(&request.artifact_path)
            .map_err(|error| format!("read reconstruction artifact: {error}"))?,
    )
    .map_err(|error| format!("parse reconstruction artifact: {error}"))?;
    // Draft autosave permits incomplete pass state, but never invalid identity,
    // speaker membership, timings, or a modified artifact.
    validate_ground_truth_draft(&request.ground_truth, &artifact)
        .map_err(|error| format!("validate utterance ground truth: {error:#}"))?;
    atomic_write(
        &request.ground_truth_path,
        &serde_json::to_vec_pretty(&request.ground_truth).map_err(|e| e.to_string())?,
    )
    .map_err(|error| format!("save utterance ground truth: {error:#}"))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadGroundTruthRequest {
    pub artifact_path: PathBuf,
    pub ground_truth_path: PathBuf,
    pub source_media_path: PathBuf,
    pub mode: AnnotationViewMode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeGroundTruthRequest {
    pub artifact_path: PathBuf,
    pub ground_truth_path: PathBuf,
    pub dataset_split: DatasetSplit,
}

#[tauri::command]
pub fn api_initialize_utterance_ground_truth(
    request: InitializeGroundTruthRequest,
) -> Result<UtteranceGroundTruth, String> {
    if request.ground_truth_path.exists() {
        return Err("ground-truth file already exists; reopen it instead".into());
    }
    let artifact: UtteranceReconstructionArtifact = serde_json::from_slice(
        &std::fs::read(&request.artifact_path)
            .map_err(|error| format!("read reconstruction artifact: {error}"))?,
    )
    .map_err(|error| format!("parse reconstruction artifact: {error}"))?;
    artifact
        .validate(None)
        .map_err(|error| format!("validate reconstruction artifact: {error:#}"))?;
    let ground_truth = UtteranceGroundTruth {
        schema_version: UTTERANCE_GROUND_TRUTH_SCHEMA_VERSION,
        annotation_version: "utterance-ground-truth-v1".into(),
        meeting_id: artifact.meeting_id.clone(),
        reconstruction_artifact_id: artifact.artifact_id.clone(),
        reconstruction_artifact_sha256: artifact.integrity_sha256.clone(),
        source_audio_sha256: artifact.source_media.sha256.clone(),
        dataset_split: request.dataset_split,
        blind_complete: false,
        review_complete: false,
        qa_passed: false,
        speakers: Vec::new(),
        speaker_intervals: Vec::new(),
        utterances: Vec::new(),
        boundaries: Vec::new(),
    };
    atomic_write(
        &request.ground_truth_path,
        &serde_json::to_vec_pretty(&ground_truth).map_err(|e| e.to_string())?,
    )
    .map_err(|error| format!("initialize utterance ground truth: {error:#}"))?;
    Ok(ground_truth)
}

#[tauri::command]
pub fn api_load_utterance_ground_truth(
    request: LoadGroundTruthRequest,
) -> Result<UtteranceAnnotationView, String> {
    let artifact: UtteranceReconstructionArtifact = serde_json::from_slice(
        &std::fs::read(&request.artifact_path)
            .map_err(|error| format!("read reconstruction artifact: {error}"))?,
    )
    .map_err(|error| format!("parse reconstruction artifact: {error}"))?;
    let ground_truth: UtteranceGroundTruth = serde_json::from_slice(
        &std::fs::read(&request.ground_truth_path)
            .map_err(|error| format!("read utterance ground truth: {error}"))?,
    )
    .map_err(|error| format!("parse utterance ground truth: {error}"))?;
    let media_sha = file_hash(&request.source_media_path)
        .map_err(|error| format!("hash source media: {error:#}"))?;
    if media_sha != artifact.source_media.sha256 {
        return Err("source media SHA-256 does not match reconstruction artifact".into());
    }
    validate_ground_truth_draft(&ground_truth, &artifact)
        .map_err(|error| format!("validate utterance ground truth: {error:#}"))?;
    annotation_view(request.mode, &artifact, ground_truth)
        .map_err(|error| format!("load utterance annotation view: {error:#}"))
}

fn validate_ground_truth_draft(
    ground_truth: &UtteranceGroundTruth,
    artifact: &UtteranceReconstructionArtifact,
) -> Result<()> {
    ground_truth.validate_structure(artifact)?;
    let statuses = ground_truth
        .speaker_intervals
        .iter()
        .map(|item| item.status)
        .chain(ground_truth.utterances.iter().map(|item| item.status))
        .chain(ground_truth.boundaries.iter().map(|item| item.status));
    if ground_truth.blind_complete
        && statuses
            .clone()
            .any(|status| status == GtAnnotationStatus::Pending)
    {
        bail!("pending annotation blocks Blind completion");
    }
    if ground_truth.review_complete
        && statuses
            .clone()
            .any(|status| status != GtAnnotationStatus::Reviewed)
    {
        bail!("Review completion requires every annotation to be reviewed");
    }
    if ground_truth.qa_passed {
        ground_truth.validate(artifact)?;
    }
    Ok(())
}

pub fn run_benchmark_files(
    artifact_path: &Path,
    ground_truth_path: &Path,
    output_directory: &Path,
) -> Result<UtteranceReconstructionReport> {
    let artifact: UtteranceReconstructionArtifact =
        serde_json::from_slice(&std::fs::read(artifact_path)?)?;
    let ground_truth: UtteranceGroundTruth =
        serde_json::from_slice(&std::fs::read(ground_truth_path)?)?;
    let report = benchmark(&artifact, &ground_truth)?;
    std::fs::create_dir_all(output_directory)?;
    atomic_write(
        &output_directory.join("utterance_reconstruction_report.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    atomic_write(
        &output_directory.join("utterance_reconstruction_report.md"),
        render_markdown(&report).as_bytes(),
    )?;
    Ok(report)
}

pub fn manifest_row(
    artifact: &UtteranceReconstructionArtifact,
    ground_truth: &UtteranceGroundTruth,
) -> Result<UtteranceGroundTruthManifestRow> {
    ground_truth.validate(artifact)?;
    Ok(UtteranceGroundTruthManifestRow {
        meeting_id: ground_truth.meeting_id.clone(),
        reconstruction_artifact_id: artifact.artifact_id.clone(),
        artifact_sha256: artifact.integrity_sha256.clone(),
        source_audio_sha256: artifact.source_media.sha256.clone(),
        annotation_version: ground_truth.annotation_version.clone(),
        dataset_split: ground_truth.dataset_split,
    })
}

pub fn validate_meeting_level_split(
    calibration: &[UtteranceGroundTruthManifestRow],
    evaluation: &[UtteranceGroundTruthManifestRow],
) -> Result<()> {
    if calibration
        .iter()
        .any(|row| row.dataset_split != DatasetSplit::Calibration)
        || evaluation
            .iter()
            .any(|row| row.dataset_split != DatasetSplit::Evaluation)
    {
        bail!("dataset split manifest contains a row in the wrong partition");
    }
    let calibration_ids = calibration
        .iter()
        .map(|row| row.meeting_id.as_str())
        .collect::<HashSet<_>>();
    if evaluation
        .iter()
        .any(|row| calibration_ids.contains(row.meeting_id.as_str()))
    {
        bail!("calibration and evaluation sets overlap at meeting level");
    }
    Ok(())
}

fn render_markdown(report: &UtteranceReconstructionReport) -> String {
    let mut rendered = format!(
        "# Utterance Reconstruction Evaluation\n\n- Meeting: `{}`\n- Artifact: `{}`\n- Split: `{:?}`\n- App commit: `{}`\n\n| Metric | V1 | V2 | Delta |\n|---|---:|---:|---:|\n| Speaker coverage | {:.4} | {:.4} | {:+.4} |\n| Assigned-only accuracy | {:.4} | {:.4} | {:+.4} |\n| Handoff F1 @250ms | {:.4} | {:.4} | {:+.4} |\n| Boundary F1 @250ms | {:.4} | {:.4} | {:+.4} |\n| Boundary F1 @500ms | {:.4} | {:.4} | {:+.4} |\n| Backchannel preservation | {:.4} | {:.4} | {:+.4} |\n| Short-speech recall | {:.4} | {:.4} | {:+.4} |\n| Overlap false-single rate | {:.4} | {:.4} | {:+.4} |\n| Lexical preservation | {:.4} | {:.4} | {:+.4} |\n\nDiagnostic coverage metrics are not accuracy. Parameter sweep rows are stored in the JSON report and must be calibrated only on calibration meetings.\n",
        report.meeting_id,
        report.artifact_id,
        report.dataset_split,
        report.app_commit_sha,
        report.v1.speaker_attribution.coverage,
        report.v2.speaker_attribution.coverage,
        report.v2.speaker_attribution.coverage - report.v1.speaker_attribution.coverage,
        report.v1.speaker_attribution.assigned_only_accuracy,
        report.v2.speaker_attribution.assigned_only_accuracy,
        report.v2.speaker_attribution.assigned_only_accuracy - report.v1.speaker_attribution.assigned_only_accuracy,
        report.v1.handoff_250ms.f1,
        report.v2.handoff_250ms.f1,
        report.v2.handoff_250ms.f1 - report.v1.handoff_250ms.f1,
        report.v1.utterance_boundary_250ms.f1,
        report.v2.utterance_boundary_250ms.f1,
        report.v2.utterance_boundary_250ms.f1 - report.v1.utterance_boundary_250ms.f1,
        report.v1.utterance_boundary_500ms.f1,
        report.v2.utterance_boundary_500ms.f1,
        report.v2.utterance_boundary_500ms.f1 - report.v1.utterance_boundary_500ms.f1,
        report.v1.special_cases.backchannel_main_utterance_preservation_rate,
        report.v2.special_cases.backchannel_main_utterance_preservation_rate,
        report.v2.special_cases.backchannel_main_utterance_preservation_rate - report.v1.special_cases.backchannel_main_utterance_preservation_rate,
        report.v1.special_cases.short_speech_boundary_recall,
        report.v2.special_cases.short_speech_boundary_recall,
        report.v2.special_cases.short_speech_boundary_recall - report.v1.special_cases.short_speech_boundary_recall,
        report.v1.special_cases.overlap_false_single_speaker_rate,
        report.v2.special_cases.overlap_false_single_speaker_rate,
        report.v2.special_cases.overlap_false_single_speaker_rate - report.v1.special_cases.overlap_false_single_speaker_rate,
        report.v1.lexical_preservation,
        report.v2.lexical_preservation,
        report.v2.lexical_preservation - report.v1.lexical_preservation,
    );
    rendered.push_str(&format!(
        "\n## Mainline quality indicators @500ms\n\n| Metric | V1 | V2/V3 latest | Delta |\n|---|---:|---:|---:|\n| False split rate | {:.4} | {:.4} | {:+.4} |\n| False merge rate | {:.4} | {:.4} | {:+.4} |\n| Speaker attribution accuracy | {:.4} | {:.4} | {:+.4} |\n| Speaker boundary accuracy | {:.4} | {:.4} | {:+.4} |\n| Utterance count error | {} | {} | {:+} |\n",
        report.v1.utterance_boundary_500ms.false_split_rate,
        report.v2.utterance_boundary_500ms.false_split_rate,
        report.v2.utterance_boundary_500ms.false_split_rate
            - report.v1.utterance_boundary_500ms.false_split_rate,
        report.v1.utterance_boundary_500ms.false_merge_rate,
        report.v2.utterance_boundary_500ms.false_merge_rate,
        report.v2.utterance_boundary_500ms.false_merge_rate
            - report.v1.utterance_boundary_500ms.false_merge_rate,
        report.v1.speaker_attribution_accuracy,
        report.v2.speaker_attribution_accuracy,
        report.v2.speaker_attribution_accuracy - report.v1.speaker_attribution_accuracy,
        report.v1.speaker_boundary_accuracy,
        report.v2.speaker_boundary_accuracy,
        report.v2.speaker_boundary_accuracy - report.v1.speaker_boundary_accuracy,
        report.v1.utterance_count_error,
        report.v2.utterance_count_error,
        report.v2.utterance_count_error - report.v1.utterance_count_error,
    ));
    rendered.push_str("\n## Frozen identity\n\n");
    rendered.push_str(&format!(
        "- Production artifact: `{}` (`{}`)\n- Source media: `{}`\n- Transcription run: `{}`\n- ASR: `{}` / `{}` / `{}`\n- V1 config: `{}`\n- V2 config: `{}`\n",
        report.source_production_artifact.artifact_id,
        report.source_production_artifact.sha256,
        report.source_media.sha256,
        report.transcription_run_id,
        report.asr.backend,
        report.asr.model,
        report.asr.version_or_hash.as_deref().unwrap_or("unknown"),
        report.v1_config_hash,
        report.v2_config_hash,
    ));
    rendered.push_str("\n## Known limitations\n\n");
    for limitation in &report.known_limitations {
        rendered.push_str(&format!("- {limitation}\n"));
    }
    rendered
}

fn file_hash(path: &Path) -> Result<String> {
    Ok(format!("sha256:{:x}", Sha256::digest(std::fs::read(path)?)))
}

fn normalize_sha(value: &str) -> String {
    if value.starts_with("sha256:") {
        value.to_string()
    } else {
        format!("sha256:{value}")
    }
}

fn seconds_ms(value: Option<f64>) -> Option<i64> {
    value
        .filter(|item| item.is_finite())
        .map(|item| (item * 1_000.0).round() as i64)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&temp, bytes)?;
    if !path.exists() {
        return std::fs::rename(&temp, path)
            .with_context(|| format!("atomically create {}", path.display()));
    }
    // `rename(temp, existing)` is not portable to Windows. Keep a same-directory
    // rollback copy so repeated autosaves replace the prior revision safely.
    let backup = path.with_extension(format!("bak-{}", uuid::Uuid::new_v4()));
    std::fs::rename(path, &backup)
        .with_context(|| format!("stage prior revision of {}", path.display()))?;
    match std::fs::rename(&temp, path) {
        Ok(()) => {
            let _ = std::fs::remove_file(backup);
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::rename(&backup, path);
            let _ = std::fs::remove_file(temp);
            Err(error).with_context(|| format!("replace {}", path.display()))
        }
    }
}

fn score_algorithm(
    artifact: &UtteranceReconstructionArtifact,
    gt: &UtteranceGroundTruth,
    result: &ReconstructionResult,
    mapping: &BTreeMap<String, String>,
    bucket: Option<ScenarioBucket>,
) -> AlgorithmMetrics {
    let selected = gt
        .utterances
        .iter()
        .filter(|item| bucket.map_or(true, |wanted| item.buckets.contains(&wanted)))
        .collect::<Vec<_>>();
    let speaker_attribution = speaker_metrics(&selected, &result.utterances, mapping);
    let utterance_boundaries = gt
        .boundaries
        .iter()
        .filter(|item| !item.uncertain && item.boundary_kind == GtBoundaryKind::Utterance)
        .filter(|boundary| boundary_in_selected(boundary.timestamp_ms, &selected, bucket))
        .map(|item| item.timestamp_ms)
        .collect::<Vec<_>>();
    let handoffs = gt
        .boundaries
        .iter()
        .filter(|item| !item.uncertain && item.boundary_kind == GtBoundaryKind::SpeakerHandoff)
        .filter(|boundary| boundary_in_selected(boundary.timestamp_ms, &selected, bucket))
        .map(|item| item.timestamp_ms)
        .collect::<Vec<_>>();
    let predicted = predicted_boundaries(&result.utterances, false)
        .into_iter()
        .filter(|timestamp| boundary_in_selected(*timestamp, &selected, bucket))
        .collect::<Vec<_>>();
    let predicted_handoffs = predicted_boundaries(&result.utterances, true)
        .into_iter()
        .filter(|timestamp| boundary_in_selected(*timestamp, &selected, bucket))
        .collect::<Vec<_>>();
    let handoff_250ms = boundary_metrics(&handoffs, &predicted_handoffs, 250);
    let handoff_500ms = boundary_metrics(&handoffs, &predicted_handoffs, 500);
    let utterance_boundary_250ms = boundary_metrics(&utterance_boundaries, &predicted, 250);
    let utterance_boundary_500ms = boundary_metrics(&utterance_boundaries, &predicted, 500);
    AlgorithmMetrics {
        speaker_attribution_accuracy: speaker_attribution.selective_accuracy,
        speaker_boundary_accuracy: handoff_500ms.recall,
        utterance_count_error: (if selected.is_empty() {
            0
        } else {
            predicted.len() + 1
        }) as i64
            - selected.len() as i64,
        speaker_attribution,
        handoff_250ms,
        handoff_500ms,
        utterance_boundary_250ms,
        utterance_boundary_500ms,
        special_cases: special_case_metrics(gt, &selected, result, mapping),
        lexical_preservation: if lexically_preserved(artifact, result) {
            1.0
        } else {
            0.0
        },
    }
}

fn speaker_metrics(
    gt: &[&GtUtterance],
    predicted: &[ReconstructedUtterance],
    mapping: &BTreeMap<String, String>,
) -> SelectiveSpeakerMetrics {
    let eligible = gt
        .iter()
        .filter(|item| {
            !item.overlap && !item.boundary_start_uncertain && !item.boundary_end_uncertain
        })
        .collect::<Vec<_>>();
    let mut assigned = 0;
    let mut correct = 0;
    for reference in &eligible {
        let Some(hypothesis) = best_overlapping(reference.start_ms, reference.end_ms, predicted)
        else {
            continue;
        };
        if let SpeakerAttribution::Single { speaker_key } = &hypothesis.speaker_attribution {
            assigned += 1;
            if mapping.get(&reference.gt_speaker_key) == Some(speaker_key) {
                correct += 1;
            }
        }
    }
    SelectiveSpeakerMetrics {
        eligible: eligible.len(),
        assigned,
        correct,
        coverage: ratio(assigned, eligible.len()),
        assigned_only_accuracy: ratio(correct, assigned),
        selective_accuracy: ratio(correct, eligible.len()),
    }
}

fn boundary_metrics(reference: &[i64], predicted: &[i64], collar_ms: i64) -> BoundaryMetrics {
    let mut errors = match_boundary_errors(reference, predicted, Some(collar_ms));
    errors.sort_by(f64::total_cmp);
    let matched = errors.len();
    let precision = ratio(matched, predicted.len());
    let recall = ratio(matched, reference.len());
    BoundaryMetrics {
        reference: reference.len(),
        predicted: predicted.len(),
        matched,
        precision,
        recall,
        f1: harmonic(precision, recall),
        over_segmentation_rate: ratio(predicted.len().saturating_sub(matched), predicted.len()),
        under_segmentation_rate: ratio(reference.len().saturating_sub(matched), reference.len()),
        false_split_rate: ratio(predicted.len().saturating_sub(matched), predicted.len()),
        false_merge_rate: ratio(reference.len().saturating_sub(matched), reference.len()),
        error_mae_ms: mean(&errors),
        error_median_ms: percentile(&errors, 0.50),
        error_p90_ms: percentile(&errors, 0.90),
        error_p95_ms: percentile(&errors, 0.95),
    }
}

fn match_boundary_errors(reference: &[i64], predicted: &[i64], collar_ms: Option<i64>) -> Vec<f64> {
    let mut used = vec![false; predicted.len()];
    let mut errors = Vec::new();
    for expected in reference {
        let closest = predicted
            .iter()
            .enumerate()
            .filter(|(index, _)| !used[*index])
            .filter_map(|(index, actual)| {
                let error = (actual - expected).abs();
                collar_ms
                    .map_or(true, |collar| error <= collar)
                    .then_some((index, error, *actual))
            })
            .min_by_key(|(_, error, actual)| (*error, *actual));
        if let Some((index, error, _)) = closest {
            used[index] = true;
            errors.push(error as f64);
        }
    }
    errors
}

fn special_case_metrics(
    ground_truth: &UtteranceGroundTruth,
    gt: &[&GtUtterance],
    result: &ReconstructionResult,
    mapping: &BTreeMap<String, String>,
) -> SpecialCaseMetrics {
    let backchannels = gt
        .iter()
        .filter(|item| item.contains_backchannel)
        .collect::<Vec<_>>();
    let preserved = backchannels
        .iter()
        .filter(|item| {
            best_overlapping(item.start_ms, item.end_ms, &result.utterances).is_some_and(|hyp| {
                mapping.get(&item.gt_speaker_key).map(String::as_str)
                    == hyp.speaker_attribution.single_key()
                    && !hyp.embedded_events.is_empty()
            })
        })
        .count();
    let short_boundaries = ground_truth
        .boundaries
        .iter()
        .filter(|item| !item.uncertain && item.boundary_kind == GtBoundaryKind::ShortSpeech)
        .filter(|item| {
            boundary_in_selected(item.timestamp_ms, gt, Some(ScenarioBucket::ShortSpeech))
        })
        .map(|item| item.timestamp_ms)
        .collect::<Vec<_>>();
    let short_metrics = boundary_metrics(
        &short_boundaries,
        &predicted_boundaries(&result.utterances, false),
        500,
    );
    let overlaps = gt.iter().filter(|item| item.overlap).collect::<Vec<_>>();
    let false_single = overlaps
        .iter()
        .filter(|item| {
            best_overlapping(item.start_ms, item.end_ms, &result.utterances).is_some_and(|hyp| {
                matches!(hyp.speaker_attribution, SpeakerAttribution::Single { .. })
            })
        })
        .count();
    SpecialCaseMetrics {
        backchannel_main_utterance_preservation_rate: ratio(preserved, backchannels.len()),
        short_speech_boundary_recall: short_metrics.recall,
        overlap_mixed_or_abstention_recall: ratio(
            overlaps.len().saturating_sub(false_single),
            overlaps.len(),
        ),
        overlap_false_single_speaker_rate: ratio(false_single, overlaps.len()),
    }
}

fn parameter_sweep(
    artifact: &UtteranceReconstructionArtifact,
    gt: &UtteranceGroundTruth,
    mapping: &BTreeMap<String, String>,
) -> Vec<SweepPoint> {
    let mut points = Vec::new();
    for overlap in [0.50, 0.60, 0.70, 0.80] {
        for margin in [0.10, 0.20, 0.30] {
            for tolerance in [25, 50, 75, 100] {
                for true_overlap in [50, 100, 150] {
                    let mut config = artifact.v2.config.clone();
                    config.assignment_min_overlap_ratio = overlap;
                    config.assignment_min_margin = margin;
                    config.alignment_tolerance_ms = tolerance;
                    config.true_overlap_min_ms = true_overlap;
                    let replay = reconstruct_v2_with_config(
                        &artifact.meeting_id,
                        &artifact.transcripts,
                        &artifact.speaker_turns,
                        &artifact.short_turn_events,
                        &config,
                    );
                    let metrics = speaker_metrics(
                        &gt.utterances.iter().collect::<Vec<_>>(),
                        &replay.utterances,
                        mapping,
                    );
                    points.push(SweepPoint {
                        assignment_min_overlap_ratio: overlap,
                        assignment_min_margin: margin,
                        alignment_tolerance_ms: tolerance,
                        true_overlap_min_ms: true_overlap,
                        coverage: metrics.coverage,
                        assigned_only_accuracy: metrics.assigned_only_accuracy,
                        ambiguous_rate: replay.metrics.ambiguous_rate,
                    });
                }
            }
        }
    }
    points
}

fn align_speakers(
    references: &[GtSpeakerInterval],
    system: &[SpeakerTurn],
) -> BTreeMap<String, String> {
    let mut weights: BTreeMap<(String, String), i64> = BTreeMap::new();
    for gt in references
        .iter()
        .filter(|item| !item.overlap && !item.uncertain)
    {
        for turn in system {
            let overlap = overlap_ms(gt.start_ms, gt.end_ms, turn.start_ms, turn.end_ms);
            if overlap > 0 {
                *weights
                    .entry((gt.gt_speaker_key.clone(), turn.speaker_key.clone()))
                    .or_default() += overlap;
            }
        }
    }
    let gt_keys = weights
        .keys()
        .map(|(gt, _)| gt.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let system_keys = weights
        .keys()
        .map(|(_, system)| system.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    // Exact maximum-weight one-to-one assignment. A greedy largest-edge pass
    // can choose the wrong speaker permutation even in a two-speaker meeting.
    let mut states = BTreeMap::from([(vec![false; system_keys.len()], (0_i64, Vec::new()))]);
    for gt in &gt_keys {
        let mut next = BTreeMap::new();
        for (used, (score, assignment)) in states {
            update_assignment_state(&mut next, used.clone(), score, &assignment, None);
            for (index, system_key) in system_keys.iter().enumerate() {
                if used[index] {
                    continue;
                }
                let weight = *weights.get(&(gt.clone(), system_key.clone())).unwrap_or(&0);
                if weight <= 0 {
                    continue;
                }
                let mut newly_used = used.clone();
                newly_used[index] = true;
                update_assignment_state(
                    &mut next,
                    newly_used,
                    score + weight,
                    &assignment,
                    Some(index),
                );
            }
        }
        states = next;
    }
    let (_, (_, assignment)) = states
        .into_iter()
        .max_by(|left, right| {
            left.1
                 .0
                .cmp(&right.1 .0)
                .then_with(|| right.0.cmp(&left.0))
        })
        .unwrap_or_default();
    gt_keys
        .into_iter()
        .zip(assignment)
        .filter_map(|(gt, system_index)| system_index.map(|index| (gt, system_keys[index].clone())))
        .collect()
}

fn update_assignment_state(
    states: &mut BTreeMap<Vec<bool>, (i64, Vec<Option<usize>>)>,
    used: Vec<bool>,
    score: i64,
    assignment: &[Option<usize>],
    selected: Option<usize>,
) {
    let entry = states.entry(used).or_insert_with(|| (i64::MIN, Vec::new()));
    if score > entry.0 {
        let mut next_assignment = assignment.to_vec();
        next_assignment.push(selected);
        *entry = (score, next_assignment);
    }
}

fn predicted_boundaries(utterances: &[ReconstructedUtterance], handoff_only: bool) -> Vec<i64> {
    utterances
        .windows(2)
        .filter(|pair| {
            !handoff_only
                || pair[0].speaker_attribution.single_key().is_some()
                    && pair[1].speaker_attribution.single_key().is_some()
                    && pair[0].speaker_attribution.single_key()
                        != pair[1].speaker_attribution.single_key()
        })
        .map(|pair| (pair[0].end_ms + pair[1].start_ms) / 2)
        .collect()
}

fn best_overlapping(
    start_ms: i64,
    end_ms: i64,
    utterances: &[ReconstructedUtterance],
) -> Option<&ReconstructedUtterance> {
    utterances
        .iter()
        .max_by_key(|item| overlap_ms(start_ms, end_ms, item.start_ms, item.end_ms))
        .filter(|item| overlap_ms(start_ms, end_ms, item.start_ms, item.end_ms) > 0)
}

fn lexically_preserved(
    artifact: &UtteranceReconstructionArtifact,
    result: &ReconstructionResult,
) -> bool {
    let source = artifact
        .transcripts
        .iter()
        .map(|item| item.transcript.as_str())
        .collect::<String>();
    let hypothesis = result
        .utterances
        .iter()
        .map(|item| item.text.as_str())
        .collect::<String>();
    normalize_lexical(&source) == normalize_lexical(&hypothesis)
}

fn normalize_lexical(value: &str) -> String {
    value.split_whitespace().collect::<String>()
}

fn worst_cases(
    artifact: &UtteranceReconstructionArtifact,
    gt: &UtteranceGroundTruth,
    mapping: &BTreeMap<String, String>,
) -> Vec<WorstCase> {
    gt.utterances
        .iter()
        .filter_map(|reference| {
            let hypothesis = best_overlapping(
                reference.start_ms,
                reference.end_ms,
                &artifact.v2.utterances,
            )?;
            let wrong_single = hypothesis.speaker_attribution.single_key().is_some()
                && (reference.overlap
                    || mapping.get(&reference.gt_speaker_key).map(String::as_str)
                        != hypothesis.speaker_attribution.single_key());
            wrong_single.then(|| WorstCase {
                meeting_id: artifact.meeting_id.clone(),
                category: if reference.overlap {
                    ErrorTaxonomy::TrueOverlap
                } else {
                    ErrorTaxonomy::Unknown
                },
                start_ms: reference.start_ms,
                end_ms: reference.end_ms,
                source_transcript_ids: hypothesis.source_transcript_ids.clone(),
                detail: format!(
                    "GT speaker={}, system={:?}",
                    reference.gt_speaker_key, hypothesis.speaker_attribution
                ),
            })
        })
        .take(20)
        .collect()
}

fn all_buckets(gt: &UtteranceGroundTruth) -> BTreeSet<ScenarioBucket> {
    gt.utterances
        .iter()
        .flat_map(|item| item.buckets.iter().copied())
        .collect()
}

fn boundary_in_selected(
    timestamp_ms: i64,
    selected: &[&GtUtterance],
    bucket: Option<ScenarioBucket>,
) -> bool {
    bucket.is_none()
        || selected
            .iter()
            .any(|item| timestamp_ms >= item.start_ms && timestamp_ms <= item.end_ms)
}

fn overlap_ms(left_start: i64, left_end: i64, right_start: i64, right_end: i64) -> i64 {
    left_end
        .min(right_end)
        .saturating_sub(left_start.max(right_start))
        .max(0)
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn harmonic(precision: f64, recall: f64) -> f64 {
    if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    }
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn percentile(values: &[f64], percentile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let index = ((values.len() - 1) as f64 * percentile).ceil() as usize;
    values.get(index).copied()
}

fn payload_hash<T: Serialize>(value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn validate_sha256(label: &str, value: &str) -> Result<()> {
    let hash = value.strip_prefix("sha256:").unwrap_or(value);
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{label} SHA-256 is invalid");
    }
    Ok(())
}

fn require_id(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{label} is missing");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utterance_reconstruction::{reconstruct_v1_with_config, ALGORITHM_VERSION};

    fn transcript() -> Transcript {
        Transcript {
            id: "t1".into(),
            meeting_id: "m1".into(),
            transcript: "hello world".into(),
            timestamp: String::new(),
            summary: None,
            action_items: None,
            key_points: None,
            audio_start_time: Some(0.0),
            audio_end_time: Some(2.0),
            duration: Some(2.0),
            asr_confidence: None,
            speaker_id: Some("system_a".into()),
            speaker_confidence: Some(1.0),
            speaker_provisional: 0,
            speaker_revision: 0,
            segment_kind: Some("speech".into()),
            audio_source: None,
            speaker_assignment_method: "automatic".into(),
            speaker_overlap: 0,
            asr_timing_json: None,
        }
    }

    fn artifact() -> UtteranceReconstructionArtifact {
        let transcripts = vec![transcript()];
        let turns = vec![SpeakerTurn {
            start_ms: 0,
            end_ms: 2_000,
            speaker_label: "A".into(),
            confidence: Some(1.0),
            speaker_key: "system_a".into(),
        }];
        let config = UtteranceReconstructionConfig::default();
        let v1 = reconstruct_v1_with_config("m1", &transcripts, &turns, &[], &config);
        let v2 = reconstruct_v2_with_config("m1", &transcripts, &turns, &[], &config);
        UtteranceReconstructionArtifact {
            schema_version: 1,
            artifact_id: "artifact-1".into(),
            meeting_id: "m1".into(),
            created_at: "2026-09-15T00:00:00Z".into(),
            app_commit_sha: "4b622f7".into(),
            source_production_artifact: SourceProductionArtifactIdentity {
                artifact_id: "production-1".into(),
                sha256: format!("sha256:{}", "a".repeat(64)),
                schema_version: 2,
            },
            source_media: FrozenFileIdentity {
                sha256: format!("sha256:{}", "b".repeat(64)),
                duration_ms: Some(2_000),
            },
            transcription_run_id: "run-1".into(),
            asr: ArtifactBackend {
                backend: "parakeet".into(),
                model: "test".into(),
                version_or_hash: None,
            },
            transcripts,
            speaker_turns: turns,
            short_turn_events: vec![],
            v1,
            v2,
            integrity_sha256: String::new(),
        }
        .seal()
        .unwrap()
    }

    fn ground_truth() -> UtteranceGroundTruth {
        UtteranceGroundTruth {
            schema_version: 1,
            annotation_version: "v1".into(),
            meeting_id: "m1".into(),
            reconstruction_artifact_id: "artifact-1".into(),
            reconstruction_artifact_sha256: artifact().integrity_sha256,
            source_audio_sha256: format!("sha256:{}", "b".repeat(64)),
            dataset_split: DatasetSplit::Evaluation,
            blind_complete: true,
            review_complete: true,
            qa_passed: true,
            speakers: vec![GtSpeaker {
                key: "gt_speaker_01".into(),
                description: String::new(),
            }],
            speaker_intervals: vec![GtSpeakerInterval {
                id: "si1".into(),
                start_ms: 0,
                end_ms: 2_000,
                gt_speaker_key: "gt_speaker_01".into(),
                overlap: false,
                uncertain: false,
                status: GtAnnotationStatus::Reviewed,
            }],
            utterances: vec![GtUtterance {
                id: "u1".into(),
                start_ms: 0,
                end_ms: 2_000,
                gt_speaker_key: "gt_speaker_01".into(),
                boundary_start_uncertain: false,
                boundary_end_uncertain: false,
                contains_backchannel: false,
                overlap: false,
                buckets: BTreeSet::from([ScenarioBucket::English]),
                status: GtAnnotationStatus::Reviewed,
            }],
            boundaries: vec![],
        }
    }

    #[test]
    fn artifact_serialization_is_deterministic_and_modification_fails_closed() {
        let artifact = artifact();
        assert_eq!(
            artifact.deterministic_json().unwrap(),
            artifact.deterministic_json().unwrap()
        );
        let mut modified = artifact.clone();
        modified.transcripts[0].transcript.push('!');
        assert!(modified.validate(None).is_err());
    }

    #[test]
    fn wrong_meeting_and_wrong_artifact_are_rejected() {
        let artifact = artifact();
        assert!(artifact.validate(Some("m2")).is_err());
        let mut gt = ground_truth();
        gt.reconstruction_artifact_id = "other".into();
        assert!(gt.validate(&artifact).is_err());
    }

    #[test]
    fn blind_never_contains_system_evidence_and_review_requires_completion() {
        let artifact = artifact();
        let mut gt = ground_truth();
        let blind = annotation_view(AnnotationViewMode::Blind, &artifact, gt.clone()).unwrap();
        assert!(blind.review_evidence.is_none());
        gt.blind_complete = false;
        assert!(annotation_view(AnnotationViewMode::Review, &artifact, gt).is_err());
        let review =
            annotation_view(AnnotationViewMode::Review, &artifact, ground_truth()).unwrap();
        assert!(review
            .review_evidence
            .unwrap()
            .system_label
            .contains("NOT GROUND TRUTH"));
    }

    #[test]
    fn pending_annotation_blocks_benchmark() {
        let artifact = artifact();
        let mut gt = ground_truth();
        gt.qa_passed = false;
        assert!(benchmark(&artifact, &gt).is_err());
    }

    #[test]
    fn collar_and_selective_speaker_metrics_have_known_answers() {
        let boundary = boundary_metrics(&[1_000, 2_000], &[1_100, 2_600], 250);
        assert_eq!(boundary.matched, 1);
        assert_eq!(boundary.precision, 0.5);
        assert_eq!(boundary.recall, 0.5);
        assert_eq!(boundary.false_split_rate, 0.5);
        assert_eq!(boundary.false_merge_rate, 0.5);
        let report = benchmark(&artifact(), &ground_truth()).unwrap();
        assert_eq!(report.v2.speaker_attribution.coverage, 1.0);
        assert_eq!(report.v2.speaker_attribution.assigned_only_accuracy, 1.0);
        assert_eq!(report.v2.speaker_attribution_accuracy, 1.0);
        assert_eq!(report.v2.utterance_count_error, 0);
        assert_eq!(report.parameter_sweep.len(), 144);
    }

    #[test]
    fn calibration_and_evaluation_identity_is_explicit() {
        let artifact = artifact();
        let mut calibration = ground_truth();
        calibration.dataset_split = DatasetSplit::Calibration;
        let mut evaluation = ground_truth();
        evaluation.dataset_split = DatasetSplit::Evaluation;
        assert_ne!(
            benchmark(&artifact, &calibration).unwrap().dataset_split,
            benchmark(&artifact, &evaluation).unwrap().dataset_split
        );
        assert_eq!(artifact.v1.algorithm_version, ALGORITHM_VERSION);
    }

    #[test]
    fn ground_truth_autosave_reopens_with_stable_speaker_mapping() {
        let artifact = artifact();
        let mut gt = ground_truth();
        gt.blind_complete = false;
        gt.review_complete = false;
        gt.qa_passed = false;
        let directory = std::env::temp_dir().join(format!(
            "huitrace-utterance-gt-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let artifact_path = directory.join("artifact.json");
        let gt_path = directory.join("gt.json");
        std::fs::write(&artifact_path, artifact.deterministic_json().unwrap()).unwrap();
        api_save_utterance_ground_truth(SaveGroundTruthRequest {
            artifact_path: artifact_path.clone(),
            ground_truth_path: gt_path.clone(),
            ground_truth: gt.clone(),
        })
        .unwrap();
        gt.annotation_version = "v2-after-autosave".into();
        api_save_utterance_ground_truth(SaveGroundTruthRequest {
            artifact_path,
            ground_truth_path: gt_path.clone(),
            ground_truth: gt.clone(),
        })
        .unwrap();
        let reopened: UtteranceGroundTruth =
            serde_json::from_slice(&std::fs::read(gt_path).unwrap()).unwrap();
        assert_eq!(reopened.annotation_version, "v2-after-autosave");
        assert_eq!(reopened.speakers, gt.speakers);
        assert_eq!(reopened.utterances, gt.utterances);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn manifest_binds_exact_artifact_audio_and_split() {
        let artifact = artifact();
        let gt = ground_truth();
        let row = manifest_row(&artifact, &gt).unwrap();
        assert_eq!(row.reconstruction_artifact_id, artifact.artifact_id);
        assert_eq!(row.artifact_sha256, artifact.integrity_sha256);
        assert_eq!(row.source_audio_sha256, artifact.source_media.sha256);
        assert_eq!(row.dataset_split, DatasetSplit::Evaluation);
    }

    #[test]
    fn calibration_and_evaluation_meetings_cannot_overlap() {
        let artifact = artifact();
        let mut calibration_gt = ground_truth();
        calibration_gt.dataset_split = DatasetSplit::Calibration;
        let calibration = manifest_row(&artifact, &calibration_gt).unwrap();
        let evaluation = manifest_row(&artifact, &ground_truth()).unwrap();
        assert!(validate_meeting_level_split(&[calibration], &[evaluation]).is_err());
    }

    #[test]
    fn pending_items_and_empty_completed_ground_truth_fail_closed() {
        let artifact = artifact();
        let mut pending = ground_truth();
        pending.utterances[0].status = GtAnnotationStatus::Pending;
        assert!(pending.validate(&artifact).is_err());

        let mut empty = ground_truth();
        empty.speakers.clear();
        empty.speaker_intervals.clear();
        empty.utterances.clear();
        assert!(empty.validate(&artifact).is_err());
    }

    #[test]
    fn duplicate_boundary_predictions_are_not_deduplicated() {
        let metrics = boundary_metrics(&[1_000, 1_000], &[1_000, 1_000], 0);
        assert_eq!(metrics.predicted, 2);
        assert_eq!(metrics.matched, 2);
        assert_eq!(metrics.f1, 1.0);
    }

    #[test]
    fn speaker_mapping_is_global_maximum_weight_not_greedy() {
        let reference = |id: &str, start_ms, end_ms, speaker: &str| GtSpeakerInterval {
            id: id.into(),
            start_ms,
            end_ms,
            gt_speaker_key: speaker.into(),
            overlap: false,
            uncertain: false,
            status: GtAnnotationStatus::Reviewed,
        };
        let turn = |start_ms, end_ms, speaker: &str| SpeakerTurn {
            start_ms,
            end_ms,
            speaker_label: speaker.into(),
            confidence: None,
            speaker_key: speaker.into(),
        };
        let mapping = align_speakers(
            &[
                reference("g1a", 0, 9, "gt_speaker_01"),
                reference("g1b", 20, 28, "gt_speaker_01"),
                reference("g2", 2, 9, "gt_speaker_02"),
            ],
            &[turn(0, 9, "A"), turn(20, 28, "B")],
        );
        assert_eq!(mapping.get("gt_speaker_01").map(String::as_str), Some("B"));
        assert_eq!(mapping.get("gt_speaker_02").map(String::as_str), Some("A"));
    }

    #[test]
    fn annotation_load_rejects_wrong_source_media() {
        let directory = tempfile::tempdir().unwrap();
        let artifact_path = directory.path().join("artifact.json");
        let gt_path = directory.path().join("gt.json");
        let expected_media = directory.path().join("expected.wav");
        let wrong_media = directory.path().join("wrong.wav");
        std::fs::write(&expected_media, b"expected media").unwrap();
        std::fs::write(&wrong_media, b"different media").unwrap();

        let mut artifact = artifact();
        artifact.source_media.sha256 = file_hash(&expected_media).unwrap();
        artifact = artifact.seal().unwrap();
        let mut gt = ground_truth();
        gt.reconstruction_artifact_sha256 = artifact.integrity_sha256.clone();
        gt.source_audio_sha256 = artifact.source_media.sha256.clone();
        std::fs::write(&artifact_path, artifact.deterministic_json().unwrap()).unwrap();
        std::fs::write(&gt_path, serde_json::to_vec(&gt).unwrap()).unwrap();

        let error = api_load_utterance_ground_truth(LoadGroundTruthRequest {
            artifact_path,
            ground_truth_path: gt_path,
            source_media_path: wrong_media,
            mode: AnnotationViewMode::Blind,
        })
        .unwrap_err();
        assert!(error.contains("source media SHA-256"));
    }
}
