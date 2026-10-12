# Claude Code 2.1.286 hook fixtures

These objects are the hook input examples published at
<https://code.claude.com/docs/en/hooks> (fetched 2026-10-12), one JSON
object per line, key order preserved from that page.

The spike capture `spikes/scratch/10-claude-compaction/hooks.jsonl` is not
in this repository. Two lines are therefore not a raw spike recording:

- `SessionStart` with `source=compact` keeps the documented common fields
  and sets `source` to the documented compact value. The page's example is
  a resume.
- `PreCompact` with `trigger=auto` is the documented manual example with
  `trigger` set to `auto`. The page states that automatic compaction sends
  `custom_instructions: null`.

`labels.jsonl` is the sha256 of each line without its trailing newline.
`Elicitation` is included so an unknown event stays `Unknown`. The last
`PreCompact` line adds `extra_field`, which the parser ignores.
