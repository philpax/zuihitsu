//! HTTP tests for the `/control/*` surface: agent creation and inspection, merge retraction and
//! primary designation, snapshotting, and self-edit.

use super::{loopback, test_state};
use crate::http_server::{AppState, router};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use std::sync::Arc;
use tower::ServiceExt;
use zuihitsu::{ManualClock, Server, time::Timestamp};

#[tokio::test]
async fn create_then_inspect_over_the_control_api() {
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(test_state(server));

    // Create the agent through the API.
    let seed = serde_json::json!({
        "agent_name": "Kestrel",
        "persona": "An assistant.",
        "seed_entries": [],
    });
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/control/agent")
                .header("content-type", "application/json")
                .body(Body::from(seed.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);

    // Genesis now reports Complete.
    let genesis = app
        .clone()
        .oneshot(
            Request::builder()
                .extension(loopback())
                .uri("/control/genesis")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(genesis.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&bytes[..], br#""Complete""#);

    // `self` exists; an unknown memory is a 404.
    let self_memory = app
        .clone()
        .oneshot(
            Request::builder()
                .extension(loopback())
                .uri("/control/memory?name=self")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(self_memory.status(), StatusCode::OK);

    let missing = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .uri("/control/memory?name=person/nobody")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unmerge_endpoint_retracts_a_merge_then_404s_when_nothing_to_retract() {
    use zuihitsu::{
        EventPayload, LinkPosture, LinkSource, MemoryId, Namespace, RelationName, Visibility,
    };

    let server = Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&zuihitsu::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let a = MemoryId::generate();
    let b = MemoryId::generate();
    server
        .control()
        .seed_events(vec![
            EventPayload::memory_created(a, Namespace::Person.with_name("marcus@direct")),
            EventPayload::memory_created(b, Namespace::Person.with_name("marcus@chat")),
            EventPayload::link_created(
                a,
                b,
                RelationName::SameAs,
                LinkPosture {
                    source: LinkSource::Operator,
                    told_by: None,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ),
        ])
        .unwrap();
    let app = router(test_state(Arc::new(server)));

    let post = |from: MemoryId, to: MemoryId| {
        let body = serde_json::json!({ "from": from.0.to_string(), "to": to.0.to_string() });
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/control/unmerge")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };

    // The first retraction removes the `same_as` edge.
    let removed = app.clone().oneshot(post(a, b)).await.unwrap();
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);

    // A second retraction finds nothing directly merged — 404.
    let again = app.oneshot(post(a, b)).await.unwrap();
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn designate_primary_endpoint_pins_a_stub_then_404s_on_an_unknown_memory() {
    use zuihitsu::{
        EventPayload, LinkPosture, LinkSource, MemoryId, Namespace, RelationName, Visibility,
    };

    let server = Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&zuihitsu::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let mut ids = [MemoryId::generate(), MemoryId::generate()];
    ids.sort();
    let (older, newer) = (ids[0], ids[1]);
    server
        .control()
        .seed_events(vec![
            EventPayload::memory_created(older, Namespace::Person.with_name("pat")),
            EventPayload::memory_created(newer, Namespace::Person.with_name("patricia")),
            EventPayload::link_created(
                older,
                newer,
                RelationName::SameAs,
                LinkPosture {
                    source: LinkSource::Operator,
                    told_by: None,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ),
        ])
        .unwrap();
    let app = router(test_state(Arc::new(server)));

    let post = |memory: MemoryId, designated: bool| {
        let body = serde_json::json!({ "memory": memory.0.to_string(), "designated": designated });
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/control/designate-primary")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };

    // Pinning the later-minted stub succeeds.
    let pinned = app.clone().oneshot(post(newer, true)).await.unwrap();
    assert_eq!(pinned.status(), StatusCode::NO_CONTENT);

    // A designation naming no live memory is a 404.
    let ghost = MemoryId::generate();
    let missing = app.oneshot(post(ghost, true)).await.unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn snapshot_endpoint_writes_a_file_or_409s_when_disabled() {
    let born = || {
        let server =
            Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
        server
            .control()
            .create_agent(&zuihitsu::SeedSelf {
                agent_name: "Kestrel".to_owned(),
                persona: "An assistant.".to_owned(),
                seed_entries: vec![],
            })
            .unwrap();
        server
    };
    let post = || {
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/control/snapshot")
            .body(Body::empty())
            .unwrap()
    };

    // Enabled: the endpoint writes a snapshot into the configured directory.
    let dir = std::env::temp_dir().join(format!(
        "zuihitsu-snapep-{}",
        zuihitsu::MemoryId::generate().0
    ));
    let app = router(AppState {
        snapshot_dir: Some(dir.clone()),
        ..test_state(Arc::new(born()))
    });
    let response = app.oneshot(post()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(zuihitsu::snapshot::latest(&dir).unwrap().is_some());
    std::fs::remove_dir_all(&dir).unwrap();

    // Disabled (no snapshot dir): the endpoint answers 409.
    let app = router(test_state(Arc::new(born())));
    let response = app.oneshot(post()).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn self_edit_endpoint_appends_revises_and_validates() {
    // A born agent whose `self` carries only its seeded persona entry.
    let server = Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&zuihitsu::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "A discreet companion with a long memory.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let app = router(test_state(Arc::new(server)));

    let edit = |body: serde_json::Value| {
        app.clone().oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/control/self")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
    };

    // An append returns 200 and the new entry id.
    let appended = edit(serde_json::json!({ "text": "I keep Marcus's memory." }))
        .await
        .unwrap();
    assert_eq!(appended.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(appended.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let appended_id = value["entry_id"].as_str().unwrap().to_owned();
    assert!(!appended_id.is_empty(), "the response names the new entry");

    // The seeded persona entry's id, read back through the entries endpoint, to revise it.
    let entries = app
        .clone()
        .oneshot(
            Request::builder()
                .extension(loopback())
                .uri("/control/entries?name=self")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(entries.into_body(), usize::MAX)
        .await
        .unwrap();
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
    let persona_id = entries[0]["entry_id"].as_str().unwrap().to_owned();

    // A revision (supersedes the persona entry) returns 200.
    let revised = edit(serde_json::json!({
        "text": "A discreet companion who keeps Marcus's memory.",
        "supersedes": persona_id,
    }))
    .await
    .unwrap();
    assert_eq!(revised.status(), StatusCode::OK);

    // An empty edit is a 400.
    let empty = edit(serde_json::json!({ "text": "   " })).await.unwrap();
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);

    // Superseding an unknown entry is a 404.
    let ghost = zuihitsu::EntryId::generate();
    let unknown = edit(serde_json::json!({ "text": "replacement", "supersedes": ghost }))
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
}
