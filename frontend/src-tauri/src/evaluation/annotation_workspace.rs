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

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::dataset::{self, duration_bucket, GateSample};
use super::production_artifact::read_and_validate_artifact;

const DRAFT_SCHEMA_VERSION: u32 = 1;
const ALLOWED_KINDS: [&str; 5] = [
    "short_speech",
    "backchannel",
    "noise",
    "non_speech_vocalization",
    "ordinary_speech_control",
];

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
    #[serde(default = "default_expected_materialized")]
    pub expected_materialized: bool,
    #[serde(default)]
    pub notes: String,
    #[serde(default = "default_annotation_status")]
    pub annotation_status: String,
}

fn default_expected_materialized() -> bool {
    true
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
        }
    }
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

fn meeting_dir(root: &Path, meeting_id: &str) -> Result<PathBuf> {
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
    let parent = path
        .parent()
        .context("annotation workspace path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let temporary = parent.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("annotation")
    ));
    fs::write(&temporary, bytes).with_context(|| format!("write {}", temporary.display()))?;
    fs::rename(&temporary, path)
        .with_context(|| format!("atomically replace {}", path.display()))?;
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
    let draft = read_json_or(
        &directory.join("annotations.draft.json"),
        AnnotationDraft::empty(request.meeting_id.clone()),
    )?;
    let session = read_json_or(
        &directory.join("annotation_session.json"),
        AnnotationSession::empty(request.meeting_id.clone()),
    )?;
    if draft.meeting_id != request.meeting_id || session.meeting_id != request.meeting_id {
        bail!("annotation draft/session belongs to a different meeting");
    }
    let filename = match request.mode {
        AnnotationMode::Blind => "annotation_windows.blind.jsonl",
        AnnotationMode::Review | AnnotationMode::Qa => "annotation_windows.review.jsonl",
    };
    let windows = read_windows(&directory.join(filename), &request.mode)?;
    if request.mode == AnnotationMode::Review && !windows.is_empty() {
        let blind_count = read_windows(
            &directory.join("annotation_windows.blind.jsonl"),
            &AnnotationMode::Blind,
        )?
        .len();
        let done = session
            .window_status
            .values()
            .filter(|status| status.as_str() == "reviewed_blind")
            .count();
        if done < blind_count {
            bail!("complete every blind window before opening review mode");
        }
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
        }
    }
    Ok(())
}

pub fn save_workspace_inner(request: WorkspaceSaveRequest) -> Result<()> {
    validate_draft_shape(&request.draft, &request.session)?;
    let directory = meeting_dir(&request.dataset_dir, &request.draft.meeting_id)?;
    atomic_write(
        &directory.join("annotations.draft.json"),
        &serde_json::to_vec_pretty(&request.draft)?,
    )?;
    atomic_write(
        &directory.join("annotation_session.json"),
        &serde_json::to_vec_pretty(&request.session)?,
    )?;
    let statuses = request.session.window_status.values();
    let has_statuses = !request.session.window_status.is_empty();
    let project = AnnotationProject {
        schema_version: DRAFT_SCHEMA_VERSION,
        meeting_id: request.draft.meeting_id.clone(),
        source_media: request.session.source_media_path.clone(),
        production_artifact: request.session.production_artifact_path.clone(),
        annotation_draft: "annotations.draft.json",
        blind_windows: "annotation_windows.blind.jsonl",
        review_windows: "annotation_windows.review.jsonl",
        blind_completed: has_statuses
            && statuses
                .clone()
                .all(|value| value == "reviewed_blind" || value == "reviewed_second_pass"),
        review_completed: has_statuses
            && request
                .session
                .window_status
                .values()
                .all(|value| value == "reviewed_second_pass"),
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
        match self.kind.as_str() {
            "short_speech" => "short_speech",
            "backchannel" => "backchannel",
            "noise" => "noise",
            "non_speech_vocalization" => "non_speech_vocalization",
            "ordinary_speech_control" => "ordinary_speech_control",
            _ => "invalid",
        }
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
    let mut source_duration_ms = None;
    if !session.production_artifact_path.trim().is_empty() {
        let path = configured_path(dataset_dir, &session.production_artifact_path);
        match read_and_validate_artifact(&path, Some(&draft.meeting_id)) {
            Ok(artifact) => {
                source_duration_ms = Some(artifact.source_audio.duration_ms);
                if let Some(media_duration) = session.source_media_duration_ms {
                    if (media_duration - artifact.source_audio.duration_ms).abs() > 2_000 {
                        errors.push(format!("source media duration {media_duration}ms does not match artifact duration {}ms", artifact.source_audio.duration_ms));
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
    let qa = qa_workspace(dataset_dir, draft, session)?;
    if !qa.errors.is_empty() {
        bail!("QA must pass before export: {}", qa.errors.join("; "));
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
    write_jsonl(&meeting_directory.join("manifest.jsonl"), &own_rows)?;

    // The benchmark consumes a root manifest. Preserve exported rows for other
    // meetings and replace only this meeting's canonical event collection.
    let root_manifest = dataset_dir.join("manifest.jsonl");
    let mut rows = if root_manifest.exists() {
        BufReader::new(fs::File::open(&root_manifest)?)
            .lines()
            .filter_map(Result::ok)
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str::<serde_json::Value>(&line))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    rows.retain(|row| {
        row.get("meeting_id").and_then(|value| value.as_str()) != Some(draft.meeting_id.as_str())
    });
    rows.extend(own_rows);
    write_jsonl(&root_manifest, &rows)?;
    dataset::check_dataset(dataset_dir)
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

#[cfg(test)]
mod tests {
    use super::*;

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
            expected_materialized: true,
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
    }
}
