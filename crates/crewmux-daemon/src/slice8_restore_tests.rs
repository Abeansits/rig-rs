//! Snapshot refusal, wave restore, gates, posture, restore-check, and crash-cart.

use std::sync::Arc;

use crewmux_core::{RigRepo, SeatRepo};
use crewmux_store::EdgeRow;
use crewmux_types::{LaunchPosture, RigName};
use serde_json::Value;

use crate::checkpoint::{CheckpointDraft, show_checkpoint, write_checkpoint};
use crate::slice8_harness::{
    kill_sessions, never_ready, restore, rig, seat, seat_field, send, spec, up,
};

#[tokio::test]
async fn corrupted_snapshot_is_refused() {
    let rig = rig().await;
    up(&rig.app, &spec("pair", &[("one", "/tmp")])).await;
    let created = crate::snapshot::create(&rig.app, "pair")
        .await
        .expect("snap");
    let id = created.get("id").and_then(Value::as_str).expect("id");
    let path = rig
        .home
        .join("snapshots")
        .join("pair")
        .join(id)
        .join("rig.db");
    let mut bytes = std::fs::read(&path).expect("bytes");
    let last = bytes.last_mut().expect("byte");
    *last ^= 0xff;
    std::fs::write(&path, bytes).expect("corrupt");
    let err = crate::restore::restore_rig(&rig.app, "pair", Some(id), LaunchPosture::Auto)
        .await
        .expect_err("refused");
    assert!(err.to_string().contains("corrupted"), "{err}");
}

#[tokio::test]
async fn newer_schema_snapshot_is_refused() {
    let rig = rig().await;
    up(&rig.app, &spec("pair", &[("one", "/tmp")])).await;
    let created = crate::snapshot::create(&rig.app, "pair")
        .await
        .expect("snap");
    let id = created
        .get("id")
        .and_then(Value::as_str)
        .expect("id")
        .to_owned();
    let path = rig
        .home
        .join("snapshots")
        .join("pair")
        .join(&id)
        .join("manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&path).expect("manifest")).expect("json");
    manifest["schema_version"] = serde_json::json!(99);
    std::fs::write(&path, serde_json::to_vec_pretty(&manifest).expect("bytes")).expect("write");
    let err = crate::snapshot::show(&rig.app, "pair", &id)
        .await
        .expect_err("newer");
    assert!(err.to_string().contains("newer"), "{err}");
}

#[tokio::test]
async fn three_seat_wave_restore_is_fully_restored() {
    let rig = rig().await;
    up(
        &rig.app,
        &spec(
            "trio",
            &[("mid", "/tmp"), ("alpha", "/tmp"), ("zeta", "/tmp")],
        ),
    )
    .await;
    edge(&rig.app, "trio", "alpha", "mid", "delegates_to").await;
    edge(&rig.app, "trio", "zeta", "mid", "spawned_by").await;
    send(&rig.app, "alpha@trio", "replay-alpha").await;
    let zeta = seat(&rig.app, "zeta").await;
    std::fs::remove_file(crate::resume::resume_path(&rig.app.home, &zeta.id)).expect("token");
    kill_sessions(&rig.app).await;
    let body = restore(&rig.app, "trio", LaunchPosture::Auto).await;
    never_ready(&body);
    assert_eq!(
        body.get("outcome").and_then(Value::as_str),
        Some("fully_restored")
    );
    let waves = body.get("waves").and_then(Value::as_array).expect("waves");
    let names: Vec<Vec<&str>> = waves
        .iter()
        .map(|wave| {
            wave.as_array()
                .expect("wave")
                .iter()
                .filter_map(Value::as_str)
                .collect()
        })
        .collect();
    assert_eq!(names, vec![vec!["alpha"], vec!["mid"], vec!["zeta"]]);
    assert_eq!(seat_field(&body, "alpha", "outcome"), "resumed");
    assert_eq!(seat_field(&body, "mid", "outcome"), "resumed");
    assert_eq!(seat_field(&body, "zeta", "outcome"), "fresh");
    let alpha = seat(&rig.app, "alpha").await;
    let replay =
        std::fs::read_to_string(crate::resume::replay_path(&rig.home, &alpha.id)).expect("replay");
    assert!(replay.contains("replay-alpha"), "{replay}");
    let zeta = seat(&rig.app, "zeta").await;
    assert!(!crate::resume::replay_path(&rig.home, &zeta.id).exists());
}

#[tokio::test]
async fn gate_restore_is_partial_with_a_noop_reason() {
    let rig = rig().await;
    up(
        &rig.app,
        &spec("gated", &[("gate-trust", "/tmp"), ("plain", "/tmp")]),
    )
    .await;
    kill_sessions(&rig.app).await;
    let body = restore(&rig.app, "gated", LaunchPosture::Auto).await;
    never_ready(&body);
    assert_eq!(
        body.get("outcome").and_then(Value::as_str),
        Some("partially_restored")
    );
    assert_eq!(
        seat_field(&body, "gate-trust", "outcome"),
        "attention_required"
    );
    assert_eq!(seat_field(&body, "gate-trust", "attention"), "trust_gate");
    assert_eq!(seat_field(&body, "gate-trust", "truth"), "pane_not_usable");
    assert_eq!(seat_field(&body, "plain", "outcome"), "resumed");
}

#[tokio::test]
async fn locked_restore_never_widens_posture() {
    let rig = rig().await;
    up(&rig.app, &spec("locked", &[("locked-one", "/tmp")])).await;
    kill_sessions(&rig.app).await;
    let body = restore(&rig.app, "locked", LaunchPosture::FullBypass).await;
    never_ready(&body);
    assert_eq!(seat_field(&body, "locked-one", "posture"), "locked");
    assert_eq!(seat_field(&body, "locked-one", "outcome"), "resumed");
}

#[tokio::test]
async fn restore_check_reports_dead_and_missing_resume() {
    let rig = rig().await;
    up(&rig.app, &spec("pair", &[("one", "/tmp"), ("two", "/tmp")])).await;
    let ready = crate::check_run::report(&rig.app).await.expect("check");
    assert_eq!(
        ready.pointer("/classes/ready").and_then(Value::as_u64),
        Some(2)
    );
    let two = seat(&rig.app, "two").await;
    let id = two.id.to_string();
    let store = Arc::clone(&rig.app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| {
            conn.execute(
                "UPDATE seat_restore SET resume_value = NULL, resume_kind = NULL WHERE seat_id = ?1",
                [id],
            )?;
            Ok(())
        })
    })
    .await
    .expect("join")
    .expect("clear");
    let caveats = crate::check_run::report(&rig.app).await.expect("check");
    assert_eq!(
        caveats.pointer("/classes/ready").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        caveats
            .pointer("/classes/ready_with_caveats")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert!(remediations_unsafe(&caveats));
    kill_sessions(&rig.app).await;
    let dead = crate::check_run::report(&rig.app).await.expect("check");
    assert_eq!(
        dead.pointer("/classes/not_ready").and_then(Value::as_u64),
        Some(2)
    );
    assert!(remediations_unsafe(&dead));
}

#[tokio::test]
async fn checkpoint_persists_artifacts_and_returns_the_newest() {
    let rig = rig().await;
    up(&rig.app, &spec("pair", &[("one", "/tmp"), ("two", "/tmp")])).await;
    let empty = show_checkpoint(&rig.app, "two").await.expect("empty");
    assert!(empty.get("checkpoint").is_some_and(Value::is_null));
    write_checkpoint(
        &rig.app,
        CheckpointDraft {
            seat: "one".to_owned(),
            summary: "first".to_owned(),
            current_task: Some("slice".to_owned()),
            next_step: Some("next".to_owned()),
            blocked_on: None,
            key_artifacts: vec!["notes.md".to_owned()],
            confidence: Some("high".to_owned()),
        },
    )
    .await
    .expect("write");
    let written = write_checkpoint(
        &rig.app,
        CheckpointDraft {
            seat: "one".to_owned(),
            summary: "second".to_owned(),
            current_task: None,
            next_step: None,
            blocked_on: None,
            key_artifacts: vec!["later.md".to_owned()],
            confidence: None,
        },
    )
    .await
    .expect("write");
    assert_eq!(
        written.pointer("/key_artifacts/0").and_then(Value::as_str),
        Some("later.md")
    );
    let shown = show_checkpoint(&rig.app, "one").await.expect("show");
    assert_eq!(shown.get("summary").and_then(Value::as_str), Some("second"));
}

#[tokio::test]
async fn crash_cart_orders_kernel_cancel_and_a_failed_rig() {
    let rig = rig().await;
    up(&rig.app, &spec("fleet", &[("oseat", "/tmp")])).await;
    up(&rig.app, &spec("kernel", &[("kseat", "/tmp")])).await;
    mark_kernel(&rig.app, "kernel").await;
    let cancelled = crate::cart::crash_cart(&rig.app, true).await.expect("cart");
    let rows = cancelled
        .get("rigs")
        .and_then(Value::as_array)
        .expect("rigs");
    assert_eq!(
        rows.first()
            .and_then(|row| row.get("rig"))
            .and_then(Value::as_str),
        Some("kernel")
    );
    assert_eq!(
        rows.first()
            .and_then(|row| row.get("kernel"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_ne!(
        rows.first()
            .and_then(|row| row.get("outcome"))
            .and_then(Value::as_str),
        Some("not_attempted")
    );
    assert_eq!(
        rows.get(1)
            .and_then(|row| row.get("outcome"))
            .and_then(Value::as_str),
        Some("not_attempted")
    );
    assert_eq!(
        cancelled.get("cancelled_before").and_then(Value::as_str),
        Some("fleet")
    );

    kill_sessions(&rig.app).await;
    break_cwd(&rig.app, "kernel").await;
    let continued = crate::cart::crash_cart(&rig.app, false)
        .await
        .expect("cart");
    let rows = continued
        .get("rigs")
        .and_then(Value::as_array)
        .expect("rigs");
    assert_eq!(rows.len(), 2);
    assert_eq!(row_outcome(rows, "kernel"), "partially_restored");
    assert_eq!(row_outcome(rows, "fleet"), "fully_restored");
}

#[tokio::test]
async fn restore_snapshot_brings_back_the_resume_token() {
    let rig = rig().await;
    up(&rig.app, &spec("pair", &[("one", "/tmp")])).await;
    let created = crate::snapshot::create(&rig.app, "pair")
        .await
        .expect("snap");
    let id = created
        .get("id")
        .and_then(Value::as_str)
        .expect("id")
        .to_owned();
    let one = seat(&rig.app, "one").await;
    let token = crate::resume::resume_path(&rig.home, &one.id);
    std::fs::remove_file(&token).expect("token");
    let seat_id = one.id.to_string();
    let store = Arc::clone(&rig.app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| {
            conn.execute(
                "DELETE FROM seat_restore WHERE seat_id = ?1",
                rusqlite::params![seat_id],
            )?;
            Ok(())
        })
    })
    .await
    .expect("join")
    .expect("delete");
    kill_sessions(&rig.app).await;
    let body = crate::restore::restore_rig(&rig.app, "pair", Some(&id), LaunchPosture::Auto)
        .await
        .expect("restore");
    assert_eq!(seat_field(&body, "one", "outcome"), "resumed");
    let text = std::fs::read_to_string(&token).expect("rewritten");
    assert!(text.starts_with("stub-"), "{text}");
}

#[tokio::test]
async fn list_omits_a_corrupt_snapshot() {
    let rig = rig().await;
    up(&rig.app, &spec("pair", &[("one", "/tmp")])).await;
    let created = crate::snapshot::create(&rig.app, "pair")
        .await
        .expect("snap");
    let id = created
        .get("id")
        .and_then(Value::as_str)
        .expect("id")
        .to_owned();
    let listed = crate::snapshot::list(&rig.app, "pair").await.expect("list");
    assert!(snapshot_ids(&listed).iter().any(|value| value == &id));
    let path = rig
        .home
        .join("snapshots")
        .join("pair")
        .join(&id)
        .join("rig.db");
    let mut bytes = std::fs::read(&path).expect("bytes");
    let last = bytes.last_mut().expect("byte");
    *last ^= 0xff;
    std::fs::write(&path, bytes).expect("corrupt");
    let listed = crate::snapshot::list(&rig.app, "pair").await.expect("list");
    assert!(
        snapshot_ids(&listed).iter().all(|value| value != &id),
        "{listed}"
    );
}

#[tokio::test]
async fn snapshot_ids_and_rig_names_reject_dot_dot() {
    let rig = rig().await;
    let rig_err = crate::snapshot::create(&rig.app, "../etc")
        .await
        .expect_err("rig");
    assert!(rig_err.to_string().contains("path segment"), "{rig_err}");
    let id_err = crate::snapshot::show(&rig.app, "pair", "..")
        .await
        .expect_err("id");
    assert!(id_err.to_string().contains("path segment"), "{id_err}");
    let restore_err =
        crate::restore::restore_rig(&rig.app, "pair", Some("../x"), LaunchPosture::Auto)
            .await
            .expect_err("restore");
    assert!(
        restore_err.to_string().contains("path segment"),
        "{restore_err}"
    );
}

#[tokio::test]
async fn cancel_raised_during_the_in_flight_rig_skips_later_rigs() {
    let rig = rig().await;
    up(&rig.app, &spec("fleet", &[("oseat", "/tmp")])).await;
    up(&rig.app, &spec("kernel", &[("kseat", "/tmp")])).await;
    mark_kernel(&rig.app, "kernel").await;
    rig.app.arm_cancel_on_restore();
    let cancelled = crate::cart::crash_cart(&rig.app, false)
        .await
        .expect("cart");
    let rows = cancelled
        .get("rigs")
        .and_then(Value::as_array)
        .expect("rigs");
    assert_eq!(
        rows.first()
            .and_then(|row| row.get("rig"))
            .and_then(Value::as_str),
        Some("kernel")
    );
    assert_ne!(
        rows.first()
            .and_then(|row| row.get("outcome"))
            .and_then(Value::as_str),
        Some("not_attempted")
    );
    assert_eq!(
        rows.get(1)
            .and_then(|row| row.get("outcome"))
            .and_then(Value::as_str),
        Some("not_attempted")
    );
    assert_eq!(
        cancelled.get("cancelled_before").and_then(Value::as_str),
        Some("fleet")
    );
}

fn snapshot_ids(body: &Value) -> Vec<String> {
    body.get("snapshots")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn row_outcome<'a>(rows: &'a [Value], name: &str) -> &'a str {
    rows.iter()
        .find(|row| row.get("rig").and_then(Value::as_str) == Some(name))
        .and_then(|row| row.get("outcome"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn remediations_unsafe(body: &Value) -> bool {
    body.get("remediations")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            !rows.is_empty()
                && rows
                    .iter()
                    .all(|row| row.get("execution_safe").and_then(Value::as_bool) == Some(false))
        })
}

async fn edge(app: &crate::state::App, rig_name: &str, source: &str, target: &str, kind: &str) {
    let (rig_id, _) = crate::snapshot::load_rig(app, rig_name).await.expect("rig");
    let source = seat(app, source).await;
    let target = seat(app, target).await;
    let edge = EdgeRow {
        source_id: source.id.to_string(),
        target_id: target.id.to_string(),
        kind: kind.to_owned(),
    };
    let rig_id = rig_id.to_string();
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| crewmux_store::put_edge(conn, &rig_id, &edge))
    })
    .await
    .expect("join")
    .expect("edge");
}

async fn mark_kernel(app: &crate::state::App, name: &str) {
    let (rig_id, _) = crate::snapshot::load_rig(app, name).await.expect("rig");
    let rig_id = rig_id.to_string();
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| crewmux_store::mark_kernel(conn, &rig_id))
    })
    .await
    .expect("join")
    .expect("kernel");
}

async fn break_cwd(app: &crate::state::App, name: &str) {
    let name = RigName::new(name).expect("name");
    crate::db::write(&app.store, move |conn| {
        let rig = conn
            .get_by_name(&name)?
            .ok_or_else(|| crewmux_core::Error::RigNotFound(name.to_string()))?;
        for mut seat in conn.list_by_rig(&rig.id)? {
            seat.cwd = Some(std::path::PathBuf::from("/no/such/rig-slice8-cwd"));
            SeatRepo::upsert(conn, &seat)?;
        }
        Ok(())
    })
    .await
    .expect("cwd");
}
