use std::collections::{BTreeMap, BTreeSet};

use crate::database::models::Transcript;
use crate::database::repositories::speaker_turn::SpeakerTurn;
use crate::diarization::short_turn_event::ShortTurnEvent;
use crate::diarization::types::SegmentKind;

use super::config::UtteranceReconstructionConfig;
use super::types::{AtomicSpan, SpeakerAttribution, SpeakerAttributionSource};

/// Frozen raw-row normalizer replayed from commit 416b807. Keep this function
/// independent from the V3 normalizer so future attribution changes cannot
/// mutate benchmark baseline inputs.
pub fn normalize_timeline_v1_frozen(
    transcripts: &[Transcript],
    speaker_turns: &[SpeakerTurn],
    short_turn_events: &[ShortTurnEvent],
) -> Vec<AtomicSpan> {
    let mut event_confidence = BTreeMap::new();
    for event in short_turn_events {
        let Some(transcript_id) = event.transcript_id.as_ref() else {
            continue;
        };
        let replace = match event_confidence.get(transcript_id) {
            Some((_, confidence)) => event.kind_confidence > *confidence,
            None => true,
        };
        if replace {
            event_confidence.insert(
                transcript_id.clone(),
                (event.kind.clone(), event.kind_confidence),
            );
        }
    }

    transcripts
        .iter()
        .map(|transcript| {
            let start = transcript.audio_start_time.map(seconds_to_ms_v1_frozen);
            let end = transcript.audio_end_time.map(seconds_to_ms_v1_frozen);
            let timing_reliable = matches!((start, end), (Some(start), Some(end)) if end >= start);
            let start_ms = start.unwrap_or_default();
            let end_ms = end.filter(|end| *end >= start_ms).unwrap_or(start_ms);
            let keys = if timing_reliable {
                speaker_turns
                    .iter()
                    .filter(|turn| turn.end_ms.min(end_ms) > turn.start_ms.max(start_ms))
                    .map(|turn| turn.speaker_key.clone())
                    .filter(|key| !key.is_empty())
                    .collect::<BTreeSet<_>>()
            } else {
                BTreeSet::new()
            };

            // Historical precedence: cross-speaker evidence wins over manual.
            let attribution = if transcript.speaker_overlap != 0 || keys.len() > 1 {
                SpeakerAttribution::Mixed {
                    speaker_keys: keys.iter().cloned().collect(),
                }
            } else if transcript.speaker_assignment_method == "manual" {
                transcript
                    .speaker_id
                    .clone()
                    .map(|speaker_key| SpeakerAttribution::Single { speaker_key })
                    .unwrap_or(SpeakerAttribution::Unknown)
            } else if let Some(speaker_key) = keys.iter().next().cloned() {
                SpeakerAttribution::Single { speaker_key }
            } else {
                transcript
                    .speaker_id
                    .clone()
                    .map(|speaker_key| SpeakerAttribution::Single { speaker_key })
                    .unwrap_or(SpeakerAttribution::Unknown)
            };
            let speaker_attribution_source = if transcript.speaker_assignment_method == "manual"
                && matches!(attribution, SpeakerAttribution::Single { .. })
            {
                SpeakerAttributionSource::Manual
            } else if !keys.is_empty() {
                SpeakerAttributionSource::ChunkTemporalOverlap
            } else if transcript.speaker_id.is_some() {
                SpeakerAttributionSource::PersistedFallback
            } else {
                SpeakerAttributionSource::Unknown
            };
            let linked_event = event_confidence.get(&transcript.id);

            AtomicSpan {
                start_ms,
                end_ms,
                timing_reliable,
                text: normalize_whitespace_v1_frozen(&transcript.transcript),
                speaker_attribution: attribution,
                source_transcript_ids: vec![transcript.id.clone()],
                asr_confidence: transcript.asr_confidence,
                // Historical V1 did not use attribution confidence to decide
                // boundaries. Do not manufacture modern reliability here.
                speaker_assignment_reliability: None,
                speaker_attribution_source,
                overlap: transcript.speaker_overlap != 0,
                segment_kind: linked_event
                    .map(|(kind, _)| kind.clone())
                    .unwrap_or_else(|| {
                        parse_segment_kind_v1_frozen(transcript.segment_kind.as_deref())
                    }),
                short_turn_confidence: linked_event.map(|(_, confidence)| *confidence),
                lexical_range: None,
            }
        })
        .collect()
}

pub fn normalize_timeline_v3(
    transcripts: &[Transcript],
    speaker_turns: &[SpeakerTurn],
    short_turn_events: &[ShortTurnEvent],
    config: &UtteranceReconstructionConfig,
) -> Vec<AtomicSpan> {
    let mut event_confidence = BTreeMap::new();
    for event in short_turn_events {
        let Some(transcript_id) = event.transcript_id.as_ref() else {
            continue;
        };
        let replace = match event_confidence.get(transcript_id) {
            Some((_, confidence)) => event.kind_confidence > *confidence,
            None => true,
        };
        if replace {
            event_confidence.insert(
                transcript_id.clone(),
                (event.kind.clone(), event.kind_confidence),
            );
        }
    }

    transcripts
        .iter()
        .map(|transcript| {
            let start = transcript.audio_start_time.map(seconds_to_ms);
            let end = transcript.audio_end_time.map(seconds_to_ms);
            let timing_reliable = matches!((start, end), (Some(start), Some(end)) if end >= start);
            let start_ms = start.unwrap_or_default();
            let end_ms = end.filter(|end| *end >= start_ms).unwrap_or(start_ms);
            let overlapping_turns = if timing_reliable {
                speaker_turns
                    .iter()
                    .filter(|turn| turn.end_ms.min(end_ms) > turn.start_ms.max(start_ms))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let keys = overlapping_turns
                .iter()
                .map(|turn| turn.speaker_key.clone())
                .filter(|key| !key.is_empty())
                .collect::<BTreeSet<_>>();

            let chunk_duration_ms = end_ms.saturating_sub(start_ms);
            let overlap_by_speaker = keys
                .iter()
                .map(|key| {
                    let mut intervals = overlapping_turns
                        .iter()
                        .filter(|turn| &turn.speaker_key == key)
                        .map(|turn| (turn.start_ms.max(start_ms), turn.end_ms.min(end_ms)))
                        .collect::<Vec<_>>();
                    intervals.sort_unstable();
                    let overlap_ms = union_duration(&intervals);
                    (key, overlap_ms)
                })
                .collect::<Vec<_>>();

            let attribution = if transcript.speaker_overlap != 0 {
                SpeakerAttribution::Mixed {
                    speaker_keys: keys.iter().cloned().collect(),
                }
            } else if transcript.speaker_assignment_method == "manual" {
                transcript
                    .speaker_id
                    .clone()
                    .map(|speaker_key| SpeakerAttribution::Single { speaker_key })
                    .unwrap_or(SpeakerAttribution::Unknown)
            } else if keys.len() > 1 {
                SpeakerAttribution::Mixed {
                    speaker_keys: keys.iter().cloned().collect(),
                }
            } else if let Some(speaker_key) = keys.iter().next().cloned() {
                SpeakerAttribution::Single { speaker_key }
            } else {
                transcript
                    .speaker_id
                    .clone()
                    .map(|speaker_key| SpeakerAttribution::Single { speaker_key })
                    .unwrap_or(SpeakerAttribution::Unknown)
            };

            let (speaker_assignment_reliability, speaker_attribution_source) =
                if transcript.speaker_assignment_method == "manual" {
                    (None, SpeakerAttributionSource::Manual)
                } else if matches!(attribution, SpeakerAttribution::Single { .. })
                    && chunk_duration_ms > 0
                    && overlap_by_speaker.len() == 1
                {
                    let best_ratio = overlap_by_speaker[0].1 as f64 / chunk_duration_ms as f64;
                    let reliability = (best_ratio >= config.assignment_min_overlap_ratio
                        && best_ratio >= config.assignment_min_margin)
                        .then_some(best_ratio.clamp(0.0, 1.0));
                    (reliability, SpeakerAttributionSource::ChunkTemporalOverlap)
                } else if !keys.is_empty() {
                    (None, SpeakerAttributionSource::ChunkTemporalOverlap)
                } else if transcript.speaker_id.is_some() {
                    // Persisted confidence describes an earlier assignment procedure. It
                    // must not masquerade as reliability recomputed from accepted turns.
                    (None, SpeakerAttributionSource::PersistedFallback)
                } else {
                    (None, SpeakerAttributionSource::Unknown)
                };

            let linked_event = event_confidence.get(&transcript.id);
            let segment_kind = linked_event
                .map(|(kind, _)| kind.clone())
                .unwrap_or_else(|| parse_segment_kind(transcript.segment_kind.as_deref()));

            AtomicSpan {
                start_ms,
                end_ms,
                timing_reliable,
                text: normalize_whitespace(&transcript.transcript),
                speaker_attribution: attribution,
                source_transcript_ids: vec![transcript.id.clone()],
                asr_confidence: transcript.asr_confidence,
                speaker_assignment_reliability,
                speaker_attribution_source,
                overlap: transcript.speaker_overlap != 0,
                segment_kind,
                short_turn_confidence: linked_event.map(|(_, confidence)| *confidence),
                lexical_range: None,
            }
        })
        .collect()
}

fn union_duration(intervals: &[(i64, i64)]) -> i64 {
    let Some(&(first_start, first_end)) = intervals.first() else {
        return 0;
    };
    let mut total = 0_i64;
    let mut start = first_start;
    let mut end = first_end;
    for &(next_start, next_end) in &intervals[1..] {
        if next_start <= end {
            end = end.max(next_end);
        } else {
            total = total.saturating_add(end.saturating_sub(start));
            start = next_start;
            end = next_end;
        }
    }
    total.saturating_add(end.saturating_sub(start))
}

fn normalize_whitespace_v1_frozen(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn seconds_to_ms_v1_frozen(seconds: f64) -> i64 {
    (seconds * 1_000.0).round() as i64
}

fn parse_segment_kind_v1_frozen(value: Option<&str>) -> SegmentKind {
    match value {
        Some("speech") => SegmentKind::Speech,
        Some("backchannel") => SegmentKind::Backchannel,
        Some("noise") => SegmentKind::Noise,
        Some("non_speech_vocalization") => SegmentKind::NonSpeechVocalization,
        _ => SegmentKind::Unknown,
    }
}

pub fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn join_text(left: &str, right: &str) -> String {
    let left = left.trim_end();
    let right = right.trim_start();
    if left.is_empty() {
        return right.to_string();
    }
    if right.is_empty() {
        return left.to_string();
    }
    let needs_space = left
        .chars()
        .next_back()
        .zip(right.chars().next())
        .is_some_and(|(left, right)| left.is_ascii_alphanumeric() && right.is_ascii_alphanumeric());
    format!("{left}{}{right}", if needs_space { " " } else { "" })
}

fn seconds_to_ms(seconds: f64) -> i64 {
    (seconds * 1_000.0).round() as i64
}

fn parse_segment_kind(value: Option<&str>) -> SegmentKind {
    match value {
        Some("speech") => SegmentKind::Speech,
        Some("backchannel") => SegmentKind::Backchannel,
        Some("noise") => SegmentKind::Noise,
        Some("non_speech_vocalization") => SegmentKind::NonSpeechVocalization,
        _ => SegmentKind::Unknown,
    }
}
