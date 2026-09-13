//! Pure overlap-based transcript/speaker timeline reconciliation.

use super::types::{
    AssignmentMethod, SegmentKind, SpeakerSegment, TranscriptSpeakerAssignment, TranscriptTiming,
};

/// Assign the speaker that covers the largest portion of each transcript
/// segment. This intentionally does not use only the transcript start: a VAD
/// segment can span a hand-over. Word-level splitting can replace this later
/// without changing the stored speaker timeline.
pub fn reconcile_transcript(
    transcripts: &[TranscriptTiming],
    speakers: &[SpeakerSegment],
) -> Vec<TranscriptSpeakerAssignment> {
    transcripts
        .iter()
        .map(|transcript| {
            let mut candidates: Vec<(&SpeakerSegment, i64)> = speakers
                .iter()
                .filter_map(|speaker| {
                    let overlap = (transcript.end_ms.min(speaker.end_ms)
                        - transcript.start_ms.max(speaker.start_ms))
                    .max(0);
                    (overlap > 0).then_some((speaker, overlap))
                })
                .collect();
            candidates.sort_by(|(a, a_ms), (b, b_ms)| {
                b_ms.cmp(a_ms)
                    .then_with(|| {
                        b.speaker_confidence
                            .partial_cmp(&a.speaker_confidence)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| a.speaker_key.cmp(&b.speaker_key))
            });

            if let Some((dominant, duration)) = candidates.first() {
                let total = transcript.end_ms.saturating_sub(transcript.start_ms).max(1) as f64;
                TranscriptSpeakerAssignment {
                    transcript_id: transcript.id.clone(),
                    speaker_key: Some(dominant.speaker_key.clone()),
                    // A backend confidence applies to the speaker segment; the
                    // overlap share prevents a confident partial turn claiming a
                    // complete long ASR segment.
                    speaker_confidence: dominant
                        .speaker_confidence
                        .map(|c| c * (*duration as f64 / total)),
                    speaker_provisional: dominant.provisional,
                    speaker_revision: dominant.revision,
                    segment_kind: dominant.segment_kind.clone(),
                    audio_source: transcript.audio_source.clone(),
                    assignment_method: dominant.assignment_method.clone(),
                    overlap: candidates.len() > 1,
                }
            } else {
                TranscriptSpeakerAssignment {
                    transcript_id: transcript.id.clone(),
                    speaker_key: None,
                    speaker_confidence: None,
                    speaker_provisional: false,
                    speaker_revision: 0,
                    segment_kind: SegmentKind::Unknown,
                    audio_source: transcript.audio_source.clone(),
                    assignment_method: AssignmentMethod::Diarization,
                    overlap: false,
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diarization::types::{AudioSource, SpeakerSegment};

    fn speaker(start_ms: i64, end_ms: i64, key: &str) -> SpeakerSegment {
        SpeakerSegment {
            start_ms,
            end_ms,
            speaker_key: key.into(),
            speaker_confidence: Some(1.0),
            audio_source: AudioSource::Mixed,
            provisional: false,
            revision: 1,
            segment_kind: SegmentKind::Speech,
            assignment_method: AssignmentMethod::Diarization,
            overlap: false,
        }
    }

    #[test]
    fn chooses_dominant_overlap_not_the_speaker_at_segment_start() {
        let transcript = TranscriptTiming {
            id: "t".into(),
            start_ms: 10_200,
            end_ms: 12_800,
            audio_source: AudioSource::Mixed,
        };
        let result = reconcile_transcript(
            &[transcript],
            &[
                speaker(10_000, 11_400, "speaker_01"),
                speaker(11_400, 13_000, "speaker_02"),
            ],
        );
        assert_eq!(result[0].speaker_key.as_deref(), Some("speaker_02"));
        assert!(result[0].overlap);
    }

    #[test]
    fn leaves_uncovered_transcript_unassigned() {
        let transcript = TranscriptTiming {
            id: "t".into(),
            start_ms: 0,
            end_ms: 10,
            audio_source: AudioSource::Imported,
        };
        let result = reconcile_transcript(&[transcript], &[]);
        assert_eq!(result[0].speaker_key, None);
        assert_eq!(result[0].segment_kind, SegmentKind::Unknown);
    }
}
