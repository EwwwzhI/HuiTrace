//! Offline service shared by manual passes, imports and completed recordings.

use super::backend::DiarizationBackend;
use super::models;
use super::types::{AssignmentMethod, AudioSource, SegmentKind, SpeakerSegment};
use crate::database::repositories::speaker_turn::SpeakerTurn;
use anyhow::{bail, Context, Result};
use std::path::Path;

pub struct OfflineDiarizationService<B = super::backend::SidecarDiarizationBackend> {
    backend: B,
}

impl Default for OfflineDiarizationService {
    fn default() -> Self {
        Self {
            backend: super::backend::SidecarDiarizationBackend,
        }
    }
}

impl<B: DiarizationBackend> OfflineDiarizationService<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    pub async fn analyze(&self, audio: &Path, models_dir: &Path) -> Result<Vec<SpeakerSegment>> {
        let paths = match models::status(models_dir).await {
            models::ModelStatus::Available(paths) => paths,
            models::ModelStatus::Missing => bail!("diarization models are not downloaded yet"),
            models::ModelStatus::Corrupted { filename, reason } => {
                bail!("diarization model {filename} failed verification: {reason}")
            }
        };
        let temp = tempfile::Builder::new()
            .prefix("mityu-diarize-")
            .suffix(".wav")
            .tempfile()
            .context("create temporary audio file")?;
        let wav = temp.path().to_path_buf();
        let input = audio.to_path_buf();
        tokio::task::spawn_blocking(move || super::service::prepare_wav(&input, &wav))
            .await
            .context("audio preparation task panicked")??;
        self.backend.diarize(temp.path(), &paths).await
    }
}

pub fn segments_from_turns(turns: &[SpeakerTurn]) -> Vec<SpeakerSegment> {
    let labels: Vec<String> = turns
        .iter()
        .map(|turn| turn.speaker_label.clone())
        .collect();
    turns
        .iter()
        .map(|turn| SpeakerSegment {
            start_ms: turn.start_ms,
            end_ms: turn.end_ms,
            speaker_key: speaker_key_for_label(&turn.speaker_label, &labels),
            speaker_confidence: turn.confidence,
            audio_source: AudioSource::Mixed,
            provisional: false,
            revision: 1,
            segment_kind: SegmentKind::Speech,
            assignment_method: AssignmentMethod::Diarization,
            overlap: false,
        })
        .collect()
}

/// Stable meeting-local keys. These are identifiers, never a claimed identity.
pub fn speaker_key_for_label(label: &str, labels: &[String]) -> String {
    if let Some(number) = label
        .split_whitespace()
        .last()
        .and_then(|s| s.parse::<usize>().ok())
    {
        return format!("speaker_{number:02}");
    }
    let mut unique: Vec<&String> = labels.iter().collect();
    unique.sort();
    unique.dedup();
    let index = unique
        .iter()
        .position(|candidate| candidate.as_str() == label)
        .unwrap_or(0)
        + 1;
    format!("speaker_{index:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_keys_are_stable_and_not_device_names() {
        assert_eq!(speaker_key_for_label("Speaker 7", &[]), "speaker_07");
        assert_eq!(
            speaker_key_for_label("cluster-b", &["cluster-b".into(), "cluster-a".into()]),
            "speaker_02"
        );
    }
}
