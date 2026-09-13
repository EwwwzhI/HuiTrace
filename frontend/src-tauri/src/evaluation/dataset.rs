//! Shared representative-data gate and annotation QA.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::production_artifact::read_and_validate_artifact;

#[derive(Debug, Clone, Deserialize)]
pub struct DatasetRecord {
    #[serde(default, alias = "id")]
    pub ground_truth_event_id: String,
    #[serde(default)]
    pub record_type: String,
    #[serde(default)]
    pub recall_eligible: bool,
    #[serde(default)]
    pub evidence_origin: String,
    #[serde(default)]
    pub meeting_id: String,
    #[serde(default)]
    pub production_artifact_path: Option<PathBuf>,
    pub start_ms: i64,
    pub end_ms: i64,
    #[serde(default)]
    pub duration_bucket: String,
    pub ground_truth_kind: String,
    #[serde(default)]
    pub ground_truth_speaker: Option<String>,
    #[serde(default)]
    pub annotation_uncertain: bool,
    #[serde(default)]
    pub expected_visible_speakers: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

pub trait GateSample {
    fn event_id(&self) -> &str;
    fn meeting_id(&self) -> &str;
    fn start_ms(&self) -> i64;
    fn end_ms(&self) -> i64;
    fn kind_label(&self) -> &'static str;
    fn ground_truth_speaker(&self) -> Option<&str>;
    fn expected_visible_speakers(&self) -> &[String];
    fn annotation_uncertain(&self) -> bool;
    fn tags(&self) -> &[String];
}

impl GateSample for DatasetRecord {
    fn event_id(&self) -> &str {
        &self.ground_truth_event_id
    }
    fn meeting_id(&self) -> &str {
        &self.meeting_id
    }
    fn start_ms(&self) -> i64 {
        self.start_ms
    }
    fn end_ms(&self) -> i64 {
        self.end_ms
    }
    fn kind_label(&self) -> &'static str {
        match self.ground_truth_kind.as_str() {
            "short_speech" => "short_speech",
            "speech" if self.end_ms - self.start_ms <= 1_200 => "short_speech",
            "speech" => "ordinary_speech_control",
            "backchannel" => "backchannel",
            "noise" => "noise",
            "non_speech_vocalization" => "non_speech_vocalization",
            "ordinary_speech_control" => "ordinary_speech_control",
            _ => "invalid",
        }
    }
    fn ground_truth_speaker(&self) -> Option<&str> {
        self.ground_truth_speaker.as_deref()
    }
    fn expected_visible_speakers(&self) -> &[String] {
        &self.expected_visible_speakers
    }
    fn annotation_uncertain(&self) -> bool {
        self.annotation_uncertain
    }
    fn tags(&self) -> &[String] {
        &self.tags
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CoveragePolicy {
    pub min_scorable_samples: usize,
    pub min_true_short_events: usize,
    pub min_short_speech: usize,
    pub min_backchannel: usize,
    pub min_negative_controls: usize,
    pub min_per_short_duration_bucket: usize,
    pub min_meetings: usize,
    pub min_multi_speaker_meetings: usize,
    pub min_overlap_cases: usize,
    pub min_speaker_handoff_cases: usize,
}

impl Default for CoveragePolicy {
    fn default() -> Self {
        Self {
            min_scorable_samples: 100,
            min_true_short_events: 60,
            min_short_speech: 20,
            min_backchannel: 15,
            min_negative_controls: 20,
            min_per_short_duration_bucket: 10,
            min_meetings: 3,
            min_multi_speaker_meetings: 2,
            min_overlap_cases: 8,
            min_speaker_handoff_cases: 8,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CoverageSummary {
    pub scorable_samples: usize,
    pub uncertain_samples: usize,
    pub true_short_events: usize,
    pub short_speech: usize,
    pub backchannel: usize,
    pub negative_controls: usize,
    pub meeting_count: usize,
    pub multi_speaker_meeting_count: usize,
    pub overlap_cases: usize,
    pub speaker_handoff_cases: usize,
    pub duration_buckets: BTreeMap<String, usize>,
    pub missing_requirements: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateCandidate {
    pub meeting_id: String,
    pub first_event_id: String,
    pub second_event_id: String,
    pub overlap_iou: f64,
    pub center_distance_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatasetCheckReport {
    pub policy: CoveragePolicy,
    pub coverage: CoverageSummary,
    pub possible_duplicates: Vec<DuplicateCandidate>,
    pub representative_data_gate: &'static str,
}

pub fn duration_bucket(duration_ms: i64) -> &'static str {
    match duration_ms {
        100..=299 => "100-300ms",
        300..=499 => "300-500ms",
        500..=799 => "500-800ms",
        800..=1_200 => "800-1200ms",
        _ => "non_short_control",
    }
}

fn is_true_short<T: GateSample>(row: &T) -> bool {
    row.end_ms() - row.start_ms() <= 1_200
        && matches!(row.kind_label(), "short_speech" | "backchannel")
}

fn is_negative<T: GateSample>(row: &T) -> bool {
    matches!(row.kind_label(), "noise" | "non_speech_vocalization")
}

pub fn coverage<T: GateSample>(rows: &[T], policy: &CoveragePolicy) -> CoverageSummary {
    let scorable = rows
        .iter()
        .filter(|row| !row.annotation_uncertain())
        .collect::<Vec<_>>();
    let true_short = scorable
        .iter()
        .copied()
        .filter(|row| is_true_short(*row))
        .collect::<Vec<_>>();
    let mut speaker_sets: HashMap<&str, HashSet<&str>> = HashMap::new();
    for row in &scorable {
        if let Some(speaker) = row.ground_truth_speaker() {
            speaker_sets
                .entry(row.meeting_id())
                .or_default()
                .insert(speaker);
        }
        for speaker in row.expected_visible_speakers() {
            speaker_sets
                .entry(row.meeting_id())
                .or_default()
                .insert(speaker);
        }
    }
    let mut result = CoverageSummary {
        scorable_samples: scorable.len(),
        uncertain_samples: rows.len() - scorable.len(),
        true_short_events: true_short.len(),
        short_speech: true_short
            .iter()
            .filter(|row| row.kind_label() == "short_speech")
            .count(),
        backchannel: true_short
            .iter()
            .filter(|row| row.kind_label() == "backchannel")
            .count(),
        negative_controls: scorable.iter().filter(|row| is_negative(**row)).count(),
        meeting_count: scorable
            .iter()
            .map(|row| row.meeting_id())
            .collect::<HashSet<_>>()
            .len(),
        multi_speaker_meeting_count: speaker_sets.values().filter(|set| set.len() >= 2).count(),
        overlap_cases: scorable
            .iter()
            .filter(|row| row.tags().iter().any(|tag| tag == "overlap"))
            .count(),
        speaker_handoff_cases: scorable
            .iter()
            .filter(|row| row.tags().iter().any(|tag| tag == "speaker_handoff"))
            .count(),
        ..CoverageSummary::default()
    };
    for bucket in ["100-300ms", "300-500ms", "500-800ms", "800-1200ms"] {
        result.duration_buckets.insert(
            bucket.into(),
            true_short
                .iter()
                .filter(|row| duration_bucket(row.end_ms() - row.start_ms()) == bucket)
                .count(),
        );
    }
    let mut require = |actual: usize, minimum: usize, label: &str| {
        if actual < minimum {
            result
                .missing_requirements
                .push(format!("{label} +{}", minimum - actual));
        }
    };
    require(
        result.scorable_samples,
        policy.min_scorable_samples,
        "scorable_samples",
    );
    require(
        result.true_short_events,
        policy.min_true_short_events,
        "true_short_events",
    );
    require(result.short_speech, policy.min_short_speech, "short_speech");
    require(result.backchannel, policy.min_backchannel, "backchannel");
    require(
        result.negative_controls,
        policy.min_negative_controls,
        "negative_controls",
    );
    require(result.meeting_count, policy.min_meetings, "meetings");
    require(
        result.multi_speaker_meeting_count,
        policy.min_multi_speaker_meetings,
        "multi_speaker_meetings",
    );
    require(result.overlap_cases, policy.min_overlap_cases, "overlap");
    require(
        result.speaker_handoff_cases,
        policy.min_speaker_handoff_cases,
        "speaker_handoff",
    );
    for bucket in ["100-300ms", "300-500ms", "500-800ms", "800-1200ms"] {
        require(
            result.duration_buckets[bucket],
            policy.min_per_short_duration_bucket,
            bucket,
        );
    }
    result
}

fn overlap_iou(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> f64 {
    let overlap = (a_end.min(b_end) - a_start.max(b_start)).max(0) as f64;
    let union = (a_end.max(b_end) - a_start.min(b_start)).max(1) as f64;
    overlap / union
}

pub fn possible_duplicates<T: GateSample>(rows: &[T]) -> Vec<DuplicateCandidate> {
    let mut duplicates = Vec::new();
    for (index, left) in rows.iter().enumerate() {
        for right in rows.iter().skip(index + 1) {
            if left.meeting_id() != right.meeting_id()
                || left.kind_label() != right.kind_label()
                || left.kind_label() == "invalid"
            {
                continue;
            }
            let iou = overlap_iou(
                left.start_ms(),
                left.end_ms(),
                right.start_ms(),
                right.end_ms(),
            );
            let center_distance =
                ((left.start_ms() + left.end_ms()) - (right.start_ms() + right.end_ms())).abs() / 2;
            if iou >= 0.80 || (iou >= 0.60 && center_distance <= 75) {
                duplicates.push(DuplicateCandidate {
                    meeting_id: left.meeting_id().to_string(),
                    first_event_id: left.event_id().to_string(),
                    second_event_id: right.event_id().to_string(),
                    overlap_iou: iou,
                    center_distance_ms: center_distance,
                });
            }
        }
    }
    duplicates
}

pub fn validate_records(rows: &[DatasetRecord]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for row in rows {
        let label = if row.ground_truth_event_id.trim().is_empty() {
            "unnamed event"
        } else {
            &row.ground_truth_event_id
        };
        if !ids.insert(row.ground_truth_event_id.as_str()) {
            bail!("{label}: duplicate ground_truth_event_id");
        }
        if row.ground_truth_event_id.trim().is_empty()
            || row.meeting_id.trim().is_empty()
            || row.record_type != "ground_truth_event"
            || !row.recall_eligible
            || row.evidence_origin != "production_artifact"
            || row.production_artifact_path.is_none()
            || row.start_ms < 0
            || row.end_ms <= row.start_ms
            || row.duration_bucket != duration_bucket(row.end_ms - row.start_ms)
            || row.kind_label() == "invalid"
        {
            bail!("{label}: invalid benchmark-ready ground-truth semantics");
        }
    }
    Ok(())
}

pub fn read_dataset_records(dataset: &Path) -> Result<Vec<DatasetRecord>> {
    let path = dataset.join("manifest.jsonl");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let reader =
        BufReader::new(File::open(&path).with_context(|| format!("open {}", path.display()))?);
    reader
        .lines()
        .enumerate()
        .filter_map(|(index, line)| match line {
            Ok(line) if line.trim().is_empty() => None,
            Ok(line) => Some(
                serde_json::from_str(&line)
                    .with_context(|| format!("parse {} line {}", path.display(), index + 1)),
            ),
            Err(error) => Some(Err(error.into())),
        })
        .collect()
}

pub fn check_dataset(dataset: &Path) -> Result<DatasetCheckReport> {
    if !dataset.is_dir() {
        bail!("dataset directory does not exist: {}", dataset.display());
    }
    let rows = read_dataset_records(dataset)?;
    validate_records(&rows)?;
    let mut artifacts = HashMap::<&str, PathBuf>::new();
    for row in &rows {
        let path = row
            .production_artifact_path
            .as_ref()
            .expect("validated production artifact path");
        let path = if path.is_absolute() {
            path.clone()
        } else {
            dataset.join(path)
        };
        if let Some(existing) = artifacts.insert(&row.meeting_id, path.clone()) {
            if existing != path {
                bail!(
                    "meeting {} references multiple production artifacts",
                    row.meeting_id
                );
            }
        }
    }
    for (meeting_id, path) in artifacts {
        read_and_validate_artifact(&path, Some(meeting_id))?;
    }
    let policy = CoveragePolicy::default();
    let coverage = coverage(&rows, &policy);
    let possible_duplicates = possible_duplicates(&rows);
    let representative_data_gate =
        if coverage.missing_requirements.is_empty() && possible_duplicates.is_empty() {
            "REPRESENTATIVE_DATA_READY"
        } else {
            "INSUFFICIENT_REPRESENTATIVE_DATA"
        };
    Ok(DatasetCheckReport {
        policy,
        coverage,
        possible_duplicates,
        representative_data_gate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> DatasetRecord {
        DatasetRecord {
            ground_truth_event_id: id.into(),
            record_type: "ground_truth_event".into(),
            recall_eligible: true,
            evidence_origin: "production_artifact".into(),
            meeting_id: "meeting-1".into(),
            production_artifact_path: None,
            start_ms: 100,
            end_ms: 300,
            duration_bucket: "100-300ms".into(),
            ground_truth_kind: "short_speech".into(),
            ground_truth_speaker: Some("speaker_01".into()),
            annotation_uncertain: false,
            expected_visible_speakers: vec!["speaker_01".into()],
            tags: vec![],
        }
    }

    #[test]
    fn uncertain_samples_do_not_fill_the_gate() {
        let mut row = sample("uncertain");
        row.annotation_uncertain = true;
        let result = coverage(&[row], &CoveragePolicy::default());
        assert_eq!(result.scorable_samples, 0);
        assert_eq!(result.uncertain_samples, 1);
        assert_eq!(result.true_short_events, 0);
    }

    #[test]
    fn missing_quota_is_reported_as_a_delta() {
        let result = coverage(&[sample("one")], &CoveragePolicy::default());
        assert!(result
            .missing_requirements
            .contains(&"scorable_samples +99".to_string()));
        assert!(result
            .missing_requirements
            .contains(&"backchannel +15".to_string()));
    }

    #[test]
    fn duplicate_ground_truth_is_flagged_for_review() {
        let first = sample("one");
        let mut second = sample("two");
        second.start_ms += 20;
        second.end_ms += 20;
        let duplicates = possible_duplicates(&[first, second]);
        assert_eq!(duplicates.len(), 1);
        assert_eq!(duplicates[0].first_event_id, "one");
        assert_eq!(duplicates[0].second_event_id, "two");
    }
}
