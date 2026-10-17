# What you can look up

**You are going to hit a question this onboarding does not answer.** That is expected — it is a
world model, not a manual. What matters is that you reach for a written answer instead of deriving
one, because **an invented answer and a known one look identical once you have stated it.**

So: here is what exists in writing on an OpenRig machine, and roughly what each kind of thing answers.
You do not need to read any of it now. You need to know it is there.

## The command surface — `docs/as-built/` in the source repo

Use the source repository's `docs/as-built/` index and documents marked
`kind: as-built`. Start with the entry point that matches your question:

- **`README.md` — "Map of Territory + Module Index."** Start here when you do not know which
  document you want.
- **`codemap.md` — "Navigation Index / Map of Territory."** Start here when you are about to go
  into the code and do not know which module owns the thing.
- **`cli-reference.md` — the `rig` command reference.** Every group, subcommand, flag,
  JSON shape. **This is the authority on how a command works**, and the capabilities piece
  (`public-what-you-can-do.md`) is only the map to it.

Check the document's verification marker against the source you are using. A
maintained document can still lag a changed command; its history is not proof
of current behavior.

### The field that tells you how much to trust one

Each of these carries frontmatter, and three fields are worth knowing by name:

- **`applies-when:`** — a trigger. *When* you should reach for this document, not what is in it.
  If you are scanning for something, scan these.
- **`siblings:` / `prerequisite-reads:`** — the graph. Documents here point at their neighbours, so
  arriving at roughly the right one is enough; it will route you.
- **`last-verified-against-source: <commit>`** — **the honest one.** It names the commit the
  document was last checked against. If `main` has moved a long way past it, expect lag —
  especially in areas that changed recently.

**A document that names what it was verified against is trustworthy in a specific way**, and the
field is how you calibrate rather than guess. Where a doc and the live binary disagree, **the
binary wins** — and the disagreement is worth reporting, not just working around.

## The living answer — `--help`

**Ask the binary you will actually run.** `rig <verb> --help` and
`rig <verb> <subcommand> --help` are authoritative for shape, flags and defaults. This costs
seconds and it is the single cheapest habit on this list.

**And use it as a search, not just a lookup.** `rig --help` lists the installed top-level commands. If
you are about to build something, read that list first — the most expensive failure here is
building a parallel solution out of primitives that already compose into the answer.

## Outside the forest — when nothing on this machine can answer it

There is a whole class of question the source repo and live `--help` do not reach: what an external
library actually does today, anything that changed after your training cutoff, what is true in the
world rather than in this repo.

**And the failure is the same one this page already warns about, pointed outward.** An agent that
does not know the external sources below exist invents instead of looking. An agent that does not know these exist
**answers from its training snapshot and states it as current** — and an invented answer and a
known one look identical once you have stated one.

Use the tools actually available in your harness. If a documentation connector
such as `context7` is installed, use its current lookup interface to find the
relevant library and version. If a web-search or fetch tool such as `exa` is
available, use it to locate current primary sources. These tool names are
examples, not a promise that your installation includes them. State any access
or version limitation that affects the answer.

### The trust rule, and it is not the one these tools invite

**A search result is a scrape of a page ABOUT the thing. The primary source is the thing.** Search
to FIND; go to the primary source to CONFIRM anything load-bearing — the registry, the API, the
repository, the running binary's `--help`.

Search indexes and caches can lag the source they describe. For a published
package version, query the package registry; for an API contract, inspect the
versioned provider documentation or implementation. An “updated” timestamp on
a secondary page does not establish which source revision it reflects.

So these extend your reach; they do not outrank a primary source, and *newer-looking* is not
*newer*.

## The one you read rather than consult

**`openrig-operating-model`** — the skill for context placement and chain walking. Everything else on this page you visit with a
question already in hand. **That one you read through once**, because it is the shape the rest
hangs on: the two trees, how context is arranged by altitude, and how a cold agent finds what it
needs. Reading it is what stops you inventing an arrangement that already exists.

## The habit all of this exists to support

**Three empty searches means your word for it is wrong, not that it does not exist.** Rename what
you are looking for and try once more, then ask someone who would know, then record the gap.

**Not loaded never means not available.** Almost nothing here is in your context; nearly all of it
is one command away.
