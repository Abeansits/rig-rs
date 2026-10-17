# Claude Code 2.1.286 hook fixtures

`hooks.jsonl` is a byte copy of
`spikes/scratch/10-claude-compaction/hooks.jsonl` from the slice 9 spike
(Claude Code 2.1.286). The file sha256 is
`7c6b2a8b3250087c18633cf74b0a70c50b97b1c0916c393a3c00542e90504fef`.

Each line is a wrapper, not raw hook stdin:

```json
{"t": 0.0, "argv_event": "SessionStart", "input": {}}
```

`input` is the Claude hook payload. `labels.jsonl` is the sha256 of each
line without its trailing newline.

The spike archive does not include the Claude transcript jsonl named by
these payloads. `s2-result.json` only stores a projection of the
`compact_boundary` rows.
