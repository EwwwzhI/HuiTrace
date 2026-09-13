use app_lib::diarization::short_turn::{
    compute_candidate_recall, compute_duration_bucket_metrics, compute_metrics,
    CandidateRecallObservation, MeetingSpeakerPrototypeStore, ShortTurnCandidate,
    ShortTurnCandidateSource, ShortTurnEvaluationObservation, ShortTurnRefiner,
};
use app_lib::diarization::types::SegmentKind;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Fixture {
    name: String,
    candidate: ShortTurnCandidate,
    known_speakers: Vec<String>,
    expected_kind: SegmentKind,
    expected_speaker: Option<String>,
    #[serde(default)]
    bypass: bool,
}

#[test]
fn metadata_driven_short_turn_fixture_suite() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("fixtures/short_turn/cases.json"))
            .expect("valid fixtures");
    let refiner = ShortTurnRefiner::default();
    let mut observations = Vec::new();

    for fixture in fixtures {
        if fixture.bypass {
            assert!(
                !refiner.should_refine(fixture.candidate.duration_ms),
                "{} should bypass",
                fixture.name
            );
            continue;
        }
        let prototypes = MeetingSpeakerPrototypeStore::new(fixture.known_speakers.clone());
        let decision = refiner.refine(&fixture.candidate, &prototypes);
        assert_eq!(decision.kind, fixture.expected_kind, "{}", fixture.name);
        assert_eq!(
            decision.speaker_key, fixture.expected_speaker,
            "{}",
            fixture.name
        );
        assert!(!decision.allow_new_speaker, "{}", fixture.name);
        observations.push(ShortTurnEvaluationObservation {
            expected_kind: fixture.expected_kind,
            expected_speaker: fixture.expected_speaker,
            predicted_kind: decision.kind,
            predicted_speaker: decision.speaker_key,
            predicted_is_new_speaker: decision.allow_new_speaker,
            manual_override_violated: false,
        });
    }

    let metrics = compute_metrics(&observations);
    assert_eq!(metrics.short_speech_recall, 1.0);
    assert_eq!(metrics.noise_false_accept_rate, 0.0);
    assert_eq!(metrics.backchannel_recall, 1.0);
    assert_eq!(metrics.backchannel_precision, 1.0);
    assert_eq!(metrics.speaker_attribution_accuracy, 1.0);
    assert_eq!(metrics.false_new_speaker_rate, 0.0);
    assert_eq!(metrics.manual_override_violation_count, 0);
}

#[test]
fn duration_buckets_and_candidate_source_recall_are_reported_separately() {
    let observation = |duration| {
        (
            duration,
            ShortTurnEvaluationObservation {
                expected_kind: SegmentKind::Backchannel,
                expected_speaker: Some("speaker_02".into()),
                predicted_kind: SegmentKind::Backchannel,
                predicted_speaker: Some("speaker_02".into()),
                predicted_is_new_speaker: false,
                manual_override_violated: false,
            },
        )
    };
    let buckets = compute_duration_bucket_metrics(&[
        observation(200),
        observation(400),
        observation(650),
        observation(900),
        observation(1_300),
    ]);
    for label in [
        "100-300ms",
        "300-500ms",
        "500-800ms",
        "800-1200ms",
        "1200-1500ms-control",
    ] {
        assert_eq!(buckets[label].sample_count, 1, "{label}");
    }

    let recall = compute_candidate_recall(&[
        CandidateRecallObservation {
            expected_short_event: true,
            found_sources: vec![ShortTurnCandidateSource::Transcript],
        },
        CandidateRecallObservation {
            expected_short_event: true,
            found_sources: vec![
                ShortTurnCandidateSource::DiarizerTurn,
                ShortTurnCandidateSource::VadEvent,
            ],
        },
    ]);
    assert_eq!(recall.sample_count, 2);
    assert_eq!(recall.transcript, 0.5);
    assert_eq!(recall.diarizer_turn, 0.5);
    assert_eq!(recall.vad_event, 0.5);
    assert_eq!(recall.union, 1.0);
}
