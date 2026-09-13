use app_lib::diarization::short_turn::{
    compute_metrics, MeetingSpeakerPrototypeStore, ShortTurnCandidate,
    ShortTurnEvaluationObservation, ShortTurnRefiner,
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
