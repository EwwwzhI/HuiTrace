//! Synthetic plumbing check, never representative or human ground truth.
use std::{fs, process::Command};

use app_lib::evaluation::{
    annotation_workspace::{
        export_manifest, initialize_annotation_project_inner, load_workspace_inner, qa_workspace,
        save_workspace_inner, AnnotationEvent, AnnotationMode, InitializeProjectRequest,
        SpeakerMapEntry, WorkspaceLoadRequest, WorkspaceSaveRequest,
    },
    production_artifact::{write_artifact, MeetingProductionArtifact},
};

#[test]
fn synthetic_export_initialize_two_pass_qa_dataset_check_and_frozen_replay() {
    let root = tempfile::tempdir().unwrap();
    let controlled_media = tempfile::tempdir().unwrap();
    let meeting = "meeting-example";
    let directory = root.path().join(meeting);
    fs::create_dir_all(&directory).unwrap();
    let media = root.path().join("synthetic.wav");
    let mut wav = hound::WavWriter::create(
        &media,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..80_000 {
        wav.write_sample(0i16).unwrap();
    }
    wav.finalize().unwrap();
    let mut artifact: MeetingProductionArtifact = serde_json::from_str(include_str!(
        "../../../evaluation/short_turn_dataset/production_artifact.example.json"
    ))
    .unwrap();
    artifact.raw_diarizer_turns = serde_json::from_value(serde_json::json!([
        {"start_ms":0,"end_ms":2000,"speaker_key":"speaker_01","confidence":0.99,"overlap":false},
        {"start_ms":2000,"end_ms":4000,"speaker_key":"speaker_02","confidence":0.99,"overlap":false},
        {"start_ms":4100,"end_ms":5000,"speaker_key":"speaker_02","confidence":0.99,"overlap":false}
    ])).unwrap();
    artifact.transcripts[0].start_ms = 4100;
    artifact.transcripts[0].end_ms = 5000;
    artifact.vad_events[0].start_ms = 4100;
    artifact.vad_events[0].end_ms = 5000;
    let artifact_path = directory.join("production.json");
    write_artifact(&artifact_path, &artifact).unwrap();
    let artifact_before = fs::read(&artifact_path).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_short_turn_export"))
        .args(["--audio"])
        .arg(&media)
        .args(["--meeting-id", meeting, "--output"])
        .arg(&directory)
        .arg("--production-artifact")
        .arg(&artifact_path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut snapshot = initialize_annotation_project_inner(
        InitializeProjectRequest {
            dataset_dir: root.path().into(),
            meeting_id: meeting.into(),
            source_media_path: media,
            production_artifact_path: artifact_path.clone(),
        },
        controlled_media.path(),
    )
    .unwrap();
    snapshot.session.speaker_map = vec![
        SpeakerMapEntry {
            key: "gt_speaker_A".into(),
            description: "Synthetic A".into(),
        },
        SpeakerMapEntry {
            key: "gt_speaker_B".into(),
            description: "Synthetic B".into(),
        },
    ];
    snapshot.draft.events = [
        ("ref-a", 0, 2000, "ordinary_speech_control", "gt_speaker_A"),
        (
            "ref-b",
            2000,
            4000,
            "ordinary_speech_control",
            "gt_speaker_B",
        ),
        ("short-a", 4100, 5000, "backchannel", "gt_speaker_A"),
    ]
    .into_iter()
    .map(|(id, start, end, kind, speaker)| AnnotationEvent {
        event_id: id.into(),
        start_ms: start,
        end_ms: end,
        kind: kind.into(),
        speaker: Some(speaker.into()),
        overlap: false,
        speaker_handoff: false,
        embedded: false,
        annotation_uncertain: false,
        expected_materialized: None,
        notes: "Synthetic fixture; not human annotation".into(),
        annotation_status: "pending".into(),
    })
    .collect();
    let save = |snapshot: &app_lib::evaluation::annotation_workspace::WorkspaceSnapshot| {
        save_workspace_inner(WorkspaceSaveRequest {
            dataset_dir: root.path().into(),
            draft: snapshot.draft.clone(),
            session: snapshot.session.clone(),
        })
    };
    save(&snapshot).unwrap();
    for window in &snapshot.windows {
        snapshot
            .session
            .window_status
            .insert(window.window_id.clone(), "reviewed_blind".into());
    }
    assert!(save(&snapshot).is_err());
    for event in &mut snapshot.draft.events {
        event.annotation_status = "blind_confirmed".into();
    }
    save(&snapshot).unwrap();
    snapshot = load_workspace_inner(WorkspaceLoadRequest {
        dataset_dir: root.path().into(),
        meeting_id: meeting.into(),
        mode: AnnotationMode::Review,
    })
    .unwrap();
    assert!(snapshot.review_evidence.is_some());
    assert!(export_manifest(root.path(), &snapshot.draft, &snapshot.session).is_err());
    for window in &snapshot.windows {
        snapshot
            .session
            .window_status
            .insert(window.window_id.clone(), "reviewed_second_pass".into());
    }
    for event in &mut snapshot.draft.events {
        event.annotation_status = "reviewed".into();
    }
    save(&snapshot).unwrap();
    assert!(
        qa_workspace(root.path(), &snapshot.draft, &snapshot.session)
            .unwrap()
            .errors
            .is_empty()
    );
    export_manifest(root.path(), &snapshot.draft, &snapshot.session).unwrap();
    let check = Command::new(env!("CARGO_BIN_EXE_short_turn_dataset_check"))
        .arg("--dataset")
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let replay = Command::new(env!("CARGO_BIN_EXE_short_turn_benchmark"))
        .arg("--dataset")
        .arg(root.path())
        .args(["--mode", "production-artifact-replay"])
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&replay.stdout).unwrap();
    assert_eq!(report["speaker_alignment"][meeting]["mapped_speakers"], 2);
    assert_eq!(
        report["speaker_alignment"][meeting]["reference_intervals"],
        2
    );
    assert_eq!(
        report["speaker_alignment"][meeting]["reference_duration_ms"],
        4000
    );
    assert_eq!(
        report["speaker_alignment"][meeting]["mapping"]["gt_speaker_A"],
        "speaker_01"
    );
    assert_eq!(fs::read(&artifact_path).unwrap(), artifact_before);
}
