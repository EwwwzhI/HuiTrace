use std::collections::{BTreeMap, BTreeSet};

use crate::database::models::Transcript;
use crate::database::repositories::speaker_turn::SpeakerTurn;
use crate::diarization::short_turn_event::ShortTurnEvent;
use crate::diarization::types::SegmentKind;

use super::types::{AtomicSpan, SpeakerAttribution};

pub fn normalize_timeline(
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

            let attribution = if transcript.speaker_overlap != 0 || keys.len() > 1 {
                SpeakerAttribution::Mixed {
                    speaker_keys: keys.into_iter().collect(),
                }
            } else if transcript.speaker_assignment_method == "manual" {
                transcript
                    .speaker_id
                    .clone()
                    .map(|speaker_key| SpeakerAttribution::Single { speaker_key })
                    .unwrap_or(SpeakerAttribution::Unknown)
            } else if let Some(speaker_key) = keys.into_iter().next() {
                SpeakerAttribution::Single { speaker_key }
            } else {
                transcript
                    .speaker_id
                    .clone()
                    .map(|speaker_key| SpeakerAttribution::Single { speaker_key })
                    .unwrap_or(SpeakerAttribution::Unknown)
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
                speaker_confidence: transcript.speaker_confidence,
                overlap: transcript.speaker_overlap != 0,
                segment_kind,
                short_turn_confidence: linked_event.map(|(_, confidence)| *confidence),
            }
        })
        .collect()
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
