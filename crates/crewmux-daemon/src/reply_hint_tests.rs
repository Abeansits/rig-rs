//! A seat id in `from` is rewritten to `seat@rig` on the From line and the reply hint.

use crewmux_core::SeatRepo;
use serde_json::Value;

use crate::db::read;
use crate::deliver::{Outbound, deliver};
use crate::flow::harness;
use crate::launch::bring_up;
use crate::query::capture;

#[tokio::test]
async fn reply_hint_uses_the_seat_address() {
    let (app, _mux) = harness(None).await;
    let spec = "name: dev-pair-stub\nseats:\n  - name: dev-impl\n    runtime: stub\n    cwd: /tmp\n  - name: dev-qa\n    runtime: stub\n    cwd: /tmp\n";
    bring_up(&app, spec).await.expect("up");
    let seats = read(&app.store, |conn| conn.list_all())
        .await
        .expect("seats");
    let seat = seats
        .iter()
        .find(|seat| seat.name.as_str() == "dev-impl")
        .expect("impl");
    let ulid = seat.id.to_string();
    let message = Outbound {
        from: ulid.clone(),
        targets: vec!["dev-qa@dev-pair-stub".to_owned()],
        rig: None,
        all: false,
        body: "hint-body".to_owned(),
        interact: false,
        reason: None,
        verify: false,
        track_direct: true,
    };
    deliver(&app, message).await.expect("send");
    let text = screen(&app).await;
    assert!(text.contains("From: dev-impl@dev-pair-stub"), "{text}");
    assert!(!text.contains(&format!("From: {ulid}")), "{text}");
    assert!(
        text.contains("↩ Reply: rig send dev-impl@dev-pair-stub \"...\""),
        "{text}"
    );
    assert!(!text.contains(&ulid), "{text}");
    deliver(
        &app,
        Outbound {
            from: "operator@host".to_owned(),
            targets: vec!["dev-qa@dev-pair-stub".to_owned()],
            rig: None,
            all: false,
            body: "operator-body".to_owned(),
            interact: false,
            reason: None,
            verify: false,
            track_direct: true,
        },
    )
    .await
    .expect("operator");
    let text = screen(&app).await;
    assert!(text.contains("From: operator@host"), "{text}");
    assert!(
        text.contains("↩ Reply: rig send operator@host \"...\""),
        "{text}"
    );
}

async fn screen(app: &crate::state::App) -> String {
    let shot = capture(app, "dev-qa", None).await.expect("capture");
    shot.get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}
