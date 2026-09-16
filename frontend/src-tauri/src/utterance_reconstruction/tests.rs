use crate::audio::transcription::{
    TimedToken, TimingSource, TranscriptTiming, TranscriptionCapabilities,
};
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
        speaker_assignment_reliability: Some(0.9),
        speaker_attribution_source: SpeakerAttributionSource::LexicalTemporalOverlap,
        overlap: false,
        segment_kind: SegmentKind::Speech,
        short_turn_confidence: None,
        lexical_range: None,
    }
}

fn reconstruct_spans(spans: &[AtomicSpan]) -> ReconstructionResult {
    let config = UtteranceReconstructionConfig::default();
    super::assembler::reconstruct_with_details(
        "meeting-test",
        spans,
        &config,
        ReconstructionProfile {
            algorithm_version: ALGORITHM_VERSION_V3.into(),
            timing_mode: ReconstructionTimingMode::NativeLexicalTiming,
            boundary_policy: BoundaryPolicy::V3SemanticBaseline,
            boundary_policy_version: BOUNDARY_POLICY_VERSION_V3.into(),
            semantic_model_version: Some(SEMANTIC_MODEL_VERSION_V1.into()),
            alignment_version: Some(ALIGNMENT_VERSION_V1.into()),
        },
        ReconstructionMetrics::default(),
        vec![],
        vec![],
    )
}

#[derive(serde::Deserialize)]
struct FrozenV1GoldenCase {
    name: String,
    left_speaker: String,
    right_speaker: String,
    gap_ms: i64,
    left_text: String,
    right_text: String,
    timing_reliable: bool,
    mixed: bool,
    overlap: bool,
    expected_utterances: usize,
    expected_score: i32,
    expected_reason: BoundaryReason,
}

#[test]
fn frozen_v1_replays_historical_golden_cases() {
    let cases: Vec<FrozenV1GoldenCase> = serde_json::from_str(include_str!(
        "../../tests/fixtures/utterance_reconstruction/v1/cases.json"
    ))
    .unwrap();
    for case in cases {
        let mut left = span("left", 0, 1_000, &case.left_text, &case.left_speaker);
        let mut right = span(
            "right",
            1_000 + case.gap_ms,
            2_000 + case.gap_ms,
            &case.right_text,
            &case.right_speaker,
        );
        left.timing_reliable = case.timing_reliable;
        right.timing_reliable = case.timing_reliable;
        left.overlap = case.overlap;
        if case.mixed {
            right.speaker_attribution = SpeakerAttribution::Mixed {
                speaker_keys: vec!["a".into(), "b".into()],
            };
        }
        let result = reconstruct(
            "frozen-v1-golden",
            &[left, right],
            &UtteranceReconstructionConfig::frozen_v1(),
        );
        assert_eq!(
            result.utterances.len(),
            case.expected_utterances,
            "{}",
            case.name
        );
        assert_eq!(
            result.boundaries[0].score, case.expected_score,
            "{}",
            case.name
        );
        assert!(
            result.boundaries[0].reasons.contains(&case.expected_reason),
            "{}",
            case.name
        );
        assert_eq!(result.profile.boundary_policy, BoundaryPolicy::V1Frozen);
        assert!(result.boundaries[0].evidence.semantic.is_none());
    }
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
fn same_speaker_medium_gap_and_incomplete_left_merges() {
    let result = reconstruct_spans(&[
        span("t1", 0, 1_000, "我觉得这个", "a"),
        span("t2", 1_900, 3_000, "方案还可以继续推进", "a"),
    ]);
    assert_eq!(result.utterances.len(), 1);
    assert!(result.boundaries[0]
        .reasons
        .contains(&BoundaryReason::SentenceIncomplete));
    assert!(result.boundaries[0]
        .reasons
        .contains(&BoundaryReason::SemanticContinuity));
}

#[test]
fn same_speaker_continuation_prefix_merges_across_vad_fragmentation() {
    let result = reconstruct_spans(&[
        span("t1", 0, 1_000, "我觉得这个方案", "a"),
        span("t2", 1_900, 3_000, "其实问题不大", "a"),
    ]);
    assert_eq!(result.utterances.len(), 1);
    assert!(result.boundaries[0]
        .evidence
        .semantic
        .as_ref()
        .expect("semantic evidence")
        .cross_boundary_continuity
        .is_some_and(|score| score > 0.8));
    assert!(result.boundaries[0].score_components.semantic_score < 0);
    assert_eq!(result.boundaries[0].score_components.semantic_score, -4);
    assert_eq!(result.boundaries[0].score_components.punctuation_score, 0);
}

#[test]
fn semantic_scoring_does_not_double_count_terminal_punctuation() {
    let merged_medium_gap = reconstruct_spans(&[
        span("t1", 0, 1_000, "这个方案今天先确定下来。", "a"),
        span("t2", 1_900, 3_000, "下一项我们讨论预算问题。", "a"),
    ]);
    assert_eq!(merged_medium_gap.utterances.len(), 1);
    assert!(merged_medium_gap.boundaries[0]
        .reasons
        .contains(&BoundaryReason::SentenceComplete));
    assert_eq!(
        merged_medium_gap.boundaries[0]
            .score_components
            .punctuation_score,
        0
    );
    assert_eq!(
        merged_medium_gap.boundaries[0]
            .score_components
            .semantic_score,
        2
    );

    let merged = reconstruct_spans(&[
        span("t1", 0, 1_000, "第一版先这样做。", "a"),
        span("t2", 1_200, 2_000, "第二阶段再优化性能。", "a"),
    ]);
    assert_eq!(merged.utterances.len(), 1);
}

#[test]
fn legacy_language_scores_apply_only_when_semantic_is_disabled() {
    let mut config = UtteranceReconstructionConfig::default();
    config.semantic_boundary_enabled = false;
    let spans = [
        span("t1", 0, 1_000, "已经完成。", "a"),
        span("t2", 1_100, 2_000, "所以继续", "a"),
    ];
    let result = super::assembler::reconstruct_with_details(
        "legacy-language-score",
        &spans,
        &config,
        ReconstructionProfile {
            algorithm_version: ALGORITHM_VERSION_V3.into(),
            timing_mode: ReconstructionTimingMode::ChunkFallback,
            boundary_policy: BoundaryPolicy::V3SemanticBaseline,
            boundary_policy_version: BOUNDARY_POLICY_VERSION_V3.into(),
            semantic_model_version: None,
            alignment_version: Some(ALIGNMENT_VERSION_V1.into()),
        },
        ReconstructionMetrics::default(),
        vec![],
        vec![],
    );
    assert_eq!(result.boundaries[0].score_components.punctuation_score, 2);
    assert_eq!(result.boundaries[0].score_components.semantic_score, -1);
    assert!(result.boundaries[0].evidence.semantic.is_none());
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
    assert!(result.boundaries[0]
        .reasons
        .contains(&BoundaryReason::ReliableSpeakerChange));
    assert_eq!(
        result.boundaries[0].decision_source,
        BoundaryDecisionSource::ReliableSpeakerHandoff
    );
}

#[test]
fn ambiguous_speaker_change_does_not_destroy_strongly_continuous_speech() {
    let mut left = span("t1", 0, 1_000, "我觉得这个方案", "a");
    let mut right = span("t2", 1_100, 2_000, "其实问题不大", "b");
    left.speaker_assignment_reliability = Some(0.40);
    right.speaker_assignment_reliability = Some(0.45);
    let result = reconstruct_spans(&[left, right]);
    assert_eq!(result.utterances.len(), 1);
    assert_eq!(
        result.utterances[0].speaker_attribution,
        SpeakerAttribution::Unknown
    );
    assert!(result.boundaries[0]
        .reasons
        .contains(&BoundaryReason::AmbiguousSpeakerChange));
    assert!(!result.boundaries[0].evidence.speaker_change_reliable);
    assert_eq!(
        result.boundaries[0].decision_source,
        BoundaryDecisionSource::ScoredDecision
    );
}

#[test]
fn unreliable_timing_uses_semantic_fallback_instead_of_hard_split() {
    let mut left = span("t1", 0, 0, "我觉得这个", "a");
    let mut right = span("t2", 0, 0, "方案可以继续推进", "a");
    left.timing_reliable = false;
    right.timing_reliable = false;
    let result = reconstruct_spans(&[left, right]);
    assert_eq!(result.utterances.len(), 1);
    assert!(result.boundaries[0]
        .reasons
        .contains(&BoundaryReason::UnreliableTiming));
}

#[test]
fn maximum_duration_remains_a_hard_safety_boundary() {
    let result = reconstruct_spans(&[
        span("t1", 0, 19_900, "一段很长的连续表达", "a"),
        span("t2", 20_000, 20_100, "继续", "a"),
    ]);
    assert_eq!(result.utterances.len(), 2);
    assert!(result.boundaries[0]
        .reasons
        .contains(&BoundaryReason::MaximumDuration));
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
fn mixed_or_overlap_is_not_force_assigned_but_uncertainty_does_not_force_a_boundary() {
    let mut mixed = span("t1", 0, 2_000, "无法可靠内部切分", "a");
    mixed.speaker_attribution = SpeakerAttribution::Mixed {
        speaker_keys: vec!["a".into(), "b".into()],
    };
    mixed.overlap = true;
    let result = reconstruct_spans(&[mixed, span("t2", 2_100, 3_000, "后续", "b")]);
    assert_eq!(result.utterances.len(), 1);
    assert_eq!(
        result.utterances[0].speaker_attribution,
        SpeakerAttribution::Unknown
    );
    assert!(result.utterances[0].mixed);
    assert!(result.utterances[0].overlap);
    assert_eq!(result.utterances[0].text, "无法可靠内部切分后续");
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
    let spans = normalize_timeline(
        &[transcript],
        &turns,
        &[],
        &UtteranceReconstructionConfig::default(),
    );
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
    let spans = normalize_timeline(
        &[transcript],
        &turns,
        &[],
        &UtteranceReconstructionConfig::default(),
    );
    assert!(matches!(
        spans[0].speaker_attribution,
        SpeakerAttribution::Mixed { .. }
    ));
}

#[test]
fn manual_assignment_wins_over_sequential_turn_handoff_but_not_true_overlap() {
    let mut transcript = raw_transcript("t1", 10.0, 18.0, "人工确认内容");
    transcript.speaker_id = Some("manual-speaker".into());
    transcript.speaker_assignment_method = "manual".into();
    let turns = vec![turn(10_000, 14_000, "a"), turn(14_000, 18_000, "b")];
    let spans = normalize_timeline(
        &[transcript.clone()],
        &turns,
        &[],
        &UtteranceReconstructionConfig::default(),
    );
    assert_eq!(
        spans[0].speaker_attribution,
        SpeakerAttribution::Single {
            speaker_key: "manual-speaker".into()
        }
    );
    assert_eq!(
        spans[0].speaker_attribution_source,
        SpeakerAttributionSource::Manual
    );
    assert_eq!(spans[0].speaker_assignment_reliability, None);

    transcript.speaker_overlap = 1;
    let spans = normalize_timeline(
        &[transcript],
        &turns,
        &[],
        &UtteranceReconstructionConfig::default(),
    );
    assert!(matches!(
        spans[0].speaker_attribution,
        SpeakerAttribution::Mixed { .. }
    ));
}

#[test]
fn chunk_reliability_is_recomputed_and_stale_persisted_confidence_is_ignored() {
    let mut transcript = raw_transcript("t1", 10.0, 12.0, "只有一半有说话人证据");
    transcript.speaker_confidence = Some(0.99);
    let spans = normalize_timeline(
        &[transcript],
        &[turn(10_000, 11_000, "a")],
        &[],
        &UtteranceReconstructionConfig::default(),
    );
    assert_eq!(
        spans[0].speaker_attribution_source,
        SpeakerAttributionSource::ChunkTemporalOverlap
    );
    assert_eq!(spans[0].speaker_assignment_reliability, None);
}

#[test]
fn persisted_speaker_without_accepted_turn_is_not_a_reliable_handoff() {
    let transcript = raw_transcript("t1", 10.0, 12.0, "回退说话人");
    let spans = normalize_timeline(
        &[transcript],
        &[],
        &[],
        &UtteranceReconstructionConfig::default(),
    );
    assert_eq!(
        spans[0].speaker_attribution_source,
        SpeakerAttributionSource::PersistedFallback
    );
    assert_eq!(spans[0].speaker_assignment_reliability, None);
}

#[test]
fn absent_asr_confidence_remains_none() {
    let mut item = span("t1", 0, 1_000, "没有分数", "a");
    item.asr_confidence = None;
    let result = reconstruct_spans(&[item]);
    assert_eq!(result.utterances[0].mean_asr_confidence, None);
}

#[test]
fn one_raw_chunk_can_resolve_an_a_to_b_handoff_with_source_ranges() {
    let mut transcript = raw_transcript("t1", 10.0, 18.0, "我觉得这个方案可以但是我不同意");
    set_timing(
        &mut transcript,
        &[
            ("我", 1_000),
            ("觉", 1_500),
            ("得", 2_000),
            ("这", 2_500),
            ("个", 3_000),
            ("方", 3_300),
            ("案", 3_600),
            ("可", 3_700),
            ("以", 3_750),
            ("但", 4_500),
            ("是", 4_800),
            ("我", 5_200),
            ("不", 5_800),
            ("同", 6_200),
            ("意", 6_600),
        ],
    );
    let turns = vec![turn(10_000, 13_800, "a"), turn(14_100, 18_000, "b")];
    let result = reconstruct_v3(&[transcript], &turns);
    assert_eq!(result.algorithm_version, ALGORITHM_VERSION_V3);
    assert_eq!(result.utterances.len(), 2);
    assert_eq!(result.utterances[0].text, "我觉得这个方案可以");
    assert_eq!(result.utterances[1].text, "但是我不同意");
    assert_eq!(result.utterances[0].source_transcript_ids, ["t1"]);
    assert_eq!(result.utterances[1].source_transcript_ids, ["t1"]);
    assert_ne!(result.utterances[0].id, result.utterances[1].id);
    assert!(!result.utterances[0].source_ranges.is_empty());
    assert_eq!(result.metrics.resolved_cross_speaker_chunk_count, 1);
}

#[test]
fn timed_single_speaker_chunk_is_single_and_deterministic() {
    let mut transcript = raw_transcript("t1", 10.0, 12.0, "单人发言");
    set_timing(
        &mut transcript,
        &[("单", 300), ("人", 600), ("发", 900), ("言", 1_200)],
    );
    let turns = vec![turn(10_000, 12_000, "a")];
    let first = reconstruct_v3(&[transcript.clone()], &turns);
    let second = reconstruct_v3(&[transcript], &turns);
    assert_eq!(first, second);
    assert_eq!(
        first.utterances[0].speaker_attribution,
        SpeakerAttribution::Single {
            speaker_key: "a".into()
        }
    );
    assert!(first.config_hash.starts_with("sha256:"));
    assert_eq!(
        first.profile.timing_mode,
        ReconstructionTimingMode::NativeLexicalTiming
    );
    assert_eq!(
        first.profile.boundary_policy,
        BoundaryPolicy::V3SemanticBaseline
    );
    assert_eq!(first.config_version, super::config::CONFIG_VERSION);
}

#[test]
fn sequential_52_48_temporal_evidence_stays_ambiguous() {
    let mut transcript = raw_transcript("t1", 10.0, 12.0, "嗯");
    set_timing(&mut transcript, &[("嗯", 1_000)]);
    let turns = vec![turn(10_000, 11_002, "a"), turn(11_002, 12_000, "b")];
    let result = reconstruct_v3(&[transcript], &turns);
    assert_eq!(
        result.alignment_diagnostics[0].status,
        WordSpeakerStatus::Ambiguous
    );
    assert_eq!(
        result.utterances[0].speaker_attribution,
        SpeakerAttribution::Unknown
    );
}

#[test]
fn simultaneous_turns_stay_mixed() {
    let mut transcript = raw_transcript("t1", 10.0, 12.0, "重叠");
    transcript.speaker_overlap = 1;
    set_timing(&mut transcript, &[("重", 500), ("叠", 1_000)]);
    let turns = vec![turn(10_000, 12_000, "a"), turn(10_000, 12_000, "b")];
    let result = reconstruct_v3(&[transcript], &turns);
    assert!(result
        .alignment_diagnostics
        .iter()
        .all(|item| item.status == WordSpeakerStatus::Mixed));
    assert!(matches!(
        result.utterances[0].speaker_attribution,
        SpeakerAttribution::Mixed { .. }
    ));
}

#[test]
fn thirty_millisecond_turn_jitter_is_not_called_true_overlap() {
    let mut transcript = raw_transcript("t1", 10.0, 12.0, "边界");
    set_timing(&mut transcript, &[("边", 1_000), ("界", 1_050)]);
    let turns = vec![turn(10_000, 11_065, "a"), turn(11_035, 12_000, "b")];
    let result = reconstruct_v3(&[transcript], &turns);
    assert_ne!(
        result.alignment_diagnostics[1].status,
        WordSpeakerStatus::Mixed
    );
}

#[test]
fn invalid_timing_uses_v3_chunk_fallback_profile() {
    let mut transcript = raw_transcript("t1", 10.0, 12.0, "先后");
    set_timing(&mut transcript, &[("先", 1_000), ("后", 500)]);
    let turns = vec![turn(10_000, 12_000, "a")];
    let v1 = normalize_timeline(
        &[transcript.clone()],
        &turns,
        &[],
        &UtteranceReconstructionConfig::default(),
    );
    let enhanced = super::timing::enhance_timeline(
        &[transcript.clone()],
        &turns,
        &v1,
        &UtteranceReconstructionConfig::default(),
    );
    assert_eq!(enhanced.valid_timing_chunks, 0);
    assert_eq!(enhanced.spans, v1);
    assert_eq!(enhanced.metrics.chunk_fallback_count, 1);
    let fallback = reconstruct_v3_with_config(
        "meeting-test",
        &[transcript],
        &turns,
        &[],
        &UtteranceReconstructionConfig::default(),
    );
    assert_eq!(fallback.algorithm_version, ALGORITHM_VERSION_V3);
    assert_eq!(
        fallback.profile.timing_mode,
        ReconstructionTimingMode::ChunkFallback
    );
    assert_eq!(
        fallback.profile.boundary_policy,
        BoundaryPolicy::V3SemanticBaseline
    );
    assert_eq!(fallback.profile.alignment_version, None);
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
        asr_timing_json: None,
    }
}

fn turn(start_ms: i64, end_ms: i64, speaker: &str) -> SpeakerTurn {
    SpeakerTurn {
        start_ms,
        end_ms,
        speaker_label: speaker.into(),
        confidence: Some(0.9),
        speaker_key: speaker.into(),
    }
}

fn set_timing(transcript: &mut Transcript, pieces: &[(&str, i64)]) {
    let timing = TranscriptTiming {
        provider: "test".into(),
        capabilities: TranscriptionCapabilities {
            token_timestamps: true,
            ..TranscriptionCapabilities::default()
        },
        tokens: pieces
            .iter()
            .map(|(text, start_ms)| TimedToken {
                text: (*text).into(),
                start_ms: *start_ms,
                end_ms: None,
                confidence: None,
                timing_source: TimingSource::NativeTokenEmission,
            })
            .collect(),
    };
    transcript.asr_timing_json = Some(serde_json::to_string(&timing).unwrap());
}

fn reconstruct_v3(transcripts: &[Transcript], turns: &[SpeakerTurn]) -> ReconstructionResult {
    let config = UtteranceReconstructionConfig::default();
    let chunk_spans = normalize_timeline(transcripts, turns, &[], &config);
    let enhanced = super::timing::enhance_timeline(transcripts, turns, &chunk_spans, &config);
    assert!(enhanced.valid_timing_chunks > 0);
    super::assembler::reconstruct_with_details(
        "meeting-test",
        &enhanced.spans,
        &config,
        ReconstructionProfile {
            algorithm_version: ALGORITHM_VERSION_V3.into(),
            timing_mode: ReconstructionTimingMode::NativeLexicalTiming,
            boundary_policy: BoundaryPolicy::V3SemanticBaseline,
            boundary_policy_version: BOUNDARY_POLICY_VERSION_V3.into(),
            semantic_model_version: Some(SEMANTIC_MODEL_VERSION_V1.into()),
            alignment_version: Some(ALIGNMENT_VERSION_V1.into()),
        },
        enhanced.metrics,
        enhanced.alignment_diagnostics,
        enhanced.timing_diagnostics,
    )
}
