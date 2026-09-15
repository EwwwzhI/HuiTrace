use super::config::{CONTINUATION_PREFIXES, STRONG_TERMINAL_PUNCTUATION, WEAK_PUNCTUATION};
use super::types::SemanticBoundaryEvidence;

pub const SEMANTIC_BASELINE_VERSION: &str = "deterministic-semantic-baseline-v1";

/// Produces conservative, language-light semantic features without rewriting
/// ASR text. These are deliberately described as a baseline rather than NLP
/// understanding so a future local model can replace this function cleanly.
pub fn evaluate(left: &str, right: &str) -> SemanticBoundaryEvidence {
    let left = left.trim();
    let right = right.trim();
    SemanticBoundaryEvidence {
        baseline_version: SEMANTIC_BASELINE_VERSION.to_string(),
        left_completeness: completeness(left),
        cross_boundary_continuity: continuity(left, right),
    }
}

fn completeness(text: &str) -> Option<f32> {
    let last = text.chars().next_back()?;
    if STRONG_TERMINAL_PUNCTUATION.contains(&last) {
        return Some(0.95);
    }
    if WEAK_PUNCTUATION.contains(&last) {
        return Some(0.20);
    }

    // Length is weak evidence only. It avoids brittle phrase-specific rules
    // such as checking for one particular Chinese trailing word.
    let lexical_length = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .count();
    Some(match lexical_length {
        0..=6 => 0.25,
        7..=14 => 0.40,
        15..=28 => 0.55,
        _ => 0.65,
    })
}

fn continuity(left: &str, right: &str) -> Option<f32> {
    if left.is_empty() || right.is_empty() {
        return None;
    }
    if CONTINUATION_PREFIXES
        .iter()
        .any(|prefix| right.starts_with(prefix))
    {
        return Some(0.90);
    }
    let last = left.chars().next_back()?;
    if WEAK_PUNCTUATION.contains(&last) {
        return Some(0.85);
    }
    if STRONG_TERMINAL_PUNCTUATION.contains(&last) {
        return Some(0.10);
    }
    completeness(left).map(|value| if value <= 0.40 { 0.75 } else { 0.60 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_distinguishes_complete_and_continuing_contexts() {
        let continuing = evaluate("我觉得这个方案", "其实没有太大的问题");
        assert!(continuing.left_completeness.unwrap() < 0.5);
        assert!(continuing.cross_boundary_continuity.unwrap() > 0.8);

        let completed = evaluate("这个问题今天先讨论到这里。", "接下来我们看第二个问题。");
        assert!(completed.left_completeness.unwrap() > 0.9);
        assert!(completed.cross_boundary_continuity.unwrap() < 0.2);
    }
}
