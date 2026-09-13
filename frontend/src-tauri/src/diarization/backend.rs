//! Backend boundary for diarization engines. The current sidecar is one
//! implementation; a Rust/ORT backend can be added without touching ASR.

use super::models::ModelPaths;
use super::types::SpeakerSegment;
use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;

#[async_trait]
pub trait DiarizationBackend: Send + Sync {
    fn name(&self) -> &'static str;
    async fn diarize(&self, wav: &Path, models: &ModelPaths) -> Result<Vec<SpeakerSegment>>;
}

pub struct SidecarDiarizationBackend;

#[async_trait]
impl DiarizationBackend for SidecarDiarizationBackend {
    fn name(&self) -> &'static str {
        "diarize-helper"
    }

    async fn diarize(&self, wav: &Path, models: &ModelPaths) -> Result<Vec<SpeakerSegment>> {
        let binary = crate::diarization::sidecar::resolve_binary()?;
        let outcome =
            crate::diarization::sidecar::run(&binary, wav, &models.segmentation, &models.embedding)
                .await?;
        Ok(super::offline::segments_from_turns(&outcome.turns))
    }
}
