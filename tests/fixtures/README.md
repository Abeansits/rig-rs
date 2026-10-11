# Parity fixtures

Golden fixtures captured from OpenRig (TypeScript, v0.6.9 @ `bed4d45`), the behaviour reference.

Layout: `tests/fixtures/<slice>/<case>.json` where `<slice>` is `sNN-<name>` (e.g. `s04-wake-ladder`).
Each file is one case:

```json
{
  "schema": 1,
  "source": { "ts_test": "packages/daemon/test/queue-wake-ladder.test.ts", "case": "escalates after 3 nudges", "openrig_commit": "bed4d45" },
  "input":  { "...": "serialized arguments / request / capture text" },
  "expect": { "...": "serialized result / response / emitted events" },
  "redact": ["$.expect.ts", "$.expect.id"]
}
```

Rules:
- Never hand-edit `expect`. Regenerate with `scripts/regen-fixtures.sh <slice>` and review the diff.
- Volatile values (ULIDs, timestamps, paths under a temp `RIG_HOME`) are redacted at capture time to `"[ts]"`, `"[id]"`, `"[home]"`.
- Raw terminal captures (claude/codex screens) go in `tests/fixtures/<slice>/captures/*.txt` with a sibling `.json` for the expected classification.
