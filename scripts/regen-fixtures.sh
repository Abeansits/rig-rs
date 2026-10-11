#!/usr/bin/env bash
# Regenerate golden fixtures for one slice from the OpenRig TS implementation.
# Usage: scripts/regen-fixtures.sh <slice-dir-name>   e.g. s04-wake-ladder
# Needs: OPENRIG_DIR pointing at an openrig checkout at the pinned commit, node >= 20, npm.
set -euo pipefail
slice="${1:?usage: regen-fixtures.sh <slice>}"
: "${OPENRIG_DIR:?set OPENRIG_DIR to an openrig checkout}"
pinned="bed4d45"
here="$(cd "$(dirname "$0")/.." && pwd)"
capture="$here/tests/parity/capture/$slice.capture.test.ts"
[ -f "$capture" ] || { echo "no capture script: $capture" >&2; exit 2; }
head="$(git -C "$OPENRIG_DIR" rev-parse --short HEAD)"
[ "$head" = "$pinned" ] || { echo "openrig at $head, expected $pinned" >&2; exit 2; }
out="$here/tests/fixtures/$slice"
mkdir -p "$out"
( cd "$OPENRIG_DIR" && [ -d node_modules ] || npm ci --ignore-scripts )
# Capture scripts are vitest files that import TS modules and write JSON via FIXTURE_OUT.
pkg="$(sed -n 's|^// package: *||p' "$capture" | head -1)"; pkg="${pkg:-daemon}"
cp "$capture" "$OPENRIG_DIR/packages/$pkg/test/__parity_capture.test.ts"
trap 'rm -f "$OPENRIG_DIR/packages/$pkg/test/__parity_capture.test.ts"' EXIT
( cd "$OPENRIG_DIR/packages/$pkg" && FIXTURE_OUT="$out" TZ=UTC npx vitest run test/__parity_capture.test.ts )
git -C "$here" status --short -- "tests/fixtures/$slice"
