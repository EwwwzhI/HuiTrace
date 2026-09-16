use crate::audio::transcription::{TimedToken, TranscriptTiming};
use crate::database::models::Transcript;
use crate::database::repositories::speaker_turn::SpeakerTurn;

use super::alignment::align_words;
use super::config::UtteranceReconstructionConfig;
use super::normalizer::normalize_whitespace;
use super::types::{
    AlignmentDiagnostic, AtomicSpan, ReconstructionMetrics, SourceLexicalRange, SpeakerAttribution,
    TimedWord, TimingDiagnostic, WordSpeakerAssignment, WordSpeakerStatus,
};

pub struct V3Timeline {
    pub spans: Vec<AtomicSpan>,
    pub metrics: ReconstructionMetrics,
    pub alignment_diagnostics: Vec<AlignmentDiagnostic>,
    pub timing_diagnostics: Vec<TimingDiagnostic>,
    pub valid_timing_chunks: usize,
}

pub fn enhance_timeline(
    transcripts: &[Transcript],
    turns: &[SpeakerTurn],
    chunk_spans: &[AtomicSpan],
    config: &UtteranceReconstructionConfig,
) -> V3Timeline {
    let mut output = V3Timeline {
        spans: Vec::new(),
        metrics: ReconstructionMetrics {
            total_chunks: transcripts.len(),
            ..ReconstructionMetrics::default()
        },
        alignment_diagnostics: Vec::new(),
        timing_diagnostics: Vec::new(),
        valid_timing_chunks: 0,
    };

    for (transcript, fallback) in transcripts.iter().zip(chunk_spans) {
        if matches!(
            fallback.speaker_attribution,
            SpeakerAttribution::Mixed { .. }
        ) && !fallback.overlap
        {
            output.metrics.cross_speaker_raw_chunk_count += 1;
        }
        let Some(encoded) = transcript.asr_timing_json.as_deref() else {
            output.spans.push(fallback.clone());
            output.metrics.chunk_fallback_count += 1;
            continue;
        };
        output.metrics.chunks_with_timing += 1;
        let timing: TranscriptTiming = match serde_json::from_str(encoded) {
            Ok(value) => value,
            Err(error) => {
                output.spans.push(fallback.clone());
                output.metrics.chunk_fallback_count += 1;
                output.timing_diagnostics.push(TimingDiagnostic {
                    source_transcript_id: transcript.id.clone(),
                    valid: false,
                    reason: format!("invalid_timing_json:{error}"),
                });
                continue;
            }
        };
        let words = match validate_and_group(transcript, &timing, config) {
            Ok(words) => words,
            Err(reason) => {
                if reason == "lexical_preservation_failed" {
                    output.metrics.lexical_preservation_failure_count += 1;
                }
                output.spans.push(fallback.clone());
                output.metrics.chunk_fallback_count += 1;
                output.timing_diagnostics.push(TimingDiagnostic {
                    source_transcript_id: transcript.id.clone(),
                    valid: false,
                    reason,
                });
                continue;
            }
        };
        output.valid_timing_chunks += 1;
        output.metrics.valid_timing_chunks += 1;
        output.timing_diagnostics.push(TimingDiagnostic {
            source_transcript_id: transcript.id.clone(),
            valid: true,
            reason: "provider_timing_validated".to_string(),
        });
        let chunk_turns = turns
            .iter()
            .filter(|turn| turn.end_ms > fallback.start_ms && turn.start_ms < fallback.end_ms)
            .cloned()
            .collect::<Vec<_>>();
        let manual_speaker = (transcript.speaker_assignment_method == "manual")
            .then_some(transcript.speaker_id.as_deref())
            .flatten();
        let assignments = align_words(
            &words,
            &chunk_turns,
            manual_speaker,
            fallback.overlap,
            config,
        );
        let spans = assignments_to_spans(fallback, &assignments);
        let rebuilt = spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>();
        if normalize_preservation(&rebuilt) != normalize_preservation(&transcript.transcript) {
            output.metrics.lexical_preservation_failure_count += 1;
            output.metrics.chunk_fallback_count += 1;
            output.metrics.valid_timing_chunks -= 1;
            output.valid_timing_chunks -= 1;
            output.spans.push(fallback.clone());
            if let Some(diagnostic) = output.timing_diagnostics.last_mut() {
                diagnostic.valid = false;
                diagnostic.reason = "atomic_span_lexical_preservation_failed".to_string();
            }
        } else {
            accumulate_assignments(&mut output, &assignments);
            if matches!(
                fallback.speaker_attribution,
                SpeakerAttribution::Mixed { .. }
            ) && !fallback.overlap
                && resolved_cross_speaker(&assignments)
            {
                output.metrics.resolved_cross_speaker_chunk_count += 1;
            }
            output.spans.extend(spans);
        }
    }
    finalize_rates(&mut output.metrics);
    output
}

pub fn validate_and_group(
    transcript: &Transcript,
    timing: &TranscriptTiming,
    config: &UtteranceReconstructionConfig,
) -> Result<Vec<TimedWord>, String> {
    let chunk_start_seconds = transcript
        .audio_start_time
        .ok_or("chunk_timing_unavailable")?;
    let chunk_end_seconds = transcript
        .audio_end_time
        .ok_or("chunk_timing_unavailable")?;
    if !chunk_start_seconds.is_finite() || !chunk_end_seconds.is_finite() {
        return Err("chunk_timing_not_finite".to_string());
    }
    let chunk_start = seconds_to_ms(chunk_start_seconds).ok_or("chunk_timing_out_of_range")?;
    let chunk_end = seconds_to_ms(chunk_end_seconds).ok_or("chunk_timing_out_of_range")?;
    if chunk_end < chunk_start {
        return Err("chunk_end_before_start".to_string());
    }
    let chunk_duration = chunk_end - chunk_start;
    if timing.tokens.is_empty() {
        return Err("timing_has_no_tokens".to_string());
    }
    if !timing.capabilities.token_timestamps && !timing.capabilities.word_timestamps {
        return Err("provider_has_no_lexical_timing_capability".to_string());
    }
    if timing.tokens.iter().any(|token| {
        token.timing_source == crate::audio::transcription::TimingSource::NativeSegment
    }) {
        return Err("segment_timing_is_not_lexical_timing".to_string());
    }
    validate_tokens(
        &timing.tokens,
        chunk_duration,
        config.timing_bounds_tolerance_ms,
    )?;
    let words = group_tokens(&transcript.id, chunk_start, &timing.tokens);
    if words.is_empty() {
        return Err("timing_has_no_lexical_units".to_string());
    }
    let reconstructed = words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<String>();
    if normalize_preservation(&reconstructed) != normalize_preservation(&transcript.transcript) {
        return Err("lexical_preservation_failed".to_string());
    }
    Ok(words)
}

fn validate_tokens(
    tokens: &[TimedToken],
    chunk_duration: i64,
    tolerance: i64,
) -> Result<(), String> {
    let tolerance = tolerance.max(0);
    let mut previous_start = None;
    let mut previous_end = None;
    for token in tokens {
        if token.start_ms < 0 {
            return Err("negative_token_timestamp".to_string());
        }
        if token.start_ms > chunk_duration.saturating_add(tolerance) {
            return Err("token_timestamp_outside_chunk".to_string());
        }
        if let Some(end) = token.end_ms {
            if end < token.start_ms {
                return Err("token_end_before_start".to_string());
            }
            if end > chunk_duration.saturating_add(tolerance) {
                return Err("token_end_outside_chunk".to_string());
            }
            if previous_end.is_some_and(|previous| end < previous) {
                return Err("token_end_not_monotonic".to_string());
            }
            previous_end = Some(end);
        }
        if previous_start.is_some_and(|previous| token.start_ms < previous) {
            return Err("token_start_not_monotonic".to_string());
        }
        previous_start = Some(token.start_ms);
    }
    Ok(())
}

fn group_tokens(source_chunk_id: &str, chunk_start: i64, tokens: &[TimedToken]) -> Vec<TimedWord> {
    let mut words: Vec<TimedWord> = Vec::new();
    let mut pending_prefix = String::new();
    let mut pending_start_index = 0;
    for (index, token) in tokens.iter().enumerate() {
        if token.text.is_empty() {
            continue;
        }
        if token
            .text
            .chars()
            .all(|character| !character.is_alphanumeric())
        {
            if let Some(previous) = words.last_mut() {
                previous.text.push_str(&token.text);
                previous.token_end_index = index + 1;
                previous.end_ms = previous
                    .end_ms
                    .max(chunk_start + token.end_ms.unwrap_or(token.start_ms));
            } else {
                if pending_prefix.is_empty() {
                    pending_start_index = index;
                }
                pending_prefix.push_str(&token.text);
            }
            continue;
        }
        let first = token
            .text
            .chars()
            .find(|character| character.is_alphanumeric());
        let can_merge = pending_prefix.is_empty()
            && !token.text.chars().next().is_some_and(char::is_whitespace)
            && words.last().is_some_and(|previous| {
                previous
                    .text
                    .chars()
                    .rev()
                    .find(|character| character.is_alphanumeric())
                    .zip(first)
                    .is_some_and(|(left, right)| {
                        left.is_ascii_alphanumeric() && right.is_ascii_alphanumeric()
                    })
            });
        let absolute_start = chunk_start + token.start_ms;
        let absolute_end = chunk_start + token.end_ms.unwrap_or(token.start_ms);
        if can_merge {
            let previous = words.last_mut().expect("checked above");
            previous.text.push_str(&token.text);
            previous.end_ms = previous.end_ms.max(absolute_end).max(absolute_start);
            previous.token_end_index = index + 1;
            continue;
        }
        let lexical_index = words.len();
        let token_start_index = if pending_prefix.is_empty() {
            index
        } else {
            pending_start_index
        };
        words.push(TimedWord {
            id: format!("{source_chunk_id}:lexical:{lexical_index}"),
            text: format!("{pending_prefix}{}", token.text),
            start_ms: absolute_start,
            end_ms: absolute_end.max(absolute_start),
            confidence: token.confidence,
            source_chunk_id: source_chunk_id.to_string(),
            lexical_index,
            token_start_index,
            token_end_index: index + 1,
            timing_source: token.timing_source,
        });
        pending_prefix.clear();
    }
    if !pending_prefix.is_empty() {
        if let Some(previous) = words.last_mut() {
            previous.text.push_str(&pending_prefix);
            previous.token_end_index = tokens.len();
        }
    }
    words
}

fn assignments_to_spans(
    fallback: &AtomicSpan,
    assignments: &[WordSpeakerAssignment],
) -> Vec<AtomicSpan> {
    let mut spans: Vec<AtomicSpan> = Vec::new();
    for assignment in assignments {
        let attribution = attribution_for(assignment);
        if let Some(previous) = spans
            .last_mut()
            .filter(|span| span.speaker_attribution == attribution)
        {
            previous.text.push_str(&assignment.word.text);
            previous.end_ms = previous.end_ms.max(assignment.word.end_ms);
            if let Some(range) = previous.lexical_range.as_mut() {
                range.lexical_end_index = assignment.word.lexical_index + 1;
                range.token_end_index = assignment.word.token_end_index;
            }
            continue;
        }
        spans.push(AtomicSpan {
            start_ms: assignment.word.start_ms,
            end_ms: assignment.word.end_ms,
            timing_reliable: true,
            text: assignment.word.text.clone(),
            speaker_attribution: attribution,
            source_transcript_ids: fallback.source_transcript_ids.clone(),
            asr_confidence: fallback.asr_confidence,
            speaker_assignment_reliability: assignment.assignment_reliability,
            speaker_attribution_source: assignment.attribution_source,
            overlap: matches!(assignment.status, WordSpeakerStatus::Mixed),
            segment_kind: fallback.segment_kind.clone(),
            short_turn_confidence: fallback.short_turn_confidence,
            lexical_range: Some(SourceLexicalRange {
                source_transcript_id: assignment.word.source_chunk_id.clone(),
                lexical_start_index: assignment.word.lexical_index,
                lexical_end_index: assignment.word.lexical_index + 1,
                token_start_index: assignment.word.token_start_index,
                token_end_index: assignment.word.token_end_index,
            }),
        });
    }
    spans
}

fn attribution_for(assignment: &WordSpeakerAssignment) -> SpeakerAttribution {
    match assignment.status {
        WordSpeakerStatus::Assigned => assignment
            .speaker_key
            .clone()
            .map(|speaker_key| SpeakerAttribution::Single { speaker_key })
            .unwrap_or(SpeakerAttribution::Unknown),
        WordSpeakerStatus::Mixed => SpeakerAttribution::Mixed {
            speaker_keys: assignment
                .candidates
                .iter()
                .map(|candidate| candidate.speaker_key.clone())
                .collect(),
        },
        WordSpeakerStatus::Ambiguous | WordSpeakerStatus::Unknown => SpeakerAttribution::Unknown,
    }
}

fn accumulate_assignments(output: &mut V3Timeline, assignments: &[WordSpeakerAssignment]) {
    for assignment in assignments {
        output.metrics.total_lexical_units += 1;
        match assignment.status {
            WordSpeakerStatus::Assigned => output.metrics.assigned_lexical_units += 1,
            WordSpeakerStatus::Ambiguous => output.metrics.ambiguous_count += 1,
            WordSpeakerStatus::Mixed => output.metrics.mixed_count += 1,
            WordSpeakerStatus::Unknown => {}
        }
        output.alignment_diagnostics.push(AlignmentDiagnostic {
            source_transcript_id: assignment.word.source_chunk_id.clone(),
            word_id: assignment.word.id.clone(),
            text: assignment.word.text.clone(),
            start_ms: assignment.word.start_ms,
            end_ms: assignment.word.end_ms,
            timing_source: assignment.word.timing_source,
            candidates: assignment.candidates.clone(),
            status: assignment.status,
            speaker_key: assignment.speaker_key.clone(),
            best_overlap_ratio: assignment.best_overlap_ratio,
            reasons: assignment.reasons.clone(),
        });
    }
}

fn resolved_cross_speaker(assignments: &[WordSpeakerAssignment]) -> bool {
    let speakers = assignments
        .iter()
        .filter(|assignment| assignment.status == WordSpeakerStatus::Assigned)
        .filter_map(|assignment| assignment.speaker_key.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    speakers.len() > 1
}

fn finalize_rates(metrics: &mut ReconstructionMetrics) {
    metrics.timing_coverage = ratio(metrics.chunks_with_timing, metrics.total_chunks);
    metrics.valid_timing_rate = ratio(metrics.valid_timing_chunks, metrics.chunks_with_timing);
    metrics.word_assignment_coverage =
        ratio(metrics.assigned_lexical_units, metrics.total_lexical_units);
    metrics.ambiguous_rate = ratio(metrics.ambiguous_count, metrics.total_lexical_units);
    metrics.mixed_rate = ratio(metrics.mixed_count, metrics.total_lexical_units);
    metrics.resolved_cross_speaker_chunk_rate = ratio(
        metrics.resolved_cross_speaker_chunk_count,
        metrics.cross_speaker_raw_chunk_count,
    );
    metrics.chunk_fallback_rate = ratio(metrics.chunk_fallback_count, metrics.total_chunks);
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn normalize_preservation(text: &str) -> String {
    normalize_whitespace(text).trim().to_string()
}

fn seconds_to_ms(seconds: f64) -> Option<i64> {
    let milliseconds = seconds * 1_000.0;
    (milliseconds.is_finite() && milliseconds >= i64::MIN as f64 && milliseconds <= i64::MAX as f64)
        .then(|| milliseconds.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::transcription::{TimingSource, TranscriptionCapabilities};

    fn transcript(text: &str) -> Transcript {
        Transcript {
            id: "t1".into(),
            meeting_id: "m1".into(),
            transcript: text.into(),
            timestamp: "00:00:00".into(),
            summary: None,
            action_items: None,
            key_points: None,
            audio_start_time: Some(10.0),
            audio_end_time: Some(12.0),
            duration: Some(2.0),
            asr_confidence: None,
            speaker_id: None,
            speaker_confidence: None,
            speaker_provisional: 0,
            speaker_revision: 0,
            segment_kind: Some("speech".into()),
            audio_source: Some("mixed".into()),
            speaker_assignment_method: "diarization".into(),
            speaker_overlap: 0,
            asr_timing_json: None,
        }
    }

    fn timing(parts: &[(&str, i64)]) -> TranscriptTiming {
        TranscriptTiming {
            provider: "test".into(),
            capabilities: TranscriptionCapabilities {
                token_timestamps: true,
                ..TranscriptionCapabilities::default()
            },
            tokens: parts
                .iter()
                .map(|(text, start_ms)| TimedToken {
                    text: (*text).into(),
                    start_ms: *start_ms,
                    end_ms: None,
                    confidence: None,
                    timing_source: TimingSource::NativeTokenEmission,
                })
                .collect(),
        }
    }

    #[test]
    fn preserves_chinese_english_and_mixed_lexical_content() {
        for (text, parts) in [
            (
                "这个方案",
                vec![("这", 100), ("个", 200), ("方", 300), ("案", 400)],
            ),
            ("the module", vec![("the", 100), (" module", 300)]),
            (
                "这个 module 的 latency",
                vec![
                    ("这", 100),
                    ("个", 200),
                    (" module", 300),
                    (" 的", 500),
                    (" latency", 700),
                ],
            ),
        ] {
            let words = validate_and_group(
                &transcript(text),
                &timing(&parts),
                &UtteranceReconstructionConfig::default(),
            )
            .unwrap();
            assert_eq!(
                words
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<String>(),
                text
            );
        }
    }

    #[test]
    fn rejects_non_monotonic_timing() {
        let result = validate_and_group(
            &transcript("ab"),
            &timing(&[("a", 200), ("b", 100)]),
            &UtteranceReconstructionConfig::default(),
        );
        assert_eq!(result.unwrap_err(), "token_start_not_monotonic");
    }
}
