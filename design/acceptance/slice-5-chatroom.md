# Slice 5: chatroom
TS sources:
- `domain/chat-repository.ts` (196 LOC): composable WHERE, topic windowing via topic markers, cursor pagination, timestamp filter comparing instants (and refusing unparseable values up front), sender filter, chronological ordering.
- `routes/chat.ts`: `POST /send`, `GET /history`, `GET /watch`, `POST /topic`.
- `cli/src/commands/chatroom.ts`.
- MCP `chat_send` / `chat_read` (ADR-0002).

## Scope
Per-rig chatroom: send, history (filters: topic window, cursor, `since` timestamp, sender, limit), topic markers, live watch (SSE/WS over UDS via the slice 4 events stream), sender identity from the seat env (`RIG_SEAT_ID` + seat token, ADR-0006), plus the CLI verbs `rig chatroom send|history|watch|topic` with `--json`.

## Non-goals
- Delivering chat into agent panes. Chat is pull/watch only; no pane paste. If TS nudges on mentions, that is reproduced through slice 4's nudge path and noted in the ledger; otherwise it is out of scope.
- Slack bridging.
- MCP wiring (slice 6 registers the tools on this API).

## Crates touched
messaging::chat, store (chat tables), daemon (routes), api, cli.

## Public types
```rust
pub struct ChatMessage { id: ChatId, rig: RigId, sender: SenderRef, body: String, at: Timestamp, topic: Option<TopicId> }
pub struct HistoryQuery { rig: RigId, topic: Option<String>, after: Option<ChatCursor>, since: Option<Timestamp>, sender: Option<SenderRef>, limit: u32 }
pub struct HistoryPage { messages: Vec<ChatMessage> /* chronological */, next: Option<ChatCursor> }
pub trait ChatRepo { fn post(&self, tx, NewChat) -> Result<ChatMessage>; fn history(&self, q: &HistoryQuery) -> Result<HistoryPage>; fn set_topic(&self, tx, rig, name) -> Result<TopicId>; fn has_topic(&self, rig, name) -> Result<bool>; }
pub enum ChatError { UnparseableSince(String), UnknownTopic(String), UnknownSender, .. }
```

## Acceptance criteria
1. `send` stores a message with the sender resolved from seat identity. Calls from outside a seat are attributed to `operator`. A spoofed sender (no or wrong seat token) is rejected.
2. History without a cursor returns the newest `limit` rows in chronological order. With a cursor, it pages forward from it (TS semantics).
3. `since` accepts RFC3339 with or without offset, and SQLite-style `YYYY-MM-DD HH:MM:SS`. It compares instants (a `+02:00` value equals its UTC twin). An unparseable value returns `UnparseableSince` (not an empty list).
4. Topic windowing: `set_topic("x")` inserts a marker. `history(topic="x")` returns only messages after that marker and before the next marker. An unknown topic returns `UnknownTopic` with the `has_topic` check.
5. Sender filter combined with topic, cursor and since composes correctly. This is a property test against a naive in-memory reference implementation.
6. `watch` streams new messages for the rig in order, with resume from the last id after a reconnect.
7. CLI `--json` shapes are goldens. `rig chatroom watch` prints messages as they arrive and exits cleanly on Ctrl-C.

## Required tests
Repository unit tests against `:memory:` (AC2–AC5), including the reference-model property test; route tests over UDS (AC1, AC6); CLI goldens (AC7); diff-harness case `chatroom-send-history` against TS (normalised). Ledger: `chat-repository`, `routes/chat` and `chatroom` CLI tests are ported.

## Definition of done
AC1–AC7 pass, diff cases Equal, ledger rows mapped, notice headers on derived files.

## Dependencies
Needs slices 0 and 1 (merged), and slice 4's events endpoint for AC6. Without slice 4, AC6 is the only blocked AC, and `watch` can use a temporary chat-only SSE route that is then folded into `/api/events`.
