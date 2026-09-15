use crate::database::models::Transcript;
use crate::database::repositories::speaker_turn::SpeakerTurn;
use crate::diarization::types::SegmentKind;

use super::*;

fn span(id: &str, start_ms: i64, end_ms: i64, text: &str, speaker: &str) -> AtomicSpan {
    AtomicSpan {
        start_ms,
        end_ms,
        timing_reliable: true,
        text: text.into(),
        speaker_attribution: SpeakerAttribution::Single {
            speaker_key: speaker.into(),
        },
        source_transcript_ids: vec![id.into()],
        asr_confidence: Some(0.9),
        speaker_confidence: Some(0.9),
        overlap: false,
        segment_kind: SegmentKind::Speech,
        short_turn_confidence: None,
    }
}

fn reconstruct_spans(spans: &[AtomicSpan]) -> ReconstructionResult {
    reconstruct(
        "meeting-test",
        spans,
        &UtteranceReconstructionConfig::default(),
    )
}

#[test]
fn same_speaker_and_250ms_pause_merges_with_continuation_hint() {
    let result = reconstruct_spans(&[
        span("t1", 0, 1_000, "我觉得这个方案", "a"),
        span("t2", 1_250, 2_000, "其实还是可以的", "a"),
    ]);
    assert_eq!(result.utterances.len(), 1);
    assert_eq!(result.utterances[0].text, "我觉得这个方案其实还是可以的");
    assert_eq!(result.boundaries[0].decision, BoundaryDecision::Merge);
    assert!(result.boundaries[0]
        .reasons
        .contains(&BoundaryReason::ContinuationPrefix));
}

#[test]
fn same_speaker_and_2000ms_silence_splits() {
    let result = reconstruct_spans(&[
        span("t1", 0, 1_000, "第一句", "a"),
        span("t2", 3_000, 4_000, "第二句", "a"),
    ]);
    assert_eq!(result.utterances.len(), 2);
    assert_eq!(result.boundaries[0].decision, BoundaryDecision::Split);
    assert!(result.boundaries[0]
        .reasons
        .contains(&BoundaryReason::LongSilence));
}

#[test]
fn reliable_speaker_change_splits() {
    let result = reconstruct_spans(&[
        span("t1", 0, 1_000, "这个问题先这样", "a"),
        span("t2", 1_100, 2_000, "我补充一点", "b"),
    ]);
    assert_eq!(result.utterances.len(), 2);
    assert!(result.boundaries[0]
        .reasons
        .contains(&BoundaryReason::SpeakerChanged));
}

#[test]
fn high_confidence_backchannel_is_preserved_and_does_not_break_a() {
    let mut backchannel = span("t2", 3_300, 3_600, "嗯", "b");
    backchannel.segment_kind = SegmentKind::Backchannel;
    backchannel.short_turn_confidence = Some(0.95);
    let result = reconstruct_spans(&[
        span("t1", 0, 3_200, "我觉得这个模型", "a"),
        backchannel,
        span("t3", 3_700, 7_000, "整体已经可以用了", "a"),
    ]);
    assert_eq!(result.utterances.len(), 1);
    assert_eq!(result.utterances[0].text, "我觉得这个模型整体已经可以用了");
    assert_eq!(result.utterances[0].embedded_events.len(), 1);
    assert_eq!(result.utterances[0].embedded_events[0].text, "嗯");
    assert_eq!(
        result.utterances[0].embedded_events[0].source_transcript_ids,
        ["t2"]
    );
}

#[test]
fn short_speech_remains_three_normal_speaker_utterances() {
    let result = reconstruct_spans(&[
        span("t1", 0, 1_000, "我觉得这个方案可以", "a"),
        span("t2", 1_100, 1_500, "我不同意", "b"),
        span("t3", 1_600, 2_500, "原因是……", "a"),
    ]);
    assert_eq!(result.utterances.len(), 3);
    assert!(result
        .utterances
        .iter()
        .all(|item| item.embedded_events.is_empty()));
}

#[test]
fn mixed_or_overlap_is_not_force_assigned_or_merged() {
    let mut mixed = span("t1", 0, 2_000, "无法可靠内部切分", "a");
    mixed.speaker_attribution = SpeakerAttribution::Mixed {
        speaker_keys: vec!["a".into(), "b".into()],
    };
    mixed.overlap = true;
    let result = reconstruct_spans(&[mixed, span("t2", 2_100, 3_000, "后续", "b")]);
    assert_eq!(result.utterances.len(), 2);
    assert!(result.utterances[0].mixed);
    assert_eq!(result.utterances[0].text, "无法可靠内部切分");
}

#[test]
fn punctuation_free_chinese_and_mixed_language_do_not_crash_or_corrupt_text() {
    let result = reconstruct_spans(&[
        span(
            "t1",
            0,
            1_000,
            "我觉得这个方案其实还可以但是目前识别还有问题",
            "a",
        ),
        span("t2", 1_200, 2_000, "这个 module", "a"),
        span("t3", 2_200, 3_000, "latency is high", "a"),
    ]);
    assert_eq!(result.utterances.len(), 1);
    assert!(result.utterances[0].text.contains("这个 module"));
    assert!(result.utterances[0].text.contains("latency is high"));
}

#[test]
fn transcript_crossing_two_speaker_turns_is_marked_mixed_without_text_split() {
    let transcript = raw_transcript("t1", 10.0, 18.0, "我觉得这个方案可以但是目前还有问题");
    let turns = vec![
        SpeakerTurn {
            start_ms: 10_000,
            end_ms: 14_000,
            speaker_label: "Speaker 1".into(),
            confidence: Some(0.9),
            speaker_key: "a".into(),
        },
        SpeakerTurn {
            start_ms: 14_000,
            end_ms: 18_000,
            speaker_label: "Speaker 2".into(),
            confidence: Some(0.9),
            speaker_key: "b".into(),
        },
    ];
    let spans = normalize_timeline(&[transcript], &turns, &[]);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].text, "我觉得这个方案可以但是目前还有问题");
    assert!(matches!(
        spans[0].speaker_attribution,
        SpeakerAttribution::Mixed { .. }
    ));
}

#[test]
fn persisted_overlap_never_becomes_a_single_speaker_claim() {
    let mut transcript = raw_transcript("t1", 10.0, 12.0, "重叠内容");
    transcript.speaker_overlap = 1;
    let turns = vec![SpeakerTurn {
        start_ms: 10_000,
        end_ms: 12_000,
        speaker_label: "Speaker 1".into(),
        confidence: Some(0.9),
        speaker_key: "a".into(),
    }];
    let spans = normalize_timeline(&[transcript], &turns, &[]);
    assert!(matches!(
        spans[0].speaker_attribution,
        SpeakerAttribution::Mixed { .. }
    ));
}

#[test]
fn reconstruction_is_deterministic() {
    let spans = [
        span("t1", 0, 1_000, "one", "a"),
        span("t2", 1_200, 2_000, "two", "a"),
    ];
    assert_eq!(reconstruct_spans(&spans), reconstruct_spans(&spans));
}

#[test]
fn every_raw_source_and_lexical_text_is_preserved_once() {
    let mut backchannel = span("t2", 1_100, 1_300, "嗯", "b");
    backchannel.segment_kind = SegmentKind::Backchannel;
    backchannel.short_turn_confidence = Some(0.9);
    let spans = [
        span("t1", 0, 1_000, "我觉得", "a"),
        backchannel,
        span("t3", 1_400, 2_000, "可以", "a"),
    ];
    let result = reconstruct_spans(&spans);
    let mut reconstructed = result
        .utterances
        .iter()
        .flat_map(|utterance| {
            std::iter::once((
                utterance.source_transcript_ids.clone(),
                utterance.text.clone(),
            ))
            .chain(
                utterance
                    .embedded_events
                    .iter()
                    .map(|event| (event.source_transcript_ids.clone(), event.text.clone())),
            )
        })
        .flat_map(|(ids, text)| {
            if ids.len() == 1 {
                vec![(ids[0].clone(), text)]
            } else {
                ids.into_iter()
                    .map(|id| {
                        let raw = spans
                            .iter()
                            .find(|span| span.source_transcript_ids[0] == id)
                            .expect("source exists");
                        (id, raw.text.clone())
                    })
                    .collect()
            }
        })
        .collect::<Vec<_>>();
    reconstructed.sort_by(|left, right| left.0.cmp(&right.0));
    let mut raw = spans
        .iter()
        .map(|span| (span.source_transcript_ids[0].clone(), span.text.clone()))
        .collect::<Vec<_>>();
    raw.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(reconstructed, raw);
}

#[test]
fn merged_text_preserves_raw_lexical_content_and_order() {
    let spans = [
        span("t1", 0, 1_000, "这个 module", "a"),
        span("t2", 1_200, 2_000, "latency 还是有点高。", "a"),
    ];
    let result = reconstruct_spans(&spans);
    let raw = spans
        .iter()
        .map(|span| span.text.as_str())
        .collect::<String>();
    let reconstructed = result
        .utterances
        .iter()
        .map(|utterance| utterance.text.as_str())
        .collect::<String>();
    assert_eq!(lexical(&raw), lexical(&reconstructed));
}

fn lexical(text: &str) -> String {
    text.chars()
        .filter(|character| {
            !character.is_whitespace() && !"。！？.!?，、；：,;:".contains(*character)
        })
        .collect()
}

fn raw_transcript(id: &str, start: f64, end: f64, text: &str) -> Transcript {
    Transcript {
        id: id.into(),
        meeting_id: "meeting-test".into(),
        transcript: text.into(),
        timestamp: "00:00:10".into(),
        summary: None,
        action_items: None,
        key_points: None,
        audio_start_time: Some(start),
        audio_end_time: Some(end),
        duration: Some(end - start),
        asr_confidence: Some(0.9),
        speaker_id: Some("a".into()),
        speaker_confidence: Some(0.5),
        speaker_provisional: 0,
        speaker_revision: 1,
        segment_kind: Some("speech".into()),
        audio_source: Some("mixed".into()),
        speaker_assignment_method: "diarization".into(),
        speaker_overlap: 0,
    }
}
