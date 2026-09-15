use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UtteranceReconstructionConfig {
    pub long_silence_ms: i64,
    pub medium_gap_ms: i64,
    pub short_gap_ms: i64,
    pub max_utterance_duration_ms: i64,
    pub max_text_length: usize,
    pub split_score_threshold: i32,
    pub speaker_changed_score: i32,
    pub ambiguous_speaker_changed_score: i32,
    pub reliable_speaker_change_confidence: f64,
    pub medium_gap_score: i32,
    pub terminal_punctuation_score: i32,
    pub same_speaker_score: i32,
    pub short_gap_score: i32,
    pub continuation_prefix_score: i32,
    pub semantic_boundary_enabled: bool,
    pub completeness_split_threshold: f32,
    pub continuity_merge_threshold: f32,
    pub completeness_split_score: i32,
    pub incomplete_merge_score: i32,
    pub continuity_merge_score: i32,
    pub weak_punctuation_score: i32,
    pub backchannel_bridge_score: i32,
    pub backchannel_min_confidence: f64,
    pub backchannel_max_duration_ms: i64,
    pub backchannel_max_side_gap_ms: i64,
    pub assignment_min_overlap_ratio: f64,
    pub assignment_min_margin: f64,
    pub alignment_tolerance_ms: i64,
    pub max_alignment_tolerance_ms: i64,
    pub timing_bounds_tolerance_ms: i64,
    pub true_overlap_min_ms: i64,
}

impl Default for UtteranceReconstructionConfig {
    fn default() -> Self {
        Self {
            long_silence_ms: 1_500,
            medium_gap_ms: 800,
            short_gap_ms: 350,
            max_utterance_duration_ms: 20_000,
            max_text_length: 90,
            split_score_threshold: 3,
            speaker_changed_score: 4,
            ambiguous_speaker_changed_score: 1,
            reliable_speaker_change_confidence: 0.75,
            medium_gap_score: 2,
            terminal_punctuation_score: 2,
            same_speaker_score: -3,
            short_gap_score: -2,
            continuation_prefix_score: -1,
            semantic_boundary_enabled: true,
            completeness_split_threshold: 0.80,
            continuity_merge_threshold: 0.70,
            completeness_split_score: 2,
            incomplete_merge_score: -1,
            continuity_merge_score: -3,
            weak_punctuation_score: -1,
            backchannel_bridge_score: -2,
            backchannel_min_confidence: 0.75,
            backchannel_max_duration_ms: 1_200,
            backchannel_max_side_gap_ms: 700,
            assignment_min_overlap_ratio: 0.60,
            assignment_min_margin: 0.20,
            alignment_tolerance_ms: 50,
            max_alignment_tolerance_ms: 200,
            timing_bounds_tolerance_ms: 200,
            true_overlap_min_ms: 100,
        }
    }
}

impl UtteranceReconstructionConfig {
    pub fn effective_alignment_tolerance_ms(&self) -> i64 {
        self.alignment_tolerance_ms
            .clamp(0, self.max_alignment_tolerance_ms.max(0))
    }
}

pub const CONFIG_VERSION: &str = "utterance-reconstruction-config-v3";

pub const CONTINUATION_PREFIXES: &[&str] = &[
    "但是",
    "然后",
    "所以",
    "而且",
    "其实",
    "就是",
    "因为",
    "不过",
    "另外",
    "同时",
    "比如",
    "也就是说",
    "换句话说",
    "尤其是",
    "然后呢",
];

pub const STRONG_TERMINAL_PUNCTUATION: &[char] = &['。', '！', '？', '.', '!', '?'];
pub const WEAK_PUNCTUATION: &[char] = &['，', '、', '；', '：', ',', ';', ':'];
