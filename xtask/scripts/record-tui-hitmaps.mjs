// Records real OpenRig v0.6.9 TUI hit maps by calling renderScreen.
// node --import tsx record-tui-hitmaps.mjs <openrig-dir> <out-dir>
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const root = process.argv[2];
const outDir = process.argv[3];
if (!root || !outDir) {
  console.error("usage: record-tui-hitmaps.mjs <openrig-dir> <out-dir>");
  process.exit(2);
}

const stateMod = await import(pathToFileURL(join(root, "packages/tui/src/state.ts")).href);
const demoMod = await import(pathToFileURL(join(root, "packages/tui/src/demo-data.ts")).href);
const renderMod = await import(pathToFileURL(join(root, "packages/tui/src/render.ts")).href);
const { createViewState } = stateMod;
const { demoSnapshot } = demoMod;
const { renderScreen } = renderMod;

const SHIFT =
  "Coordinates are ratatui widget rects. ADR-0004 does not require cell parity with the TypeScript renderer, so hits are compared by action kind and target.";

function specOf(snap, action) {
  if (action?.type !== "cross" || action.kind !== "spec-of") return null;
  for (const host of snap.hosts ?? []) {
    for (const rig of host.rigs ?? []) {
      for (const pod of rig.pods ?? []) {
        const agent = pod.agents?.find((item) => item.name === action.name);
        if (agent?.spec) return agent.spec;
      }
    }
  }
  return null;
}

function identity(action, snap) {
  if (!action || typeof action.type !== "string") return null;
  if (action.type === "toggle-expand") return { kind: "toggle", target: action.key };
  if (action.type === "drill") return { kind: "select", target: `${action.resource}:${action.name}` };
  if (action.type === "cross") {
    const spec = specOf(snap, action);
    return { kind: "cross", target: spec ?? action.name };
  }
  if (action.type === "attention-open") return { kind: "attention", target: action.id };
  return null;
}

function screenOf(name, state, snap) {
  const cols = 120;
  const rows = 40;
  const painted = renderScreen(state, snap, { cols, rows, nowMs: 0 });
  const hits = painted.hitMap.map((hit) => {
    const id = identity(hit.action, snap);
    return {
      y: hit.y,
      x1: hit.x1,
      x2: hit.x2,
      raw: hit.action,
      identity: id,
    };
  });
  return {
    screen: name,
    source: "openrig v0.6.9",
    cols,
    rows,
    shift_reason: SHIFT,
    hits,
  };
}

function viewFor(snap) {
  return createViewState({ instanceId: "record", getSnapshot: () => snap });
}

const demo = demoSnapshot();
const explorer = viewFor(demo);
explorer.dispatch({ type: "toggle-expand", key: "pod:vm-host/openrig-build/dev50" });

const detail = viewFor(demo);
detail.dispatch({ type: "toggle-expand", key: "pod:vm-host/openrig-build/dev50" });
detail.dispatch({
  type: "drill",
  resource: "agent",
  name: "dev50.driver",
  target: { host: "vm-host", rig: "openrig-build", pod: "dev50" },
});
const detailProbe = renderScreen(detail.get(), demo, { cols: 120, rows: 40, nowMs: 0 });
detail.dispatch({
  type: "layout",
  contentMaxOffset: detailProbe.contentMaxOffset,
  contentTargetCount: detailProbe.contentTargets.length,
});
detail.dispatch({ type: "content-scroll", delta: detailProbe.contentMaxOffset });

const feed = {
  ...demo,
  attentionRead: {
    scope: "instance",
    readAt: "2026-08-05T09:38:00.000Z",
    items: demo.attention.map((item) => ({
      id: `queue:${item.qitemId}`,
      kind: "action",
      summary: item.summary,
      unblocks: null,
      urgency: "urgent",
      at: item.tsUpdated,
      scope: "instance",
      project: null,
      source: `/api/queue/${item.qitemId}`,
      recipient: item.destinationSession,
    })),
    sources: [{ source: "queue", state: "available", detail: "demo attention feed" }],
    detail: null,
    detailError: null,
  },
};
const attention = viewFor(feed);
attention.dispatch({ type: "jump", section: "needs" });

const screens = [
  screenOf("explorer-expanded-pod", explorer.get(), demo),
  screenOf("seat-detail-spec-cross", detail.get(), demo),
  screenOf("attention-list", attention.get(), feed),
];

mkdirSync(outDir, { recursive: true });
for (const screen of screens) {
  const v1 = screen.hits.filter((hit) => hit.identity);
  if (v1.length === 0) {
    console.error(`${screen.screen} recorded no v1 hit`);
    process.exit(1);
  }
  const path = join(outDir, `${screen.screen}.json`);
  writeFileSync(path, `${JSON.stringify(screen, null, 2)}\n`);
  console.log(`${screen.screen} ${v1.length} v1 hits -> ${path}`);
}
