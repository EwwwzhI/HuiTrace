//! Local-only working state for the Phase 2D.2 short-turn annotation workspace.
//!
//! This module intentionally owns *draft* annotations, rather than reusing the
//! production short-turn event table.  Production evidence is immutable input;
//! a canonical annotation event is created once on the source-meeting timeline
//! and overlapping export windows are only viewports onto that event.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Manager;

use super::dataset::{self, duration_bucket, GateSample};
use super::production_artifact::read_and_validate_artifact;

const DRAFT_SCHEMA_VERSION: u32 = 1;
const MIN_MEDIA_DURATION_TOLERANCE_MS: i64 = 2_000;
const MAX_MEDIA_DURATION_TOLERANCE_MS: i64 = 10_000;
const ALLOWED_KINDS: [&str; 5] = [
    "short_speech",
    "backchannel",
    "noise",
    "non_speech_vocalization",
    "ordinary_speech_control",
];

fn source_media_duration_tolerance_ms(artifact_duration_ms: i64) -> i64 {
    // Browser media elements report the container timeline, while production
    // artifacts use recording metadata / decoded evidence. Encoder delay and
    // trailing container padding grow with longer files, so a fixed 2s limit
    // produces false blockers. Keep a bounded relative tolerance while still
    // rejecting an accidentally selected source with a materially different run time.
    (artifact_duration_ms / 50).clamp(
        MIN_MEDIA_DURATION_TOLERANCE_MS,
        MAX_MEDIA_DURATION_TOLERANCE_MS,
    )
}

fn source_media_duration_matches(media_duration_ms: i64, artifact_duration_ms: i64) -> bool {
    (media_duration_ms - artifact_duration_ms).abs()
        <= source_media_duration_tolerance_ms(artifact_duration_ms)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationMode {
    Blind,
    Review,
    Qa,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnnotationEvent {
    pub event_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub kind: String,
    #[serde(default)]
    pub speaker: Option<String>,
    #[serde(default)]
    pub overlap: bool,
    #[serde(default)]
    pub speaker_handoff: bool,
    #[serde(default)]
    pub embedded: bool,
    #[serde(default)]
    pub annotation_uncertain: bool,
    #[serde(default)]
    pub expected_materialized: Option<bool>,
    #[serde(default)]
    pub notes: String,
    #[serde(default = "default_annotation_status")]
    pub annotation_status: String,
}

fn default_annotation_status() -> String {
    "blind_confirmed".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnnotationDraft {
    pub schema_version: u32,
    pub meeting_id: String,
    #[serde(default)]
    pub events: Vec<AnnotationEvent>,
}

impl AnnotationDraft {
    pub fn empty(meeting_id: String) -> Self {
        Self {
            schema_version: DRAFT_SCHEMA_VERSION,
            meeting_id,
            events: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeakerMapEntry {
    pub key: String,
    /// Local-only annotation aid.  This is deliberately never exported.
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnnotationSession {
    pub schema_version: u32,
    pub meeting_id: String,
    /// May be an absolute local path; it never appears in a manifest/artifact.
    #[serde(default)]
    pub source_media_path: String,
    /// Read from the local media element and used only to reject an accidental
    /// media/artifact mismatch. It is never exported to a manifest.
    #[serde(default)]
    pub source_media_duration_ms: Option<i64>,
    /// Kept local and converted to a relative path during manifest export.
    #[serde(default)]
    pub production_artifact_path: String,
    #[serde(default)]
    pub speaker_map: Vec<SpeakerMapEntry>,
    #[serde(default)]
    pub window_status: BTreeMap<String, String>,
    #[serde(default = "default_next_event_sequence")]
    pub next_event_sequence: u64,
    #[serde(default)]
    pub last_blind_window_id: Option<String>,
    #[serde(default)]
    pub last_review_window_id: Option<String>,
    #[serde(default)]
    pub artifact_identity: Option<ProductionArtifactIdentity>,
}

fn default_next_event_sequence() -> u64 {
    1
}

impl AnnotationSession {
    pub fn empty(meeting_id: String) -> Self {
        Self {
            schema_version: DRAFT_SCHEMA_VERSION,
            meeting_id,
            source_media_path: String::new(),
            source_media_duration_ms: None,
            production_artifact_path: String::new(),
            speaker_map: Vec::new(),
            window_status: BTreeMap::new(),
            next_event_sequence: 1,
            last_blind_window_id: None,
            last_review_window_id: None,
            artifact_identity: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProductionArtifactIdentity {
    pub artifact_id: String,
    pub sha256: String,
    pub schema_version: u32,
    pub transcription_run_id: String,
    pub relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationWindow {
    pub window_id: String,
    pub meeting_id: String,
    pub source_start_ms: i64,
    pub source_end_ms: i64,
    #[serde(default)]
    pub audio_path: String,
    #[serde(default)]
    pub production_artifact_path: String,
    #[serde(default)]
    pub candidate_suggestions: Vec<SystemSuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSuggestion {
    pub start_ms: i64,
    pub end_ms: i64,
    #[serde(default)]
    pub candidate_sources: Vec<String>,
    #[serde(default)]
    pub vad_confidence: Option<f64>,
    #[serde(default)]
    pub label: String,
}

/// Review-only evidence. This deliberately has a distinct type from
/// `AnnotationEvent`; no system evidence can be persisted as ground truth.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewEvidence {
    pub transcripts: Vec<ReviewTranscript>,
    pub diarizer_turns: Vec<ReviewDiarizerTurn>,
    pub vad_events: Vec<ReviewVadEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewTranscript {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReviewDiarizerTurn {
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker_key: String,
    pub overlap: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReviewVadEvent {
    pub start_ms: i64,
    pub end_ms: i64,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLoadRequest {
    pub dataset_dir: PathBuf,
    pub meeting_id: String,
    pub mode: AnnotationMode,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    pub draft: AnnotationDraft,
    pub session: AnnotationSession,
    pub windows: Vec<AnnotationWindow>,
    pub mode: AnnotationMode,
    /// `None` in blind mode by construction, not merely hidden by the client.
    pub review_evidence: Option<ReviewEvidence>,
    pub initialized: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnnotationProject {
    pub schema_version: u32,
    pub meeting_id: String,
    pub source_media: String,
    pub production_artifact: String,
    pub annotation_draft: &'static str,
    pub blind_windows: &'static str,
    pub review_windows: &'static str,
    pub blind_completed: bool,
    pub review_completed: bool,
    pub production_artifact_identity: ProductionArtifactIdentity,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeProjectRequest {
    pub dataset_dir: PathBuf,
    pub meeting_id: String,
    pub source_media_path: PathBuf,
    pub production_artifact_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSaveRequest {
    pub dataset_dir: PathBuf,
    pub draft: AnnotationDraft,
    pub session: AnnotationSession,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalQaReport {
    pub errors: Vec<String>,
    pub possible_duplicates: Vec<dataset::DuplicateCandidate>,
    pub source_duration_ms: Option<i64>,
}

pub(super) fn meeting_dir(root: &Path, meeting_id: &str) -> Result<PathBuf> {
    if meeting_id.trim().is_empty() || meeting_id.contains(['/', '\\']) || meeting_id.contains("..")
    {
        bail!("meeting_id must be a simple local meeting identifier");
    }
    Ok(root.join(meeting_id))
}

fn read_json_or<T: for<'a> Deserialize<'a>>(path: &Path, default: T) -> Result<T> {
    if !path.exists() {
        return Ok(default);
    }
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let parent = path
        .parent()
        .context("annotation workspace path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let temporary = parent.join(format!(
        ".{}.tmp.{}.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("annotation"),
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&temporary, bytes).with_context(|| format!("write {}", temporary.display()))?;
    fs::rename(&temporary, path)
        .with_context(|| format!("atomically replace {}", path.display()))?;
    Ok(())
}

fn expected_window_ids(windows: &[AnnotationWindow]) -> BTreeSet<&str> {
    windows
        .iter()
        .map(|window| window.window_id.as_str())
        .collect()
}

fn all_blind_windows_complete(
    windows: &[AnnotationWindow],
    statuses: &BTreeMap<String, String>,
) -> bool {
    !windows.is_empty()
        && expected_window_ids(windows).into_iter().all(|id| {
            matches!(
                statuses.get(id).map(String::as_str),
                Some("reviewed_blind" | "reviewed_second_pass")
            )
        })
}

fn all_review_windows_complete(
    windows: &[AnnotationWindow],
    statuses: &BTreeMap<String, String>,
) -> bool {
    !windows.is_empty()
        && expected_window_ids(windows)
            .into_iter()
            .all(|id| statuses.get(id).map(String::as_str) == Some("reviewed_second_pass"))
}

fn events_overlapping_window<'a>(
    draft: &'a AnnotationDraft,
    window: &'a AnnotationWindow,
) -> impl Iterator<Item = &'a AnnotationEvent> {
    draft.events.iter().filter(move |event| {
        event.start_ms < window.source_end_ms && event.end_ms > window.source_start_ms
    })
}

fn validate_window_completion(
    draft: &AnnotationDraft,
    window: &AnnotationWindow,
    review: bool,
) -> Result<()> {
    let count = events_overlapping_window(draft, window)
        .filter(|event| {
            event.annotation_status == "pending"
                || (review && event.annotation_status == "review_pending")
        })
        .count();
    if count > 0 {
        bail!("Window {} contains {} unconfirmed annotations; confirm all event labels before completing it", window.window_id, count);
    }
    Ok(())
}

fn validate_blind_entry(draft: &AnnotationDraft) -> Result<()> {
    let count = draft
        .events
        .iter()
        .filter(|event| event.annotation_status == "pending")
        .count();
    if count > 0 {
        bail!("Blind pass contains {count} unconfirmed annotations. Confirm them before opening Review.");
    }
    Ok(())
}

fn require_complete(
    windows: &[AnnotationWindow],
    session: &AnnotationSession,
    review: bool,
) -> Result<()> {
    let complete = if review {
        all_review_windows_complete(windows, &session.window_status)
    } else {
        all_blind_windows_complete(windows, &session.window_status)
    };
    if !complete {
        let total = expected_window_ids(windows).len();
        let completed = windows
            .iter()
            .filter(|window| {
                let status = session
                    .window_status
                    .get(&window.window_id)
                    .map(String::as_str);
                status == Some("reviewed_second_pass")
                    || (!review && status == Some("reviewed_blind"))
            })
            .map(|window| &window.window_id)
            .collect::<BTreeSet<_>>()
            .len();
        let pass = if review { "Review" } else { "Blind" };
        bail!(
            "{pass} annotation is incomplete: {} / {total} windows remain (completed: {completed})",
            total - completed
        );
    }
    Ok(())
}

fn read_windows(path: &Path, mode: &AnnotationMode) -> Result<Vec<AnnotationWindow>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let reader = BufReader::new(fs::File::open(path)?);
    reader
        .lines()
        .enumerate()
        .filter_map(|(index, line)| match line {
            Ok(line) if line.trim().is_empty() => None,
            Ok(line) => Some(
                serde_json::from_str::<AnnotationWindow>(&line)
                    .with_context(|| format!("parse {} line {}", path.display(), index + 1)),
            ),
            Err(error) => Some(Err(error.into())),
        })
        .map(|window| {
            window.map(|mut value| {
                // This is the blind integrity boundary: no system-derived field is
                // deserialized into the returned blind viewport.
                if *mode == AnnotationMode::Blind {
                    value.candidate_suggestions.clear();
                }
                value
            })
        })
        .collect()
}

fn configured_path(dataset_dir: &Path, configured: &str) -> PathBuf {
    let path = PathBuf::from(configured);
    if path.is_absolute() {
        path
    } else {
        dataset_dir.join(path)
    }
}

fn validate_artifact_identity(dataset_dir: &Path, session: &AnnotationSession) -> Result<PathBuf> {
    let identity = session
        .artifact_identity
        .as_ref()
        .context("annotation project has no production artifact identity")?;
    if session.production_artifact_path != identity.relative_path {
        bail!("session production artifact path differs from the bound artifact identity");
    }
    let path = configured_path(dataset_dir, &identity.relative_path);
    let sha = artifact_sha256(&path)
        .with_context(|| format!("hash bound artifact {}", path.display()))?;
    if sha != identity.sha256 {
        bail!("bound production artifact changed (SHA-256 mismatch)");
    }
    let artifact = read_and_validate_artifact(&path, Some(&session.meeting_id))?;
    if artifact.artifact_id != identity.artifact_id
        || artifact.schema_version != identity.schema_version
        || artifact.transcription_run_id != identity.transcription_run_id
    {
        bail!("bound production artifact identity changed");
    }
    Ok(path)
}

fn validate_window_provenance(
    windows: &[AnnotationWindow],
    meeting_id: &str,
    artifact_path: &Path,
) -> Result<()> {
    let expected = artifact_path.canonicalize()?;
    let mut ids = BTreeSet::new();
    for window in windows {
        if window.meeting_id != meeting_id {
            bail!("window {} belongs to a different meeting", window.window_id);
        }
        if window.window_id.trim().is_empty() || !ids.insert(window.window_id.as_str()) {
            bail!("window ids must be nonempty and unique");
        }
        if window.source_start_ms < 0 || window.source_end_ms <= window.source_start_ms {
            bail!("window {} has invalid timing", window.window_id);
        }
        if window.production_artifact_path.trim().is_empty() {
            bail!(
                "window {} has no production_artifact_path",
                window.window_id
            );
        }
        let actual = PathBuf::from(&window.production_artifact_path)
            .canonicalize()
            .with_context(|| format!("open window artifact for {}", window.window_id))?;
        if actual != expected {
            bail!(
                "window {} was exported from a different production artifact",
                window.window_id
            );
        }
    }
    Ok(())
}

pub fn initialize_annotation_project_inner(
    request: InitializeProjectRequest,
    controlled_media_root: &Path,
) -> Result<WorkspaceSnapshot> {
    let directory = meeting_dir(&request.dataset_dir, &request.meeting_id)?;
    if !request.dataset_dir.is_dir() {
        bail!("dataset directory does not exist");
    }
    if !request.source_media_path.is_file() {
        bail!("source media does not exist");
    }
    let extension = request
        .source_media_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "wav" | "mp3" | "m4a" | "mp4" | "webm") {
        bail!("unsupported source media type");
    }
    let artifact_path = request
        .production_artifact_path
        .canonicalize()
        .context("open production artifact")?;
    let artifact = read_and_validate_artifact(&artifact_path, Some(&request.meeting_id))?;
    let blind = read_windows(
        &directory.join("annotation_windows.blind.jsonl"),
        &AnnotationMode::Blind,
    )?;
    let review = read_windows(
        &directory.join("annotation_windows.review.jsonl"),
        &AnnotationMode::Review,
    )?;
    if blind.is_empty() || review.is_empty() {
        bail!("blind and review window exports are required");
    }
    validate_window_provenance(&blind, &request.meeting_id, &artifact_path)?;
    validate_window_provenance(&review, &request.meeting_id, &artifact_path)?;
    let relative = artifact_path
        .strip_prefix(request.dataset_dir.canonicalize()?)
        .map_err(|_| {
            anyhow::anyhow!("production artifact must live below the dataset directory")
        })?;
    let identity = ProductionArtifactIdentity {
        artifact_id: artifact.artifact_id,
        sha256: artifact_sha256(&artifact_path)?,
        schema_version: artifact.schema_version,
        transcription_run_id: artifact.transcription_run_id,
        relative_path: relative.to_string_lossy().replace('\\', "/"),
    };
    let media_directory = controlled_media_root
        .join("mityu-recordings")
        .join("huitrace-annotation")
        .join(&request.meeting_id);
    fs::create_dir_all(&media_directory)?;
    let media_path = media_directory.join(format!("source.{extension}"));
    fs::copy(&request.source_media_path, &media_path)
        .with_context(|| format!("copy source media to {}", media_path.display()))?;
    let mut session = AnnotationSession::empty(request.meeting_id.clone());
    session.source_media_path = media_path.to_string_lossy().into_owned();
    session.production_artifact_path = identity.relative_path.clone();
    session.artifact_identity = Some(identity.clone());
    let draft = AnnotationDraft::empty(request.meeting_id.clone());
    atomic_write(
        &directory.join("annotation_session.json"),
        &serde_json::to_vec_pretty(&session)?,
    )?;
    atomic_write(
        &directory.join("annotations.draft.json"),
        &serde_json::to_vec_pretty(&draft)?,
    )?;
    let project = AnnotationProject {
        schema_version: DRAFT_SCHEMA_VERSION,
        meeting_id: request.meeting_id.clone(),
        source_media: session.source_media_path.clone(),
        production_artifact: identity.relative_path.clone(),
        annotation_draft: "annotations.draft.json",
        blind_windows: "annotation_windows.blind.jsonl",
        review_windows: "annotation_windows.review.jsonl",
        blind_completed: false,
        review_completed: false,
        production_artifact_identity: identity,
    };
    atomic_write(
        &directory.join("annotation_project.json"),
        &serde_json::to_vec_pretty(&project)?,
    )?;
    load_workspace_inner(WorkspaceLoadRequest {
        dataset_dir: request.dataset_dir,
        meeting_id: request.meeting_id,
        mode: AnnotationMode::Blind,
    })
}

fn load_review_evidence(
    dataset_dir: &Path,
    session: &AnnotationSession,
    meeting_id: &str,
) -> Result<ReviewEvidence> {
    if session.production_artifact_path.trim().is_empty() {
        bail!("a production artifact is required before review mode");
    }
    let artifact = read_and_validate_artifact(
        &configured_path(dataset_dir, &session.production_artifact_path),
        Some(meeting_id),
    )?;
    Ok(ReviewEvidence {
        transcripts: artifact
            .transcripts
            .into_iter()
            .map(|row| ReviewTranscript {
                start_ms: row.start_ms,
                end_ms: row.end_ms,
                text: row.text,
            })
            .collect(),
        diarizer_turns: artifact
            .raw_diarizer_turns
            .into_iter()
            .map(|row| ReviewDiarizerTurn {
                start_ms: row.start_ms,
                end_ms: row.end_ms,
                speaker_key: row.speaker_key,
                overlap: row.overlap,
            })
            .collect(),
        vad_events: artifact
            .vad_events
            .into_iter()
            .map(|row| ReviewVadEvent {
                start_ms: row.start_ms,
                end_ms: row.end_ms,
                confidence: row.confidence,
            })
            .collect(),
    })
}

pub fn load_workspace_inner(request: WorkspaceLoadRequest) -> Result<WorkspaceSnapshot> {
    let directory = meeting_dir(&request.dataset_dir, &request.meeting_id)?;
    let initialized = directory.join("annotation_project.json").is_file()
        && directory.join("annotation_session.json").is_file()
        && directory.join("annotations.draft.json").is_file();
    let draft = read_json_or(
        &directory.join("annotations.draft.json"),
        AnnotationDraft::empty(request.meeting_id.clone()),
    )?;
    let mut session = read_json_or(
        &directory.join("annotation_session.json"),
        AnnotationSession::empty(request.meeting_id.clone()),
    )?;
    if draft.meeting_id != request.meeting_id || session.meeting_id != request.meeting_id {
        bail!("annotation draft/session belongs to a different meeting");
    }
    let next_from_events = draft
        .events
        .iter()
        .filter_map(|event| event.event_id.rsplit('-').next()?.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    session.next_event_sequence = session.next_event_sequence.max(next_from_events);
    let filename = match request.mode {
        AnnotationMode::Blind => "annotation_windows.blind.jsonl",
        AnnotationMode::Review | AnnotationMode::Qa => "annotation_windows.review.jsonl",
    };
    let windows = read_windows(&directory.join(filename), &request.mode)?;
    if matches!(request.mode, AnnotationMode::Review | AnnotationMode::Qa) && !initialized {
        bail!("initialize the annotation project before opening review or QA");
    }
    if matches!(request.mode, AnnotationMode::Review | AnnotationMode::Qa) {
        validate_blind_entry(&draft)?;
        let blind_windows = read_windows(
            &directory.join("annotation_windows.blind.jsonl"),
            &AnnotationMode::Blind,
        )?;
        if !all_blind_windows_complete(&blind_windows, &session.window_status) {
            bail!("complete every blind window before opening review mode");
        }
    }
    if initialized {
        validate_artifact_identity(&request.dataset_dir, &session)?;
    }
    let review_evidence = match request.mode {
        AnnotationMode::Blind => None,
        AnnotationMode::Review | AnnotationMode::Qa => Some(load_review_evidence(
            &request.dataset_dir,
            &session,
            &request.meeting_id,
        )?),
    };
    Ok(WorkspaceSnapshot {
        draft,
        session,
        windows,
        mode: request.mode,
        review_evidence,
        initialized,
    })
}

fn validate_draft_shape(draft: &AnnotationDraft, session: &AnnotationSession) -> Result<()> {
    if draft.schema_version != DRAFT_SCHEMA_VERSION
        || session.schema_version != DRAFT_SCHEMA_VERSION
    {
        bail!("unsupported annotation workspace schema version");
    }
    if draft.meeting_id != session.meeting_id {
        bail!("draft and session meeting_id differ");
    }
    let mut ids = BTreeSet::new();
    let mut speaker_keys = BTreeSet::new();
    for speaker in &session.speaker_map {
        if speaker.key.trim().is_empty() || !speaker_keys.insert(speaker.key.as_str()) {
            bail!("speaker map keys must be nonempty and unique");
        }
        if !speaker.key.starts_with("gt_speaker_") {
            bail!("ground-truth speaker keys must use the gt_speaker_ namespace");
        }
    }
    for event in &draft.events {
        if event.event_id.trim().is_empty() || !ids.insert(event.event_id.as_str()) {
            bail!("event ids must be nonempty and unique");
        }
        if event.start_ms < 0 || event.end_ms <= event.start_ms {
            bail!("{} has invalid timing", event.event_id);
        }
        if !ALLOWED_KINDS.contains(&event.kind.as_str()) {
            bail!("{} has invalid annotation kind", event.event_id);
        }
        if let Some(speaker) = &event.speaker {
            if speaker.trim().is_empty() {
                bail!("{} has an empty speaker key", event.event_id);
            }
            if !speaker_keys.contains(speaker.as_str()) {
                bail!(
                    "{} references speaker not present in meeting-local speaker map",
                    event.event_id
                );
            }
        }
    }
    Ok(())
}

pub fn save_workspace_inner(request: WorkspaceSaveRequest) -> Result<()> {
    validate_draft_shape(&request.draft, &request.session)?;
    let directory = meeting_dir(&request.dataset_dir, &request.draft.meeting_id)?;
    if !directory.join("annotation_project.json").is_file() {
        bail!("annotation project is not initialized");
    }
    validate_artifact_identity(&request.dataset_dir, &request.session)?;
    let blind_windows = read_windows(
        &directory.join("annotation_windows.blind.jsonl"),
        &AnnotationMode::Blind,
    )?;
    let review_windows = read_windows(
        &directory.join("annotation_windows.review.jsonl"),
        &AnnotationMode::Review,
    )?;
    // Autosave is the backend authority for completion; validate before any write.
    for window in &blind_windows {
        if matches!(
            request
                .session
                .window_status
                .get(&window.window_id)
                .map(String::as_str),
            Some("reviewed_blind" | "reviewed_second_pass")
        ) {
            validate_window_completion(&request.draft, window, false)?;
        }
    }
    for window in &review_windows {
        if request
            .session
            .window_status
            .get(&window.window_id)
            .map(String::as_str)
            == Some("reviewed_second_pass")
        {
            validate_window_completion(&request.draft, window, true)?;
        }
    }
    atomic_write(
        &directory.join("annotations.draft.json"),
        &serde_json::to_vec_pretty(&request.draft)?,
    )?;
    atomic_write(
        &directory.join("annotation_session.json"),
        &serde_json::to_vec_pretty(&request.session)?,
    )?;
    let identity = request
        .session
        .artifact_identity
        .clone()
        .context("missing artifact identity")?;
    let project = AnnotationProject {
        schema_version: DRAFT_SCHEMA_VERSION,
        meeting_id: request.draft.meeting_id.clone(),
        source_media: request.session.source_media_path.clone(),
        production_artifact: identity.relative_path.clone(),
        annotation_draft: "annotations.draft.json",
        blind_windows: "annotation_windows.blind.jsonl",
        review_windows: "annotation_windows.review.jsonl",
        blind_completed: all_blind_windows_complete(&blind_windows, &request.session.window_status),
        review_completed: all_review_windows_complete(
            &review_windows,
            &request.session.window_status,
        ),
        production_artifact_identity: identity,
    };
    atomic_write(
        &directory.join("annotation_project.json"),
        &serde_json::to_vec_pretty(&project)?,
    )
}

impl GateSample for AnnotationEvent {
    fn event_id(&self) -> &str {
        &self.event_id
    }
    fn meeting_id(&self) -> &str {
        "workspace-event"
    }
    fn start_ms(&self) -> i64 {
        self.start_ms
    }
    fn end_ms(&self) -> i64 {
        self.end_ms
    }
    fn kind_label(&self) -> &'static str {
        dataset::GroundTruthKind::from_label(&self.kind)
            .map(|kind| kind.as_label())
            .unwrap_or("invalid")
    }

    fn ground_truth_speaker(&self) -> Option<&str> {
        self.speaker.as_deref()
    }
    fn expected_visible_speakers(&self) -> &[String] {
        &[]
    }
    fn annotation_uncertain(&self) -> bool {
        self.annotation_uncertain
    }
    fn tags(&self) -> &[String] {
        &[]
    }
}

fn event_tags(event: &AnnotationEvent) -> Vec<String> {
    let mut tags = Vec::new();
    if event.overlap {
        tags.push("overlap".into());
    }
    if event.speaker_handoff {
        tags.push("speaker_handoff".into());
    }
    tags
}

fn local_duplicates(
    events: &[AnnotationEvent],
    meeting_id: &str,
) -> Vec<dataset::DuplicateCandidate> {
    let rows = events.iter().map(|event| serde_json::json!({
        "ground_truth_event_id": event.event_id, "record_type": "ground_truth_event", "recall_eligible": true,
        "evidence_origin": "production_artifact", "meeting_id": meeting_id, "production_artifact_path": "placeholder.json",
        "start_ms": event.start_ms, "end_ms": event.end_ms, "duration_bucket": duration_bucket(event.end_ms-event.start_ms),
        "ground_truth_kind": event.kind, "ground_truth_speaker": event.speaker, "annotation_uncertain": event.annotation_uncertain,
        "expected_visible_speakers": [], "tags": event_tags(event)
    })).map(serde_json::from_value::<dataset::DatasetRecord>).collect::<Result<Vec<_>, _>>().unwrap_or_default();
    dataset::possible_duplicates(&rows)
}

pub fn qa_workspace(
    dataset_dir: &Path,
    draft: &AnnotationDraft,
    session: &AnnotationSession,
) -> Result<LocalQaReport> {
    let mut errors = Vec::new();
    if let Err(error) = validate_draft_shape(draft, session) {
        errors.push(error.to_string());
    }
    if let Err(error) = validate_artifact_identity(dataset_dir, session) {
        errors.push(format!("production artifact identity: {error:#}"));
    }
    let mut source_duration_ms = None;
    if !session.production_artifact_path.trim().is_empty() {
        let path = configured_path(dataset_dir, &session.production_artifact_path);
        match read_and_validate_artifact(&path, Some(&draft.meeting_id)) {
            Ok(artifact) => {
                source_duration_ms = Some(artifact.source_audio.duration_ms);
                if let Some(media_duration) = session.source_media_duration_ms {
                    let artifact_duration = artifact.source_audio.duration_ms;
                    if !source_media_duration_matches(media_duration, artifact_duration) {
                        let delta = (media_duration - artifact_duration).abs();
                        let tolerance = source_media_duration_tolerance_ms(artifact_duration);
                        errors.push(format!(
                            "source media duration {media_duration}ms does not match artifact duration {artifact_duration}ms (difference {delta}ms exceeds {tolerance}ms tolerance)"
                        ));
                    }
                }
            }
            Err(error) => errors.push(format!("production artifact: {error:#}")),
        }
    } else {
        errors.push("production artifact path is required".into());
    }
    for event in &draft.events {
        if let Some(duration) = source_duration_ms {
            if event.end_ms > duration {
                errors.push(format!("{} exceeds source duration", event.event_id));
            }
        }
        if matches!(event.kind.as_str(), "short_speech" | "backchannel")
            && !event.annotation_uncertain
            && event.speaker.is_none()
        {
            errors.push(format!(
                "{} requires a speaker unless uncertain",
                event.event_id
            ));
        }
        if event.kind == "noise" && event.speaker.is_some() {
            errors.push(format!(
                "{} noise should not have a speaker",
                event.event_id
            ));
        }
        if event.kind == "ordinary_speech_control" && event.end_ms - event.start_ms <= 1_200 {
            errors.push(format!(
                "{} ordinary_speech_control must be longer than 1200 ms",
                event.event_id
            ));
        }
        if !matches!(
            event.annotation_status.as_str(),
            "blind_confirmed" | "reviewed"
        ) {
            errors.push(format!(
                "{} requires explicit annotation confirmation",
                event.event_id
            ));
        }
        if let Some(speaker) = &event.speaker {
            if !session
                .speaker_map
                .iter()
                .any(|entry| entry.key == *speaker)
            {
                errors.push(format!(
                    "{} references speaker not present in meeting-local speaker map",
                    event.event_id
                ));
            }
        }
    }
    let _ = dataset_dir; // Dataset-wide coverage is intentionally only read in QA/review, never blind mode.
    Ok(LocalQaReport {
        errors,
        possible_duplicates: local_duplicates(&draft.events, &draft.meeting_id),
        source_duration_ms,
    })
}

fn relative_artifact_path(root: &Path, raw: &str) -> Result<PathBuf> {
    let root = root
        .canonicalize()
        .with_context(|| format!("open dataset directory {}", root.display()))?;
    let configured = PathBuf::from(raw);
    let artifact = (if configured.is_absolute() {
        configured
    } else {
        root.join(configured)
    })
    .canonicalize()
    .with_context(|| "open production artifact")?;
    artifact.strip_prefix(&root).map(PathBuf::from).map_err(|_| anyhow::anyhow!("production artifact must live below the local dataset directory; manifests never contain absolute paths"))
}

fn artifact_sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8_192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn manifest_line(
    event: &AnnotationEvent,
    meeting_id: &str,
    artifact: &Path,
    artifact_sha: &str,
    visible: &[String],
) -> serde_json::Value {
    serde_json::json!({
        "ground_truth_event_id": event.event_id,
        "record_type": "ground_truth_event",
        "recall_eligible": true,
        "evidence_origin": "production_artifact",
        "meeting_id": meeting_id,
        "production_artifact_path": artifact,
        "production_artifact_sha256": artifact_sha,
        "start_ms": event.start_ms,
        "end_ms": event.end_ms,
        "duration_bucket": duration_bucket(event.end_ms-event.start_ms),
        "ground_truth_kind": event.kind,
        "ground_truth_speaker": event.speaker,
        "annotation_uncertain": event.annotation_uncertain,
        "expected_materialized": event.expected_materialized,
        "embedded": event.embedded,
        "expected_visible_speakers": visible,
        "tags": event_tags(event),
        "notes": event.notes,
    })
}

fn write_jsonl(path: &Path, rows: &[serde_json::Value]) -> Result<()> {
    let mut bytes = Vec::new();
    for row in rows {
        writeln!(&mut bytes, "{}", serde_json::to_string(row)?)?;
    }
    atomic_write(path, &bytes)
}

pub fn export_manifest(
    dataset_dir: &Path,
    draft: &AnnotationDraft,
    session: &AnnotationSession,
) -> Result<dataset::DatasetCheckReport> {
    let directory = meeting_dir(dataset_dir, &draft.meeting_id)?;
    let blind = read_windows(
        &directory.join("annotation_windows.blind.jsonl"),
        &AnnotationMode::Blind,
    )?;
    let review = read_windows(
        &directory.join("annotation_windows.review.jsonl"),
        &AnnotationMode::Review,
    )?;
    require_complete(&blind, session, false)?;
    require_complete(&review, session, true)?;
    let qa = qa_workspace(dataset_dir, draft, session)?;
    if !qa.errors.is_empty() {
        bail!("QA must pass before export: {}", qa.errors.join("; "));
    }
    validate_artifact_identity(dataset_dir, session)?;
    let saved_draft: AnnotationDraft =
        serde_json::from_slice(&fs::read(directory.join("annotations.draft.json"))?)?;
    let saved_session: AnnotationSession =
        serde_json::from_slice(&fs::read(directory.join("annotation_session.json"))?)?;
    if &saved_draft != draft || &saved_session != session {
        bail!(
            "Save the current draft and completion state before exporting the Benchmark Manifest"
        );
    }
    let artifact = relative_artifact_path(dataset_dir, &session.production_artifact_path)?;
    let artifact_sha = artifact_sha256(&dataset_dir.join(&artifact))?;
    let visible_speakers = session
        .speaker_map
        .iter()
        .map(|entry| entry.key.clone())
        .collect::<Vec<_>>();
    let meeting_directory = meeting_dir(dataset_dir, &draft.meeting_id)?;
    let own_rows = draft
        .events
        .iter()
        .map(|event| {
            manifest_line(
                event,
                &draft.meeting_id,
                &artifact,
                &artifact_sha,
                &visible_speakers,
            )
        })
        .collect::<Vec<_>>();

    // The benchmark consumes a root manifest. Preserve exported rows for other
    // meetings and replace only this meeting's canonical event collection.
    let root_manifest = dataset_dir.join("manifest.jsonl");
    let mut rows = if root_manifest.exists() {
        let mut parsed = Vec::new();
        for (index, line) in BufReader::new(fs::File::open(&root_manifest)?)
            .lines()
            .enumerate()
        {
            let line = line
                .with_context(|| format!("read {} line {}", root_manifest.display(), index + 1))?;
            if !line.trim().is_empty() {
                parsed.push(
                    serde_json::from_str::<serde_json::Value>(&line).with_context(|| {
                        format!("parse {} line {}", root_manifest.display(), index + 1)
                    })?,
                );
            }
        }
        parsed
    } else {
        Vec::new()
    };
    rows.retain(|row| {
        row.get("meeting_id").and_then(|value| value.as_str()) != Some(draft.meeting_id.as_str())
    });
    rows.extend(own_rows.clone());
    let records = rows
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<std::result::Result<Vec<dataset::DatasetRecord>, _>>()?;
    let report = dataset::check_dataset_records(dataset_dir, &records)?;
    let meeting_manifest = meeting_directory.join("manifest.jsonl");
    let previous = if meeting_manifest.exists() {
        Some(fs::read(&meeting_manifest)?)
    } else {
        None
    };
    write_jsonl(&meeting_manifest, &own_rows)?;
    if let Err(error) = write_jsonl(&root_manifest, &rows) {
        match previous {
            Some(bytes) => atomic_write(&meeting_manifest, &bytes)
                .context("restore meeting manifest after root write failure")?,
            None => fs::remove_file(&meeting_manifest)
                .context("remove meeting manifest after root write failure")?,
        }
        return Err(error);
    }
    Ok(report)
}

/// Tauri boundary for local QA.  No network, inference, or production-state
/// mutation is reachable from this command.
#[tauri::command]
pub fn qa_workspace_command(
    dataset_dir: PathBuf,
    draft: AnnotationDraft,
    session: AnnotationSession,
) -> Result<LocalQaReport, String> {
    qa_workspace(&dataset_dir, &draft, &session)
        .map_err(|error| format!("annotation QA: {error:#}"))
}

/// Tauri boundary for the canonical draft → existing benchmark manifest mapping.
#[tauri::command]
pub fn export_manifest_command(
    dataset_dir: PathBuf,
    draft: AnnotationDraft,
    session: AnnotationSession,
) -> Result<dataset::DatasetCheckReport, String> {
    export_manifest(&dataset_dir, &draft, &session)
        .map_err(|error| format!("annotation manifest export: {error:#}"))
}

#[tauri::command]
pub fn load_workspace(request: WorkspaceLoadRequest) -> Result<WorkspaceSnapshot, String> {
    load_workspace_inner(request).map_err(|error| format!("load annotation workspace: {error:#}"))
}

#[tauri::command]
pub fn save_workspace(request: WorkspaceSaveRequest) -> Result<(), String> {
    save_workspace_inner(request).map_err(|error| format!("save annotation workspace: {error:#}"))
}

#[tauri::command]
pub fn initialize_annotation_project(
    app: tauri::AppHandle,
    request: InitializeProjectRequest,
) -> Result<WorkspaceSnapshot, String> {
    let audio_dir = app
        .path()
        .audio_dir()
        .map_err(|error| format!("resolve controlled audio directory: {error}"))?;
    initialize_annotation_project_inner(request, &audio_dir)
        .map_err(|error| format!("initialize annotation project: {error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::production_artifact::{
        write_artifact, ArtifactBackend, ArtifactDiarizerTurn, ArtifactTranscript,
        ArtifactVadEvent, MeetingProductionArtifact, ProductionConfigSnapshot,
        ProductionSafetyObservations, SourceAudioMetadata, ARTIFACT_SCHEMA_VERSION,
        PHASE_2C1_FROZEN_BASELINE_COMMIT,
    };

    fn event(id: &str, start: i64, end: i64, speaker: Option<&str>) -> AnnotationEvent {
        AnnotationEvent {
            event_id: id.into(),
            start_ms: start,
            end_ms: end,
            kind: "backchannel".into(),
            speaker: speaker.map(str::to_string),
            overlap: false,
            speaker_handoff: false,
            embedded: false,
            annotation_uncertain: false,
            expected_materialized: None,
            notes: String::new(),
            annotation_status: "blind_confirmed".into(),
        }
    }

    #[test]
    fn event_identity_is_canonical_across_overlapping_viewports() {
        let item = event("meeting-event-0021", 4_610, 4_890, Some("speaker_02"));
        assert!(item.start_ms < 5_000 && item.end_ms > 4_000);
        let draft = AnnotationDraft {
            schema_version: 1,
            meeting_id: "meeting".into(),
            events: vec![item],
        };
        assert_eq!(draft.events.len(), 1);
        assert_eq!(draft.events[0].event_id, "meeting-event-0021");
    }

    #[test]
    fn editing_bounds_keeps_event_id_and_deleting_removes_canonical_event() {
        let mut draft = AnnotationDraft {
            schema_version: 1,
            meeting_id: "meeting".into(),
            events: vec![event("meeting-event-0001", 100, 280, Some("speaker_01"))],
        };
        draft.events[0].end_ms = 310;
        assert_eq!(draft.events[0].event_id, "meeting-event-0001");
        draft.events.clear();
        assert!(draft.events.is_empty());
    }

    #[test]
    fn blind_windows_never_return_suggestions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("windows.jsonl");
        fs::write(&path, r#"{"window_id":"w","meeting_id":"m","source_start_ms":0,"source_end_ms":5,"candidate_suggestions":[{"start_ms":1,"end_ms":2}]}"#).unwrap();
        assert!(read_windows(&path, &AnnotationMode::Blind).unwrap()[0]
            .candidate_suggestions
            .is_empty());
        assert_eq!(
            read_windows(&path, &AnnotationMode::Review).unwrap()[0]
                .candidate_suggestions
                .len(),
            1
        );
    }

    #[test]
    fn duration_bucket_and_manifest_privacy_are_automatic() {
        let row = manifest_line(
            &event("id", 100, 399, Some("speaker_01")),
            "m",
            Path::new("m/m.production.json"),
            "a".repeat(64).as_str(),
            &["speaker_01".into()],
        );
        assert_eq!(row["duration_bucket"], "100-300ms");
        assert_eq!(row["tags"], serde_json::json!([]));
        assert!(row.get("speaker_description").is_none());
        assert_eq!(row["production_artifact_sha256"], "a".repeat(64));
        assert_eq!(row["expected_materialized"], serde_json::Value::Null);
    }

    #[test]
    fn media_duration_check_allows_bounded_container_timing_drift() {
        let artifact_duration = 210_597;
        assert_eq!(source_media_duration_tolerance_ms(artifact_duration), 4_211);
        assert!(source_media_duration_matches(214_221, artifact_duration));
        assert!(!source_media_duration_matches(220_000, artifact_duration));
        assert_eq!(source_media_duration_tolerance_ms(30_000), 2_000);
        assert_eq!(source_media_duration_tolerance_ms(900_000), 10_000);
    }

    fn window(id: &str) -> AnnotationWindow {
        AnnotationWindow {
            window_id: id.into(),
            meeting_id: "meeting".into(),
            source_start_ms: 0,
            source_end_ms: 1_000,
            audio_path: String::new(),
            production_artifact_path: String::new(),
            candidate_suggestions: Vec::new(),
        }
    }

    #[test]
    fn expected_materialized_preserves_auto_yes_and_no() {
        let mut item = event("id", 0, 100, None);
        assert_eq!(
            serde_json::to_value(&item).unwrap()["expected_materialized"],
            serde_json::Value::Null
        );
        item.expected_materialized = Some(true);
        assert_eq!(
            serde_json::to_value(&item).unwrap()["expected_materialized"],
            true
        );
        item.expected_materialized = Some(false);
        assert_eq!(
            serde_json::to_value(&item).unwrap()["expected_materialized"],
            false
        );
    }

    #[test]
    fn completion_uses_every_expected_window_id() {
        let windows = vec![window("w1"), window("w2"), window("w3")];
        let mut statuses = BTreeMap::from([("w1".into(), "reviewed_blind".into())]);
        assert!(!all_blind_windows_complete(&windows, &statuses));
        statuses.insert("w2".into(), "reviewed_second_pass".into());
        statuses.insert("w3".into(), "reviewed_blind".into());
        assert!(all_blind_windows_complete(&windows, &statuses));
        assert!(!all_review_windows_complete(&windows, &statuses));
        statuses.insert("w1".into(), "reviewed_second_pass".into());
        statuses.insert("w3".into(), "reviewed_second_pass".into());
        assert!(all_review_windows_complete(&windows, &statuses));
    }

    #[test]
    fn completion_rejects_pending_across_overlapping_windows_only() {
        let mut a = window("a");
        a.source_end_ms = 5_000;
        let mut b = window("b");
        b.source_start_ms = 4_000;
        b.source_end_ms = 9_000;
        let mut c = window("c");
        c.source_start_ms = 4_800;
        c.source_end_ms = 9_000;
        let mut draft = AnnotationDraft::empty("meeting".into());
        draft.events.push(event("e", 4_500, 4_800, None));
        draft.events[0].annotation_status = "pending".into();
        assert!(validate_window_completion(&draft, &a, false).is_err());
        assert!(validate_window_completion(&draft, &b, false).is_err());
        assert!(validate_window_completion(&draft, &c, false).is_ok());
        draft.events[0].annotation_status = "blind_confirmed".into();
        assert!(validate_window_completion(&draft, &a, false).is_ok());
        assert!(validate_window_completion(&draft, &b, false).is_ok());
        draft.events[0].annotation_status = "review_pending".into();
        assert!(validate_window_completion(&draft, &a, true).is_err());
        draft.events[0].annotation_status = "reviewed".into();
        assert!(validate_window_completion(&draft, &a, true).is_ok());
    }

    /// Exercises real initialize/save/load/QA/export boundaries with a frozen artifact.
    #[test]
    fn synthetic_integrity_workflow_a_through_d() {
        let root = tempfile::tempdir().unwrap();
        let media_root = tempfile::tempdir().unwrap();
        let directory = root.path().join("meeting-1");
        fs::create_dir_all(&directory).unwrap();
        let artifact = directory.join("production.json");
        write_artifact(&artifact, &production_artifact("meeting-1")).unwrap();
        let media = root.path().join("source.wav");
        fs::write(&media, b"RIFF-test").unwrap();
        let windows = (0..200)
            .map(|i| {
                serde_json::json!({
                    "window_id": format!("w{i}"), "meeting_id": "meeting-1",
                    "source_start_ms": i * 10, "source_end_ms": (i + 1) * 10,
                    "production_artifact_path": artifact
                })
            })
            .collect::<Vec<_>>();
        for pass in ["blind", "review"] {
            write_jsonl(
                &directory.join(format!("annotation_windows.{pass}.jsonl")),
                &windows,
            )
            .unwrap();
        }
        let mut snapshot = initialize_annotation_project_inner(
            InitializeProjectRequest {
                dataset_dir: root.path().into(),
                meeting_id: "meeting-1".into(),
                source_media_path: media,
                production_artifact_path: artifact,
            },
            media_root.path(),
        )
        .unwrap();
        snapshot.draft.events.push(event("e", 0, 200, None));
        snapshot.draft.events[0].kind = "noise".into();
        snapshot.draft.events[0].annotation_status = "pending".into();
        let save = |snapshot: &WorkspaceSnapshot| {
            save_workspace_inner(WorkspaceSaveRequest {
                dataset_dir: root.path().into(),
                draft: snapshot.draft.clone(),
                session: snapshot.session.clone(),
            })
        };
        save(&snapshot).unwrap();
        let prior = fs::read(directory.join("annotation_session.json")).unwrap();
        snapshot
            .session
            .window_status
            .insert("w0".into(), "reviewed_blind".into());
        assert!(save(&snapshot)
            .unwrap_err()
            .to_string()
            .contains("unconfirmed"));
        assert_eq!(
            prior,
            fs::read(directory.join("annotation_session.json")).unwrap()
        );
        snapshot.draft.events[0].annotation_status = "blind_confirmed".into();
        save(&snapshot).unwrap();

        let root_manifest = root.path().join("manifest.jsonl");
        let meeting_manifest = directory.join("manifest.jsonl");
        fs::write(&root_manifest, b"\n").unwrap();
        fs::write(&meeting_manifest, b"existing meeting manifest\n").unwrap();
        let rejected = |snapshot: &WorkspaceSnapshot, message: &str| {
            let root_before = fs::read(&root_manifest).unwrap();
            let meeting_before = fs::read(&meeting_manifest).unwrap();
            assert!(
                export_manifest(root.path(), &snapshot.draft, &snapshot.session)
                    .unwrap_err()
                    .to_string()
                    .contains(message)
            );
            assert_eq!(root_before, fs::read(&root_manifest).unwrap());
            assert_eq!(meeting_before, fs::read(&meeting_manifest).unwrap());
        };
        rejected(&snapshot, "Blind annotation is incomplete");
        for i in 0..200 {
            snapshot
                .session
                .window_status
                .insert(format!("w{i}"), "reviewed_blind".into());
        }
        save(&snapshot).unwrap();
        // Simulate historical corrupt state without passing through the new save gate.
        snapshot.draft.events[0].annotation_status = "pending".into();
        atomic_write(
            &directory.join("annotations.draft.json"),
            &serde_json::to_vec(&snapshot.draft).unwrap(),
        )
        .unwrap();
        let load_review = || {
            load_workspace_inner(WorkspaceLoadRequest {
                dataset_dir: root.path().into(),
                meeting_id: "meeting-1".into(),
                mode: AnnotationMode::Review,
            })
        };
        assert!(load_review()
            .unwrap_err()
            .to_string()
            .contains("Blind pass contains 1"));
        snapshot.draft.events[0].annotation_status = "blind_confirmed".into();
        save(&snapshot).unwrap();
        assert!(load_review().unwrap().review_evidence.is_some());
        assert!(
            qa_workspace(root.path(), &snapshot.draft, &snapshot.session)
                .unwrap()
                .errors
                .is_empty()
        );
        for i in 0..100 {
            snapshot
                .session
                .window_status
                .insert(format!("w{i}"), "reviewed_second_pass".into());
        }
        save(&snapshot).unwrap();
        rejected(&snapshot, "Review annotation is incomplete");
        for i in 100..199 {
            snapshot
                .session
                .window_status
                .insert(format!("w{i}"), "reviewed_second_pass".into());
        }
        rejected(&snapshot, "1 / 200");
        snapshot.session.window_status.remove("w199");
        rejected(&snapshot, "incomplete");
        snapshot
            .session
            .window_status
            .insert("w199".into(), "reviewed_second_pass".into());
        snapshot.draft.events[0].annotation_status = "pending".into();
        rejected(&snapshot, "QA must pass");
        snapshot.draft.events[0].annotation_status = "review_pending".into();
        assert!(save(&snapshot).is_err());
        rejected(&snapshot, "QA must pass");
        // Reopen affected Review windows while retaining their completed Blind pass.
        for i in 0..20 {
            snapshot
                .session
                .window_status
                .insert(format!("w{i}"), "reviewed_blind".into());
        }
        save(&snapshot).unwrap();
        assert!(load_review().is_ok());
        for i in 0..20 {
            snapshot
                .session
                .window_status
                .insert(format!("w{i}"), "reviewed_second_pass".into());
        }
        snapshot.draft.events[0].annotation_status = "reviewed".into();
        save(&snapshot).unwrap();
        snapshot.draft.events[0].notes = "unsaved edit".into();
        rejected(&snapshot, "Save the current draft");
        snapshot.draft.events[0].notes.clear();
        fs::write(&root_manifest, b"invalid-json\n").unwrap();
        rejected(&snapshot, "parse");
        fs::write(&root_manifest, b"\n").unwrap();
        let report = export_manifest(root.path(), &snapshot.draft, &snapshot.session).unwrap();
        assert_eq!(report.coverage.scorable_samples, 1);
        assert_eq!(
            fs::read(&root_manifest).unwrap(),
            fs::read(&meeting_manifest).unwrap()
        );
        assert_eq!(
            dataset::check_dataset(root.path())
                .unwrap()
                .coverage
                .scorable_samples,
            1
        );
    }

    #[test]
    fn short_ordinary_control_is_rejected_by_qa() {
        let mut draft = AnnotationDraft {
            schema_version: 1,
            meeting_id: "meeting".into(),
            events: vec![event("control", 0, 1_200, None)],
        };
        draft.events[0].kind = "ordinary_speech_control".into();
        let report = qa_workspace(
            Path::new("."),
            &draft,
            &AnnotationSession::empty("meeting".into()),
        )
        .unwrap();
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("longer than 1200")));
    }

    fn production_artifact(meeting_id: &str) -> MeetingProductionArtifact {
        MeetingProductionArtifact {
            schema_version: ARTIFACT_SCHEMA_VERSION,
            artifact_id: "artifact-1".into(),
            transcription_run_id: "run-1".into(),
            meeting_id: meeting_id.into(),
            source_audio: SourceAudioMetadata {
                path_hint: None,
                duration_ms: 2_000,
                sha256: None,
            },
            created_at: "2026-09-14T00:00:00Z".into(),
            app_commit_sha: PHASE_2C1_FROZEN_BASELINE_COMMIT.into(),
            asr: ArtifactBackend {
                backend: "test".into(),
                model: "test".into(),
                version_or_hash: None,
            },
            diarization: ArtifactBackend {
                backend: "test".into(),
                model: "test".into(),
                version_or_hash: None,
            },
            production_config: ProductionConfigSnapshot::default(),
            transcripts: vec![ArtifactTranscript {
                id: "t1".into(),
                start_ms: 100,
                end_ms: 300,
                text: "ok".into(),
                asr_confidence: None,
            }],
            raw_diarizer_turns: vec![ArtifactDiarizerTurn {
                start_ms: 0,
                end_ms: 1_000,
                speaker_key: "speaker_01".into(),
                confidence: None,
                overlap: false,
            }],
            vad_events: vec![ArtifactVadEvent {
                start_ms: 100,
                end_ms: 300,
                confidence: None,
            }],
            accepted_speakers: vec!["speaker_01".into()],
            visible_speakers: vec!["speaker_01".into()],
            safety_observations: ProductionSafetyObservations::default(),
            production_metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn initializes_project_and_binds_artifact_identity() {
        let root = tempfile::tempdir().unwrap();
        let media_root = tempfile::tempdir().unwrap();
        let meeting_id = "meeting-1";
        let directory = root.path().join(meeting_id);
        fs::create_dir_all(&directory).unwrap();
        let artifact_path = directory.join("production.json");
        write_artifact(&artifact_path, &production_artifact(meeting_id)).unwrap();
        let media_path = root.path().join("source.wav");
        fs::write(&media_path, b"RIFF-test").unwrap();
        let row = serde_json::json!({"window_id":"w1","meeting_id":meeting_id,"source_start_ms":0,"source_end_ms":1000,"audio_path":"w1.wav","production_artifact_path":artifact_path});
        let line = format!("{}\n", serde_json::to_string(&row).unwrap());
        fs::write(directory.join("annotation_windows.blind.jsonl"), &line).unwrap();
        fs::write(directory.join("annotation_windows.review.jsonl"), &line).unwrap();
        let snapshot = initialize_annotation_project_inner(
            InitializeProjectRequest {
                dataset_dir: root.path().to_path_buf(),
                meeting_id: meeting_id.into(),
                source_media_path: media_path,
                production_artifact_path: artifact_path.clone(),
            },
            media_root.path(),
        )
        .unwrap();
        assert!(snapshot.initialized);
        assert!(directory.join("annotation_project.json").is_file());
        assert!(snapshot
            .session
            .source_media_path
            .contains("mityu-recordings"));
        assert_eq!(
            snapshot.session.artifact_identity.unwrap().artifact_id,
            "artifact-1"
        );
        fs::write(&artifact_path, b"replaced").unwrap();
        assert!(load_workspace_inner(WorkspaceLoadRequest {
            dataset_dir: root.path().to_path_buf(),
            meeting_id: meeting_id.into(),
            mode: AnnotationMode::Blind,
        })
        .unwrap_err()
        .to_string()
        .contains("SHA-256 mismatch"));
    }
}
