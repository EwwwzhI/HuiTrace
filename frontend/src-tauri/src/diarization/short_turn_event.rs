//! Phase 2C domain model for short events that do not align with ASR rows.
//!
//! Transcript rows remain the text evidence. These events are meeting-local,
//! derived timeline annotations, so an embedded response never forces the
//! enclosing transcript to adopt the response speaker.

use sha2::{Digest, Sha256};

use super::short_turn::{ShortTurnCandidate, ShortTurnCandidateSource, ShortTurnDecision};
use super::types::{AssignmentMethod, AudioSource, SegmentKind, TranscriptTiming};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShortTurnEvent {
    pub id: String,
    pub meeting_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub transcript_id: Option<String>,
    pub kind: SegmentKind,
    pub kind_confidence: f64,
    pub speaker_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_display_name: Option<String>,
    pub speaker_confidence: Option<f64>,
    /// Latest ShortTurnRefiner result, even while a manual override is effective.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic_speaker_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic_speaker_confidence: Option<f64>,
    pub candidate_sources: Vec<ShortTurnCandidateSource>,
    pub audio_source: AudioSource,
    pub revision: i64,
    pub assignment_method: AssignmentMethod,
    pub transcript_aligned: bool,
    pub user_visible: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaterializedShortTurnDecision {
    pub event: Option<ShortTurnEvent>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShortTurnMaterializationPolicy {
    /// Engineering value: short ASR rows own rendering when the merged event
    /// still overlaps at least this much of the row.
    pub aligned_min_iou: f64,
    /// Preserve an otherwise unknown event internally only with strong direct
    /// speech evidence. It remains hidden until a safer classification exists.
    pub strong_internal_evidence: f64,
}

impl Default for ShortTurnMaterializationPolicy {
    fn default() -> Self {
        Self {
            aligned_min_iou: 0.75,
            strong_internal_evidence: 0.75,
        }
    }
}

impl ShortTurnMaterializationPolicy {
    pub fn materialize(
        &self,
        meeting_id: &str,
        candidate: &ShortTurnCandidate,
        decision: &ShortTurnDecision,
        transcripts: &[TranscriptTiming],
    ) -> MaterializedShortTurnDecision {
        let relation = best_transcript_relation(candidate, transcripts);
        let transcript_id = relation.map(|(timing, _)| timing.id.clone());
        let transcript_aligned = relation.is_some_and(|(timing, iou)| {
            candidate
                .candidate_sources
                .contains(&ShortTurnCandidateSource::Transcript)
                && timing.end_ms.saturating_sub(timing.start_ms) <= 1_200
                && iou >= self.aligned_min_iou
        });

        let strong_internal = candidate
            .diarization_confidence
            .unwrap_or_default()
            .max(candidate.vad_confidence.unwrap_or_default())
            >= self.strong_internal_evidence
            && candidate.diarization_coverage_ratio.unwrap_or_default() >= 0.75;
        let (store, user_visible) = match decision.kind {
            SegmentKind::Speech | SegmentKind::Backchannel => (true, true),
            SegmentKind::Unknown => (strong_internal, false),
            SegmentKind::Noise | SegmentKind::NonSpeechVocalization => (false, false),
        };
        if !store {
            return MaterializedShortTurnDecision { event: None };
        }

        MaterializedShortTurnDecision {
            event: Some(ShortTurnEvent {
                id: stable_event_id(meeting_id, candidate),
                meeting_id: meeting_id.to_string(),
                start_ms: candidate.start_ms,
                end_ms: candidate.end_ms,
                transcript_id,
                kind: decision.kind.clone(),
                kind_confidence: decision.kind_confidence,
                speaker_key: decision.speaker_key.clone(),
                speaker_display_name: None,
                speaker_confidence: decision.speaker_confidence,
                automatic_speaker_key: decision.speaker_key.clone(),
                automatic_speaker_confidence: decision.speaker_confidence,
                candidate_sources: sorted_sources(&candidate.candidate_sources),
                audio_source: candidate.audio_source.clone(),
                revision: 1,
                assignment_method: AssignmentMethod::ShortTurnRefinement,
                transcript_aligned,
                user_visible,
            }),
        }
    }
}

pub fn stable_event_id(meeting_id: &str, candidate: &ShortTurnCandidate) -> String {
    let mut hasher = Sha256::new();
    hasher.update(meeting_id.as_bytes());
    // Source availability may change between otherwise equivalent reruns, so
    // provenance must not define identity. Quantized boundaries tolerate small
    // backend jitter while still keeping neighbouring real events distinct.
    hasher.update(quantize_ms(candidate.start_ms).to_le_bytes());
    hasher.update(quantize_ms(candidate.end_ms).to_le_bytes());
    // Transcript segmentation is not physical-event identity. Retranscription,
    // split, and merge operations must not manufacture a new event id.
    let digest = format!("{:x}", hasher.finalize());
    format!("short_turn_{}", &digest[..24])
}

/// Reconcile inferred events with the prior physical events. Stable hashes are
/// useful for exact reruns, but cannot cover boundary jitter without eventually
/// colliding adjacent events. This deterministic one-to-one match is the
/// durability layer for manual annotations.
pub fn reconcile_event_ids(inferred: &mut [ShortTurnEvent], previous: &[ShortTurnEvent]) {
    let mut edges = Vec::new();
    for (new_index, new) in inferred.iter().enumerate() {
        for (old_index, old) in previous.iter().enumerate() {
            if new.meeting_id != old.meeting_id || !compatible_kind(&new.kind, &old.kind) {
                continue;
            }
            let overlap = overlap_ms(new.start_ms, new.end_ms, old.start_ms, old.end_ms);
            let shorter = (new.end_ms - new.start_ms)
                .min(old.end_ms - old.start_ms)
                .max(1);
            let new_center = new.start_ms + (new.end_ms - new.start_ms) / 2;
            let old_center = old.start_ms + (old.end_ms - old.start_ms) / 2;
            let center_distance = (new_center - old_center).abs();
            if overlap * 2 < shorter || center_distance > 200 {
                continue;
            }
            let union = new.end_ms.max(old.end_ms) - new.start_ms.min(old.start_ms);
            let iou_millionths = overlap * 1_000_000 / union.max(1);
            let transcript_bonus =
                i64::from(new.transcript_id.is_some() && new.transcript_id == old.transcript_id);
            edges.push((
                std::cmp::Reverse((iou_millionths, -center_distance, transcript_bonus)),
                new_index,
                old_index,
            ));
        }
    }
    edges.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut used_new = std::collections::HashSet::new();
    let mut used_old = std::collections::HashSet::new();
    for (_, new_index, old_index) in edges {
        if used_new.insert(new_index) && used_old.insert(old_index) {
            inferred[new_index].id = previous[old_index].id.clone();
        }
    }
}

fn compatible_kind(left: &SegmentKind, right: &SegmentKind) -> bool {
    left == right
        || matches!(
            left,
            SegmentKind::Speech | SegmentKind::Backchannel | SegmentKind::Unknown
        ) && matches!(
            right,
            SegmentKind::Speech | SegmentKind::Backchannel | SegmentKind::Unknown
        )
}

fn quantize_ms(value: i64) -> i64 {
    ((value + 25) / 50) * 50
}

pub fn sorted_sources(sources: &[ShortTurnCandidateSource]) -> Vec<ShortTurnCandidateSource> {
    let mut sorted = sources.to_vec();
    sorted.sort();
    sorted.dedup();
    sorted
}

fn best_transcript_relation<'a>(
    candidate: &ShortTurnCandidate,
    transcripts: &'a [TranscriptTiming],
) -> Option<(&'a TranscriptTiming, f64)> {
    transcripts
        .iter()
        .filter(|timing| candidate.transcript_ids.contains(&timing.id))
        .filter_map(|timing| {
            let intersection = overlap_ms(
                candidate.start_ms,
                candidate.end_ms,
                timing.start_ms,
                timing.end_ms,
            );
            (intersection > 0).then(|| {
                let union =
                    candidate.end_ms.max(timing.end_ms) - candidate.start_ms.min(timing.start_ms);
                (timing, intersection as f64 / union.max(1) as f64)
            })
        })
        .max_by(|(left_timing, left_iou), (right_timing, right_iou)| {
            left_iou
                .total_cmp(right_iou)
                .then_with(|| right_timing.id.cmp(&left_timing.id))
        })
}

fn overlap_ms(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> i64 {
    a_end.min(b_end).saturating_sub(a_start.max(b_start)).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diarization::short_turn::ShortTurnEvidence;

    fn candidate(
        start_ms: i64,
        end_ms: i64,
        sources: Vec<ShortTurnCandidateSource>,
    ) -> ShortTurnCandidate {
        ShortTurnCandidate {
            start_ms,
            end_ms,
            duration_ms: end_ms.saturating_sub(start_ms) as u64,
            candidate_sources: sources,
            transcript_ids: vec!["t-1".into()],
            text: String::new(),
            asr_confidence: None,
            diarization_speaker: Some("speaker_02".into()),
            diarization_confidence: Some(0.92),
            diarization_coverage_ratio: Some(1.0),
            direct_diarization_turns: Vec::new(),
            vad_confidence: None,
            audio_source: AudioSource::Mixed,
            overlaps_existing_turn: true,
            true_speaker_overlap: false,
            previous_speaker: None,
            next_speaker: None,
            previous_gap_ms: None,
            next_gap_ms: None,
        }
    }

    fn decision(kind: SegmentKind) -> ShortTurnDecision {
        ShortTurnDecision {
            kind,
            kind_confidence: 0.9,
            speaker_key: Some("speaker_02".into()),
            speaker_confidence: Some(0.8),
            evidence: ShortTurnEvidence {
                normalized_text: String::new(),
                lexical_backchannel: false,
                asr_confidence_present: false,
                diarization_confidence_present: true,
                base_speaker_confidence: 0.92,
                duration_weight: 0.35,
                effective_speaker_confidence: 0.32,
                reason: "test".into(),
            },
            allow_new_speaker: false,
        }
    }

    #[test]
    fn short_transcript_is_stored_but_owned_by_primary_rendering() {
        let candidate = candidate(100, 400, vec![ShortTurnCandidateSource::Transcript]);
        let event = ShortTurnMaterializationPolicy::default()
            .materialize(
                "m-1",
                &candidate,
                &decision(SegmentKind::Backchannel),
                &[TranscriptTiming {
                    id: "t-1".into(),
                    start_ms: 100,
                    end_ms: 400,
                    audio_source: AudioSource::Mixed,
                }],
            )
            .event
            .expect("event");
        assert!(event.transcript_aligned);
        assert!(event.user_visible);
    }

    #[test]
    fn embedded_event_does_not_claim_transcript_rendering() {
        let candidate = candidate(2_100, 2_400, vec![ShortTurnCandidateSource::DiarizerTurn]);
        let event = ShortTurnMaterializationPolicy::default()
            .materialize(
                "m-1",
                &candidate,
                &decision(SegmentKind::Speech),
                &[TranscriptTiming {
                    id: "t-1".into(),
                    start_ms: 0,
                    end_ms: 5_000,
                    audio_source: AudioSource::Mixed,
                }],
            )
            .event
            .expect("event");
        assert!(!event.transcript_aligned);
        assert_eq!(event.transcript_id.as_deref(), Some("t-1"));
    }

    #[test]
    fn identity_ignores_provenance_and_transcript_segmentation() {
        let first = candidate(1_000, 1_300, vec![ShortTurnCandidateSource::DiarizerTurn]);
        let mut retranscribed = candidate(
            1_000,
            1_300,
            vec![
                ShortTurnCandidateSource::DiarizerTurn,
                ShortTurnCandidateSource::VadEvent,
            ],
        );
        retranscribed.transcript_ids = vec!["split-a".into(), "split-b".into()];
        assert_eq!(
            stable_event_id("m-1", &first),
            stable_event_id("m-1", &retranscribed)
        );
    }

    #[test]
    fn reconciliation_survives_sixty_ms_jitter_without_colliding_adjacent_events() {
        let policy = ShortTurnMaterializationPolicy::default();
        let old_a = policy
            .materialize(
                "m-1",
                &candidate(1_000, 1_300, vec![ShortTurnCandidateSource::DiarizerTurn]),
                &decision(SegmentKind::Backchannel),
                &[],
            )
            .event
            .unwrap();
        let old_b = policy
            .materialize(
                "m-1",
                &candidate(1_400, 1_700, vec![ShortTurnCandidateSource::DiarizerTurn]),
                &decision(SegmentKind::Backchannel),
                &[],
            )
            .event
            .unwrap();
        let mut next = vec![
            policy
                .materialize(
                    "m-1",
                    &candidate(
                        1_060,
                        1_360,
                        vec![
                            ShortTurnCandidateSource::Transcript,
                            ShortTurnCandidateSource::VadEvent,
                        ],
                    ),
                    &decision(SegmentKind::Speech),
                    &[],
                )
                .event
                .unwrap(),
            policy
                .materialize(
                    "m-1",
                    &candidate(1_460, 1_760, vec![ShortTurnCandidateSource::VadEvent]),
                    &decision(SegmentKind::Backchannel),
                    &[],
                )
                .event
                .unwrap(),
        ];
        reconcile_event_ids(&mut next, &[old_a.clone(), old_b.clone()]);
        assert_eq!(next[0].id, old_a.id);
        assert_eq!(next[1].id, old_b.id);
        assert_ne!(next[0].id, next[1].id);
    }
}
