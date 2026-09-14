//! Shared preparation service for Blind and Review annotation windows.
//!
//! Both the reproducible CLI and the local Tauri preparation command call this
//! module. It is the only implementation of audio decoding, candidate
//! extraction, window slicing, and annotation-window manifest generation.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::diarization::short_turn::{
    ShortTurnCandidateExtractor, TranscriptCandidateInput, VadEventCandidateInput,
};
use crate::diarization::types::{
    AssignmentMethod, AudioSource, SegmentKind, SpeakerSegment, TranscriptTiming,
};

use super::annotation_workspace::meeting_dir;
use super::production_artifact::{read_and_validate_artifact, MeetingProductionArtifact};

pub const DEFAULT_WINDOW_MS: i64 = 5_000;
pub const DEFAULT_STRIDE_MS: i64 = 4_000;

#[derive(Debug, Clone)]
pub struct ShortTurnExportRequest {
    pub audio: PathBuf,
    pub meeting_id: String,
    pub output: PathBuf,
    pub production_artifact: PathBuf,
    pub window_ms: i64,
    pub stride_ms: i64,
    pub overwrite_existing: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortTurnExportResult {
    pub window_count: usize,
    pub candidate_count: usize,
    pub blind_manifest: PathBuf,
    pub review_manifest: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareAnnotationWindowsRequest {
    pub dataset_root: PathBuf,
    pub source_media: PathBuf,
    pub production_artifact: PathBuf,
    pub meeting_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareAnnotationWindowsResult {
    pub meeting_id: String,
    pub blind_window_count: usize,
    pub review_window_count: usize,
    pub candidate_count: usize,
    pub controlled_production_artifact: PathBuf,
    pub blind_manifest: PathBuf,
    pub review_manifest: PathBuf,
}

fn validate_source_media(path: &Path) -> Result<()> {
    if !path.is_file() {
        bail!("source media does not exist: {}", path.display());
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "wav" | "mp3" | "m4a" | "mp4" | "webm") {
        bail!("source media type is unsupported; select WAV, MP3, M4A, MP4, or WebM");
    }
    Ok(())
}

fn existing_window_output(directory: &Path) -> Result<bool> {
    if directory.join("annotation_windows.blind.jsonl").exists()
        || directory.join("annotation_windows.review.jsonl").exists()
    {
        return Ok(true);
    }
    if !directory.is_dir() {
        return Ok(false);
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("window_") && name.ends_with(".wav"))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn project_is_initialized(directory: &Path) -> bool {
    [
        "annotation_project.json",
        "annotations.draft.json",
        "annotation_session.json",
    ]
    .iter()
    .any(|name| directory.join(name).exists())
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn copy_artifact_without_overwrite(source: &Path, destination: &Path) -> Result<()> {
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .with_context(|| format!("create controlled artifact {}", destination.display()))?;
    if let Err(error) = std::io::copy(&mut input, &mut output).and_then(|_| output.sync_all()) {
        drop(output);
        let _ = fs::remove_file(destination);
        return Err(error).context("copy Production Artifact into controlled meeting directory");
    }
    Ok(())
}

fn candidate_inputs(
    artifact: &MeetingProductionArtifact,
) -> (
    Vec<TranscriptCandidateInput>,
    Vec<SpeakerSegment>,
    Vec<VadEventCandidateInput>,
) {
    let transcripts = artifact
        .transcripts
        .iter()
        .map(|row| TranscriptCandidateInput {
            timing: TranscriptTiming {
                id: row.id.clone(),
                start_ms: row.start_ms,
                end_ms: row.end_ms,
                audio_source: AudioSource::Imported,
            },
            text: row.text.clone(),
            asr_confidence: row.asr_confidence,
        })
        .collect();
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
        .collect();
    let vad = artifact
        .vad_events
        .iter()
        .map(|event| VadEventCandidateInput {
            start_ms: event.start_ms,
            end_ms: event.end_ms,
            confidence: event.confidence,
            audio_source: AudioSource::Imported,
        })
        .collect();
    (transcripts, speakers, vad)
}

pub fn export_annotation_windows(request: ShortTurnExportRequest) -> Result<ShortTurnExportResult> {
    if request.meeting_id.trim().is_empty() {
        bail!("meeting id must not be empty");
    }
    if request.window_ms <= 0 || request.stride_ms <= 0 || request.stride_ms > request.window_ms {
        bail!("window and stride must be positive, with stride <= window");
    }
    validate_source_media(&request.audio)?;
    if !request.overwrite_existing && existing_window_output(&request.output)? {
        bail!("annotation windows already exist for this meeting");
    }
    fs::create_dir_all(&request.output)
        .with_context(|| format!("create window output {}", request.output.display()))?;
    let artifact =
        read_and_validate_artifact(&request.production_artifact, Some(&request.meeting_id))?;
    let (transcripts, speakers, vad) = candidate_inputs(&artifact);
    let candidates = ShortTurnCandidateExtractor {
        config: artifact.production_config.short_turn.clone(),
    }
    .extract(&transcripts, &speakers, &vad);
    let decoded = crate::audio::decoder::decode_audio_file(&request.audio)
        .context("audio decode failed while preparing annotation windows")?;
    let samples = decoded.to_whisper_format();
    let audio_end_ms = samples.len() as i64 * 1_000 / 16_000;
    if audio_end_ms <= 0 {
        bail!("source media decoded to an empty audio stream");
    }
    let blind_manifest = request.output.join("annotation_windows.blind.jsonl");
    let review_manifest = request.output.join("annotation_windows.review.jsonl");
    let mut blind_writer = BufWriter::new(File::create(&blind_manifest)?);
    let mut review_writer = BufWriter::new(File::create(&review_manifest)?);
    let artifact_path = request
        .production_artifact
        .canonicalize()
        .context("resolve controlled Production Artifact path")?
        .to_string_lossy()
        .into_owned();

    let mut window_start = 0;
    let mut window_count = 0;
    while window_start < audio_end_ms {
        let window_end = (window_start + request.window_ms).min(audio_end_ms);
        window_count += 1;
        let filename = format!("window_{window_count:04}.wav");
        write_clip(
            &request.output.join(&filename),
            &samples,
            window_start,
            window_end,
        )
        .with_context(|| format!("window generation failed for {filename}"))?;
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
            "window_id": format!("{}-window-{window_count:04}", request.meeting_id),
            "meeting_id": request.meeting_id,
            "audio_path": filename,
            "source_start_ms": window_start,
            "source_end_ms": window_end,
            "production_artifact_path": &artifact_path,
            "instructions": "Annotate every event on the source meeting timeline, including missed speech, noise, ordinary non-short controls, overlap, handoff, and uncertain cases. Write separate ground_truth_event rows to manifest.jsonl."
        });
        writeln!(blind_writer, "{}", serde_json::to_string(&common)?)?;
        let mut review = common;
        review
            .as_object_mut()
            .expect("annotation window object")
            .insert(
                "candidate_suggestions".into(),
                serde_json::json!(suggestions),
            );
        writeln!(review_writer, "{}", serde_json::to_string(&review)?)?;
        if window_end == audio_end_ms {
            break;
        }
        window_start += request.stride_ms;
    }
    blind_writer.flush()?;
    review_writer.flush()?;
    Ok(ShortTurnExportResult {
        window_count,
        candidate_count: candidates.len(),
        blind_manifest,
        review_manifest,
    })
}

pub fn prepare_annotation_windows(
    request: PrepareAnnotationWindowsRequest,
) -> Result<PrepareAnnotationWindowsResult> {
    if !request.dataset_root.is_dir() {
        bail!(
            "dataset root does not exist: {}",
            request.dataset_root.display()
        );
    }
    let dataset_root = request.dataset_root.canonicalize()?;
    validate_source_media(&request.source_media)?;
    let source_media = request
        .source_media
        .canonicalize()
        .context("resolve source media path")?;
    let selected_artifact = request
        .production_artifact
        .canonicalize()
        .context("open selected Production Artifact")?;
    let artifact = read_and_validate_artifact(&selected_artifact, None)
        .context("selected Production Artifact is invalid")?;
    if artifact.meeting_id != request.meeting_id {
        bail!(
            "Production Artifact meeting id '{}' does not match selected meeting '{}'",
            artifact.meeting_id,
            request.meeting_id
        );
    }
    let directory = meeting_dir(&dataset_root, &artifact.meeting_id)?;
    if project_is_initialized(&directory) {
        bail!("this annotation project has already been initialized; preparing windows again may invalidate existing annotation state");
    }
    if existing_window_output(&directory)? {
        bail!("annotation windows already exist for this meeting; use the existing data or remove it explicitly before preparing again");
    }
    fs::create_dir_all(&directory)?;
    let controlled_artifact = directory.join(format!("{}.production.json", artifact.meeting_id));
    let source_hash = file_sha256(&selected_artifact)?;
    if controlled_artifact.exists() {
        if file_sha256(&controlled_artifact)? != source_hash {
            bail!("existing Production Artifact differs from selected artifact");
        }
    } else {
        copy_artifact_without_overwrite(&selected_artifact, &controlled_artifact)?;
        if file_sha256(&controlled_artifact)? != source_hash {
            let _ = fs::remove_file(&controlled_artifact);
            bail!("controlled Production Artifact SHA-256 differs after copy");
        }
    }

    let staging = directory.join(format!(".prepare-{}", uuid::Uuid::new_v4()));
    let export = export_annotation_windows(ShortTurnExportRequest {
        audio: source_media,
        meeting_id: artifact.meeting_id.clone(),
        output: staging.clone(),
        production_artifact: controlled_artifact.clone(),
        window_ms: DEFAULT_WINDOW_MS,
        stride_ms: DEFAULT_STRIDE_MS,
        overwrite_existing: false,
    });
    let export = match export {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    let mut moved = Vec::new();
    for entry in fs::read_dir(&staging)? {
        let entry = entry?;
        let destination = directory.join(entry.file_name());
        if let Err(error) = fs::rename(entry.path(), &destination) {
            for path in moved {
                let _ = fs::remove_file(path);
            }
            let _ = fs::remove_dir_all(&staging);
            return Err(error).context("publish prepared annotation windows");
        }
        moved.push(destination);
    }
    fs::remove_dir(&staging)?;
    let blind_manifest = directory.join("annotation_windows.blind.jsonl");
    let review_manifest = directory.join("annotation_windows.review.jsonl");
    Ok(PrepareAnnotationWindowsResult {
        meeting_id: artifact.meeting_id,
        blind_window_count: export.window_count,
        review_window_count: export.window_count,
        candidate_count: export.candidate_count,
        controlled_production_artifact: controlled_artifact,
        blind_manifest,
        review_manifest,
    })
}

#[tauri::command]
pub async fn api_prepare_short_turn_annotation_windows(
    request: PrepareAnnotationWindowsRequest,
) -> Result<PrepareAnnotationWindowsResult, String> {
    tauri::async_runtime::spawn_blocking(move || prepare_annotation_windows(request))
        .await
        .map_err(|error| format!("prepare annotation windows task failed: {error}"))?
        .map_err(|error| format!("prepare annotation windows: {error:#}"))
}

fn write_clip(path: &Path, samples: &[f32], start_ms: i64, end_ms: i64) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::production_artifact::{
        write_artifact, ArtifactBackend, ArtifactDiarizerTurn, ArtifactTranscript,
        ArtifactVadEvent, ProductionConfigSnapshot, ProductionSafetyObservations,
        SourceAudioMetadata, ARTIFACT_SCHEMA_VERSION, PHASE_2C1_FROZEN_BASELINE_COMMIT,
    };

    fn artifact(meeting_id: &str) -> MeetingProductionArtifact {
        MeetingProductionArtifact {
            schema_version: ARTIFACT_SCHEMA_VERSION,
            artifact_id: "artifact-1".into(),
            transcription_run_id: "run-1".into(),
            meeting_id: meeting_id.into(),
            source_audio: SourceAudioMetadata {
                path_hint: None,
                duration_ms: 6_000,
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
                model: "pyannote+campplus".into(),
                version_or_hash: None,
            },
            production_config: ProductionConfigSnapshot::default(),
            transcripts: vec![ArtifactTranscript {
                id: "transcript-1".into(),
                start_ms: 100,
                end_ms: 450,
                text: "嗯".into(),
                asr_confidence: Some(0.9),
            }],
            raw_diarizer_turns: vec![ArtifactDiarizerTurn {
                start_ms: 0,
                end_ms: 1_000,
                speaker_key: "speaker_01".into(),
                confidence: Some(0.9),
                overlap: false,
            }],
            vad_events: vec![ArtifactVadEvent {
                start_ms: 100,
                end_ms: 450,
                confidence: Some(0.9),
            }],
            accepted_speakers: vec!["speaker_01".into()],
            visible_speakers: vec!["speaker_01".into()],
            safety_observations: ProductionSafetyObservations::default(),
            production_metadata: serde_json::json!({"snapshot_source":"test"}),
        }
    }

    fn write_silence(path: &Path, duration_ms: usize) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for _ in 0..duration_ms * 16 {
            writer.write_sample(0.0_f32).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let audio = temp.path().join("source.wav");
        let artifact_path = temp.path().join("production.json");
        write_silence(&audio, 6_000);
        write_artifact(&artifact_path, &artifact("meeting-1")).unwrap();
        (temp, audio, artifact_path)
    }

    #[test]
    fn shared_export_keeps_blind_manifest_free_of_suggestions() {
        let (temp, audio, artifact_path) = fixture();
        let output = temp.path().join("windows");
        let result = export_annotation_windows(ShortTurnExportRequest {
            audio,
            meeting_id: "meeting-1".into(),
            output,
            production_artifact: artifact_path,
            window_ms: DEFAULT_WINDOW_MS,
            stride_ms: DEFAULT_STRIDE_MS,
            overwrite_existing: false,
        })
        .unwrap();

        assert_eq!(result.window_count, 2);
        let blind = fs::read_to_string(result.blind_manifest).unwrap();
        let review = fs::read_to_string(result.review_manifest).unwrap();
        assert!(!blind.contains("candidate_suggestions"));
        assert_eq!(review.matches("candidate_suggestions").count(), 2);
    }

    #[test]
    fn preparation_copies_artifact_and_publishes_both_passes() {
        let (temp, audio, artifact_path) = fixture();
        let dataset_root = temp.path().join("dataset");
        fs::create_dir(&dataset_root).unwrap();
        let result = prepare_annotation_windows(PrepareAnnotationWindowsRequest {
            dataset_root,
            source_media: audio,
            production_artifact: artifact_path.clone(),
            meeting_id: "meeting-1".into(),
        })
        .unwrap();

        assert_eq!(result.blind_window_count, 2);
        assert_eq!(result.review_window_count, 2);
        assert_eq!(
            file_sha256(&artifact_path).unwrap(),
            file_sha256(&result.controlled_production_artifact).unwrap()
        );
        assert!(result.blind_manifest.is_file());
        assert!(result.review_manifest.is_file());
        let first_window: serde_json::Value = serde_json::from_str(
            fs::read_to_string(&result.blind_manifest)
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            first_window["production_artifact_path"],
            result
                .controlled_production_artifact
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        );
    }

    #[test]
    fn preparation_reuses_identical_controlled_artifact() {
        let (temp, audio, artifact_path) = fixture();
        let dataset_root = temp.path().join("dataset");
        let meeting = dataset_root.join("meeting-1");
        fs::create_dir_all(&meeting).unwrap();
        fs::copy(&artifact_path, meeting.join("meeting-1.production.json")).unwrap();

        assert!(prepare_annotation_windows(PrepareAnnotationWindowsRequest {
            dataset_root,
            source_media: audio,
            production_artifact: artifact_path,
            meeting_id: "meeting-1".into(),
        })
        .is_ok());
    }

    #[test]
    fn preparation_rejects_mismatch_conflict_existing_windows_and_initialized_project() {
        let (_temp, audio, artifact_path) = fixture();
        for (marker, expected) in [
            (
                "meeting-1.production.json",
                "differs from selected artifact",
            ),
            (
                "annotation_windows.blind.jsonl",
                "annotation windows already exist",
            ),
            ("annotation_project.json", "already been initialized"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let meeting = root.path().join("meeting-1");
            fs::create_dir(&meeting).unwrap();
            fs::write(meeting.join(marker), b"different").unwrap();
            let error = prepare_annotation_windows(PrepareAnnotationWindowsRequest {
                dataset_root: root.path().to_path_buf(),
                source_media: audio.clone(),
                production_artifact: artifact_path.clone(),
                meeting_id: "meeting-1".into(),
            })
            .unwrap_err()
            .to_string();
            assert!(error.contains(expected), "{error}");
        }

        let root = tempfile::tempdir().unwrap();
        let error = prepare_annotation_windows(PrepareAnnotationWindowsRequest {
            dataset_root: root.path().to_path_buf(),
            source_media: audio,
            production_artifact: artifact_path,
            meeting_id: "meeting-2".into(),
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("does not match selected meeting"));
    }

    #[test]
    fn preparation_rejects_missing_paths_and_unsupported_media() {
        let (temp, _audio, artifact_path) = fixture();
        let missing_root = temp.path().join("missing-root");
        let missing_audio = temp.path().join("missing.wav");
        let error = prepare_annotation_windows(PrepareAnnotationWindowsRequest {
            dataset_root: missing_root,
            source_media: missing_audio,
            production_artifact: artifact_path.clone(),
            meeting_id: "meeting-1".into(),
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("dataset root does not exist"));

        let unsupported = temp.path().join("source.txt");
        fs::write(&unsupported, b"audio").unwrap();
        let error = prepare_annotation_windows(PrepareAnnotationWindowsRequest {
            dataset_root: temp.path().to_path_buf(),
            source_media: unsupported,
            production_artifact: artifact_path,
            meeting_id: "meeting-1".into(),
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("source media type is unsupported"));
    }
}
