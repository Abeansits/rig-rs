# Parity ledger

One row per OpenRig TS test file that is in scope. Status: `ported` | `partial` | `skipped` | `todo`.
`skipped` needs a reason (out-of-scope feature, TS-internal detail, superseded by fresh schema).
CI (`guards` job) fails if a row says `ported` but the referenced Rust test path does not exist.

| TS test file | Slice | Status | Rust test(s) | Notes |
|---|---|---|---|---|
