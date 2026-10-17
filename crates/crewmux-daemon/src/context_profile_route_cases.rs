//! One case per `context-pack-profile-route.test.ts` title, except the synthetic world graph.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::context_route_tests::{Harness, WORLD, boot, call, sha256_hex, write_pack};

fn enc(text: &str) -> String {
    text.replace('/', "%2F").replace(':', "%3A")
}

fn profile_path(refer: &str, query: &str) -> String {
    format!(
        "/api/context-packs/library/by-ref/profile?ref={}&{query}",
        enc(refer)
    )
}

async fn profile(harness: &Harness, query: &str) -> (u16, Value) {
    call(
        &harness.sock,
        "GET",
        &profile_path("packs/world", query),
        None,
    )
    .await
}

fn message(body: &Value) -> String {
    body["message"].as_str().unwrap_or("").to_owned()
}

fn atom_ids(body: &Value) -> Vec<String> {
    body["pieces"]
        .as_array()
        .expect("pieces")
        .iter()
        .map(|piece| piece["atomId"].as_str().unwrap_or("").to_owned())
        .collect()
}

async fn seated() -> Harness {
    let harness = boot().await;
    let topology = harness.dir.join("topology");
    let seat = topology.join("rigs").join("r1").join("seats").join("s1");
    std::fs::create_dir_all(&seat).expect("seat");
    std::fs::write(
        seat.join("RECAP.md"),
        "## Recent Decisions\nwe chose X because Y",
    )
    .expect("recap");
    harness
        .app
        .context
        .set_grants(topology, harness.dir.join("missions"));
    write_pack(
        &harness.dir.join("context"),
        "packs/world",
        WORLD,
        &[("walk.md", "## Welcome\nhello world")],
    );
    harness
}

struct Work {
    workspace: PathBuf,
    mission: PathBuf,
    slice: PathBuf,
}

fn work_tree(root: &Path) -> Work {
    let workspace = root.join("workspace");
    let mission = workspace.join("missions").join("release-x");
    let slice = mission.join("slices").join("10-work-install");
    std::fs::create_dir_all(&slice).expect("slice");
    std::fs::write(
        workspace.join("SPEC.md"),
        "# Project\nProject intent sentinel",
    )
    .expect("project");
    std::fs::write(
        mission.join("SPEC.md"),
        "# Mission\nMission change sentinel",
    )
    .expect("mission");
    std::fs::write(slice.join("SPEC.md"), "# Slice\nExact outcome sentinel").expect("slice file");
    Work {
        workspace,
        mission,
        slice,
    }
}

fn grant_work(harness: &Harness, workspace: &Path) {
    harness
        .app
        .context
        .set_workspace(Some(workspace.to_path_buf()));
    harness
        .app
        .context
        .set_grants(harness.dir.join("topology"), workspace.join("missions"));
}

fn assert_relabeled_project(body: &Value) {
    let project: Vec<&Value> = body["pieces"]
        .as_array()
        .expect("pieces")
        .iter()
        .filter(|piece| piece["altitude"] == "project")
        .collect();
    let labeled: Vec<(&str, &str)> = project
        .iter()
        .map(|piece| {
            (
                piece["address"].as_str().unwrap_or(""),
                piece["source"].as_str().unwrap_or(""),
            )
        })
        .collect();
    assert_eq!(
        labeled,
        [
            ("project:SPEC.md", "manifest"),
            ("project:context/first.md", "manifest"),
            ("project:context/second.md", "manifest"),
        ]
    );
    let addressed: Vec<&str> = body["pieces"]
        .as_array()
        .expect("pieces")
        .iter()
        .filter(|piece| piece.get("altitude").is_some())
        .map(|piece| piece["address"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        addressed,
        [
            "project:SPEC.md",
            "project:context/first.md",
            "project:context/second.md",
            "mission:SPEC.md",
            "mission:slices/10-work-install/SPEC.md",
        ]
    );
    let texts: Vec<&str> = project
        .iter()
        .map(|piece| piece["text"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        texts,
        [
            "# Project\nProject intent sentinel",
            "# First\nFirst authored context",
            "# Second\nSecond authored context",
        ]
    );
    assert_eq!(
        body["pieces"]
            .as_array()
            .expect("pieces")
            .iter()
            .filter(|piece| piece["address"] == "project:SPEC.md")
            .count(),
        1
    );
}

fn work_shape(body: &Value) -> Vec<Value> {
    body["pieces"]
        .as_array()
        .expect("pieces")
        .iter()
        .filter(|piece| piece.get("altitude").is_some())
        .map(|piece| {
            json!({
                "atomId": piece["atomId"],
                "address": piece["address"],
                "altitude": piece["altitude"],
                "source": piece["source"],
                "text": piece["text"],
                "sha256": piece["sha256"],
            })
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn composes_handover_with_a_seat_tree_recap() {
    let harness = seated().await;
    let (status, body) =
        profile(&harness, "situation=handover&runtime=claude&rig=r1&seat=s1").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(atom_ids(&body), ["welcome", "recap"]);
    let pieces = body["pieces"].as_array().expect("pieces");
    let recap = pieces
        .iter()
        .find(|piece| piece["atomId"] == "recap")
        .expect("recap");
    let welcome = pieces
        .iter()
        .find(|piece| piece["atomId"] == "welcome")
        .expect("welcome");
    assert_eq!(recap["sourceKind"], "seat");
    assert!(
        recap["text"]
            .as_str()
            .unwrap_or("")
            .contains("we chose X because Y")
    );
    assert_eq!(welcome["sourceKind"], "library");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_piece_carries_its_sha256() {
    let harness = seated().await;
    let (status, body) =
        profile(&harness, "situation=handover&runtime=claude&rig=r1&seat=s1").await;
    assert_eq!(status, 200, "{body}");
    for piece in body["pieces"].as_array().expect("pieces") {
        let text = piece["text"].as_str().expect("text");
        assert_eq!(piece["sha256"], sha256_hex(text));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_composes_without_the_seat_tree() {
    let harness = seated().await;
    let (status, body) = profile(&harness, "situation=fresh&runtime=claude").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(atom_ids(&body), ["welcome"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_atom_without_rig_seat_fails_loud() {
    let harness = seated().await;
    let (status, body) = profile(&harness, "situation=handover&runtime=claude").await;
    assert_eq!(status, 422, "{body}");
    let text = message(&body).to_ascii_lowercase();
    assert!(text.contains("seat"));
    assert!(text.contains("root") || text.contains("config") || text.contains("rig"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn budget_overage_is_reported() {
    let harness = seated().await;
    let (status, body) = profile(
        &harness,
        "situation=handover&runtime=claude&rig=r1&seat=s1&budget=1",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["pieces"].as_array().expect("pieces").len(), 2);
    assert!(body["budget"]["overageTokens"].as_u64().unwrap_or(0) > 0);
    assert!(body["budget"]["dropCandidates"].as_array().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn symlinked_seat_read_is_refused() {
    let harness = seated().await;
    let outside = harness.dir.join("outside-secret.md");
    std::fs::write(&outside, "## Recent Decisions\nSECRET BYTES").expect("secret");
    let recap = harness
        .dir
        .join("topology")
        .join("rigs")
        .join("r1")
        .join("seats")
        .join("s1")
        .join("RECAP.md");
    std::fs::remove_file(&recap).expect("rm");
    std::os::unix::fs::symlink(&outside, &recap).expect("link");
    let (status, body) =
        profile(&harness, "situation=handover&runtime=claude&rig=r1&seat=s1").await;
    assert_eq!(status, 422, "{body}");
    assert!(message(&body).to_ascii_lowercase().contains("symlink"));
    assert!(!body.to_string().contains("SECRET BYTES"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grant_is_scoped_to_the_named_seat() {
    let harness = seated().await;
    let (denied, denied_body) = profile(&harness, "situation=handover&runtime=claude").await;
    assert_eq!(denied, 422, "{denied_body}");
    assert!(message(&denied_body).to_ascii_lowercase().contains("seat"));
    let (status, body) =
        profile(&harness, "situation=handover&runtime=claude&rig=r1&seat=s1").await;
    assert_eq!(status, 200, "{body}");
    let recap = body["pieces"]
        .as_array()
        .expect("pieces")
        .iter()
        .find(|piece| piece["atomId"] == "recap")
        .expect("recap");
    let real = recap["provenance"]["realPath"].as_str().unwrap_or("");
    assert!(real.contains("rigs/r1/seats/s1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dotdot_filename_is_not_flagged() {
    let harness = seated().await;
    let seat = harness
        .dir
        .join("topology")
        .join("rigs")
        .join("r1")
        .join("seats")
        .join("s1");
    std::fs::write(
        seat.join("..hidden-notes.md"),
        "## Recent Decisions\ninnocent bytes",
    )
    .expect("notes");
    let manifest = WORLD.replace(
        "seat:RECAP.md#recent-decisions",
        "seat:..hidden-notes.md#recent-decisions",
    );
    std::fs::write(
        harness
            .dir
            .join("context")
            .join("packs")
            .join("world")
            .join("manifest.yaml"),
        manifest,
    )
    .expect("manifest");
    let (status, body) =
        profile(&harness, "situation=handover&runtime=claude&rig=r1&seat=s1").await;
    assert_eq!(status, 200, "{body}");
    let recap = body["pieces"]
        .as_array()
        .expect("pieces")
        .iter()
        .find(|piece| piece["atomId"] == "recap")
        .expect("recap");
    assert_eq!(recap["provenance"]["escapesRoot"], false);
    assert_eq!(body["provenanceWarnings"], json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dangling_symlink_is_a_named_failure() {
    let harness = seated().await;
    let recap = harness
        .dir
        .join("topology")
        .join("rigs")
        .join("r1")
        .join("seats")
        .join("s1")
        .join("RECAP.md");
    std::fs::remove_file(&recap).expect("rm");
    std::os::unix::fs::symlink(harness.dir.join("never-existed.md"), &recap).expect("link");
    let (status, body) =
        profile(&harness, "situation=handover&runtime=claude&rig=r1&seat=s1").await;
    assert_eq!(status, 422, "{body}");
    let text = message(&body);
    let lower = text.to_ascii_lowercase();
    assert!(lower.contains("dangling") || lower.contains("symlink"));
    assert!(text.contains("RECAP.md"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provenance_warnings_is_always_an_array() {
    let harness = seated().await;
    let (status, body) =
        profile(&harness, "situation=handover&runtime=claude&rig=r1&seat=s1").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["provenanceWarnings"], json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mission_atom_resolves_from_slices_root() {
    let harness = seated().await;
    let missions = harness.dir.join("missions");
    let mission = missions.join("release-x");
    std::fs::create_dir_all(&mission).expect("mission");
    std::fs::write(mission.join("NOTES.md"), "## Watch Items\nW-99 lives here").expect("notes");
    harness
        .app
        .context
        .set_grants(harness.dir.join("topology"), missions);
    let manifest = format!(
        "{WORLD}  - id: watch\n    address: \"mission:NOTES.md#watch-items\"\n    taxonomy: mission\n    situations: [handover]\n    purpose: width\n    order: 50\n    priority: recommended\n"
    );
    std::fs::write(
        harness
            .dir
            .join("context")
            .join("packs")
            .join("world")
            .join("manifest.yaml"),
        manifest,
    )
    .expect("manifest");
    let (status, body) = profile(
        &harness,
        "situation=handover&runtime=claude&rig=r1&seat=s1&mission=release-x",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let watch = body["pieces"]
        .as_array()
        .expect("pieces")
        .iter()
        .find(|piece| piece["atomId"] == "watch")
        .expect("watch");
    assert_eq!(watch["sourceKind"], "mission");
    assert!(
        watch["text"]
            .as_str()
            .unwrap_or("")
            .contains("W-99 lives here")
    );
    let (denied, denied_body) =
        profile(&harness, "situation=handover&runtime=claude&rig=r1&seat=s1").await;
    assert_eq!(denied, 422, "{denied_body}");
    assert!(
        message(&denied_body)
            .to_ascii_lowercase()
            .contains("mission")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_mission_slice_adds_the_spec_walk() {
    let harness = seated().await;
    let work = work_tree(&harness.dir);
    grant_work(&harness, &work.workspace);
    let (status, body) = profile(
        &harness,
        "situation=fresh&runtime=codex&mission=release-x&slice=10-work-install",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let pieces = body["pieces"].as_array().expect("pieces");
    let defaults: Vec<&Value> = pieces
        .iter()
        .filter(|piece| piece["source"] == "default")
        .collect();
    assert_eq!(
        defaults
            .iter()
            .map(|piece| {
                (
                    piece["altitude"].as_str().unwrap_or(""),
                    piece["address"].as_str().unwrap_or(""),
                    piece["sourceKind"].as_str().unwrap_or(""),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            ("project", "project:SPEC.md", "project"),
            ("mission", "mission:SPEC.md", "mission"),
            ("slice", "mission:slices/10-work-install/SPEC.md", "mission"),
        ]
    );
    let texts: Vec<&str> = defaults
        .iter()
        .map(|piece| piece["text"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        texts,
        [
            "# Project\nProject intent sentinel",
            "# Mission\nMission change sentinel",
            "# Slice\nExact outcome sentinel",
        ]
    );
    let paths: Vec<&str> = defaults
        .iter()
        .map(|piece| piece["provenance"]["nominalPath"].as_str().unwrap_or(""))
        .collect();
    let expected = [
        work.workspace.join("SPEC.md").display().to_string(),
        work.mission.join("SPEC.md").display().to_string(),
        work.slice.join("SPEC.md").display().to_string(),
    ];
    assert_eq!(
        paths,
        expected.iter().map(String::as_str).collect::<Vec<_>>()
    );
    let altitude = pieces
        .get(pieces.len().saturating_sub(3))
        .and_then(|piece| piece["altitude"].as_str());
    assert_eq!(altitude, Some("project"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_manifest_relabels_intent_and_appends_context() {
    let harness = seated().await;
    let work = work_tree(&harness.dir);
    std::fs::create_dir_all(work.workspace.join("context")).expect("context");
    std::fs::write(
        work.workspace.join("context").join("first.md"),
        "# First\nFirst authored context",
    )
    .expect("first");
    std::fs::write(
        work.workspace.join("context").join("second.md"),
        "# Second\nSecond authored context",
    )
    .expect("second");
    std::fs::write(
        work.workspace.join("project.yaml"),
        "schema: openrig.project/v0alpha1\nkind: project\ninstall:\n  intent: SPEC.md\n  context:\n    - context/first.md\n    - context/second.md\nmissions:\n  root: missions\n",
    )
    .expect("project");
    grant_work(&harness, &work.workspace);
    let (status, body) = profile(
        &harness,
        "situation=fresh&runtime=codex&mission=release-x&slice=10-work-install",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_relabeled_project(&body);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_manifest_replaces_the_intent_address() {
    let harness = seated().await;
    let work = work_tree(&harness.dir);
    std::fs::write(
        work.workspace.join("SPEC.md"),
        "# Project\nConventional intent must not compose",
    )
    .expect("spec");
    std::fs::create_dir_all(work.workspace.join("context")).expect("context");
    std::fs::write(
        work.workspace.join("context").join("project-intent.md"),
        "# Project\nAuthored project intent",
    )
    .expect("intent");
    std::fs::write(
        work.workspace.join("project.yaml"),
        "schema: openrig.project/v0alpha1\nkind: project\ninstall:\n  intent: context/project-intent.md\nmissions:\n  root: missions\n",
    )
    .expect("project");
    grant_work(&harness, &work.workspace);
    let (status, body) = profile(
        &harness,
        "situation=fresh&runtime=codex&mission=release-x&slice=10-work-install",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let project: Vec<&Value> = body["pieces"]
        .as_array()
        .expect("pieces")
        .iter()
        .filter(|piece| piece["altitude"] == "project")
        .collect();
    assert_eq!(project.len(), 1);
    assert_eq!(project[0]["address"], "project:context/project-intent.md");
    assert_eq!(project[0]["source"], "manifest");
    assert_eq!(project[0]["text"], "# Project\nAuthored project intent");
    assert!(
        !body["pieces"]
            .as_array()
            .expect("pieces")
            .iter()
            .any(|piece| piece["address"] == "project:SPEC.md")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn optional_project_intent_warns_and_preserves_baseline() {
    let intents = ["42", "../outside.md", "\"SPEC.md#\""];
    for intent in intents {
        let harness = seated().await;
        let work = work_tree(&harness.dir);
        grant_work(&harness, &work.workspace);
        std::fs::write(
            work.workspace.join("project.yaml"),
            "schema: openrig.project/v0alpha1\nkind: project\nmissions:\n  root: missions\n",
        )
        .expect("baseline");
        let (base_status, baseline) = profile(
            &harness,
            "situation=fresh&runtime=codex&mission=release-x&slice=10-work-install",
        )
        .await;
        assert_eq!(base_status, 200, "{baseline}");
        std::fs::write(
            work.workspace.join("project.yaml"),
            format!(
                "schema: openrig.project/v0alpha1\nkind: project\ninstall:\n  intent: {intent}\nmissions:\n  root: missions\n"
            ),
        )
        .expect("invalid");
        let (status, body) = profile(
            &harness,
            "situation=fresh&runtime=codex&mission=release-x&slice=10-work-install",
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(work_shape(&body), work_shape(&baseline));
        assert_eq!(
            body["warnings"],
            json!([
                "project.yaml: optional install.intent must be a relative Markdown address; ignored the invalid value and kept the baseline work install."
            ])
        );
        assert_eq!(body["provenanceWarnings"], json!([]));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn optional_project_context_warns_and_preserves_baseline() {
    let contexts = [
        "not-a-list",
        "\n    - context/first.md\n    - ../outside.md",
        "\n    - context/first.md\n    - \"SPEC.md#a/b/c\"",
    ];
    for context in contexts {
        let harness = seated().await;
        let work = work_tree(&harness.dir);
        std::fs::create_dir_all(work.workspace.join("context")).expect("context");
        std::fs::write(
            work.workspace.join("context").join("first.md"),
            "# First\nValid context must not compose from a mixed list",
        )
        .expect("first");
        grant_work(&harness, &work.workspace);
        std::fs::write(
            work.workspace.join("project.yaml"),
            "schema: openrig.project/v0alpha1\nkind: project\nmissions:\n  root: missions\n",
        )
        .expect("baseline");
        let (base_status, baseline) = profile(
            &harness,
            "situation=fresh&runtime=codex&mission=release-x&slice=10-work-install",
        )
        .await;
        assert_eq!(base_status, 200, "{baseline}");
        std::fs::write(
            work.workspace.join("project.yaml"),
            format!(
                "schema: openrig.project/v0alpha1\nkind: project\ninstall:\n  context: {context}\nmissions:\n  root: missions\n"
            ),
        )
        .expect("invalid");
        let (status, body) = profile(
            &harness,
            "situation=fresh&runtime=codex&mission=release-x&slice=10-work-install",
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(work_shape(&body), work_shape(&baseline));
        assert_eq!(
            body["warnings"],
            json!([
                "project.yaml: optional install.context must be a list of relative Markdown addresses; ignored the invalid value and kept the baseline work install."
            ])
        );
        assert_eq!(body["provenanceWarnings"], json!([]));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_default_without_slice_refuses() {
    let harness = seated().await;
    let workspace = harness.dir.join("workspace");
    std::fs::create_dir_all(workspace.join("missions").join("release-x")).expect("mission");
    grant_work(&harness, &workspace);
    let (status, body) = profile(&harness, "situation=fresh&runtime=codex&mission=release-x").await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], "slice_required");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_inputs_are_named_4xx() {
    let harness = seated().await;
    let (bad, body) = profile(&harness, "situation=someday&runtime=claude").await;
    assert_eq!(bad, 400, "{body}");
    assert!(message(&body).contains("situation"));
    write_pack(
        &harness.dir.join("context"),
        "packs/bare",
        "name: bare\nversion: \"1\"\ntaxonomy: world\nfiles:\n  - { path: a.md, role: x }\n",
        &[("a.md", "## A\nbody")],
    );
    let (synced, _) = call(
        &harness.sock,
        "POST",
        "/api/context-packs/library/sync",
        Some(b"{}".to_vec()),
    )
    .await;
    assert_eq!(synced, 200);
    let (bare, bare_body) = call(
        &harness.sock,
        "GET",
        &profile_path("packs/bare", "situation=fresh&runtime=claude"),
        None,
    )
    .await;
    assert_eq!(bare, 422, "{bare_body}");
    assert!(message(&bare_body).contains("atoms"));
    let (missing, _) = call(
        &harness.sock,
        "GET",
        &profile_path("packs/ghost", "situation=fresh&runtime=claude"),
        None,
    )
    .await;
    assert_eq!(missing, 404);
}
