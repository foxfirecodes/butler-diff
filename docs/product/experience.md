# the experience

## the default surface

one clear comparison identity, a compact decision navigator, and a generous surface for concise explanations and before/after cases, with relevant code alongside. selecting a decision focuses its supporting code; selecting code reveals the associated decision when one is known. the existing diff remains a usable fallback while inference is pending or unavailable.

```text
repo / stack / branch range                  jump   ···
──────────────────────────────────────────────────────
decisions      │  what / how / why       │ relevant code
               │  before → after cases  │
current choice │  confidence + gaps     │ imports folded
               │  design and tradeoffs  │
──────────────────────────────────────────────────────
                   optional source or question peek
```

this is a hierarchy sketch, not a layout commitment. semantic review leads and source stays synchronized rather than becoming a separate disconnected mode. allow the code pane to collapse for focused reading. evidence can also be tests, artifacts, or an attributed rationale annotation.

### progressive depth

1. **review:** what/how/why in concise language, before/after cases, compact confidence scores, and relevant questions.
2. **follow:** narrated behavior steps, alternative outcomes, or a short explanation of uncertainty.
3. **inspect:** exact source, test evidence, a folded import block, or a longer agent investigation. one temporary detail surface at a time.

use the same selection and navigation model across these depths. opening a trail should not create a second, disconnected review workspace.

## a complete keyboard loop

extend the existing jump palette and back/forward history. keep navigation discoverable through visible command labels and shortcuts in menus.

| intent | proposed behavior |
| --- | --- |
| go somewhere | keep `⌘K` for files, branches, stacks; add decisions, cases, and threads as those surfaces arrive |
| do something | a searchable action palette for less frequent actions; shortcut to be decided |
| keep reviewing | next/previous decision and case; file/hunk navigation remains available in the diff |
| finish an item | mark a decision inspected and advance; retain viewed-file actions in code view |
| find remaining work | next uninspected decision, low-confidence claim, unexplained change, or open question |
| investigate | inspect evidence or ask about the selected decision; selected lines remain a valid entry point |
| return | Escape closes the peek; back/forward restores the originating location |

exact new bindings need a small interaction spike. respect text selection, comment editing, and existing platform conventions; single-letter shortcuts should only operate in the appropriate focused surface. offer familiar shortcuts before adding an optional modal/Vim-style keymap.

viewed/inspected means “i looked at this version.” resolving a discussion means “this discussion is settled.” inspecting a decision does not mark all supporting files viewed, and folding a block does not mark it reviewed. neither action automatically approves a change or proves it correct.

### protect place and attention

- preserve scroll, selection, draft, and navigation position while peeking or receiving an answer.
- when commits arrive, show a compact update affordance and preserve the version currently being read until the reviewer switches. make the snapshot identity visible.
- an agent response can indicate that it is ready without scrolling, opening a pane, or moving focus.
- resuming a review should return to the last location with a short account of what has changed.
- filters show how much they hide and offer a single clear/reset action.

## visual rules

give the decision and its changed outcomes the strongest useful contrast. code uses its normal readable hierarchy when opened. metadata is quieter but remains readable. reserve color for meaning: changed outcomes, additions/deletions in evidence, current focus, and actionable uncertainty; pair it with text or shape.

use restrained diff backgrounds, an optional stronger-contrast setting, and changed-token emphasis where reliable. support both split and unified reading without adding a second set of controls.

prefer a short source-linked sentence or small table to a diagram. use a diagram when order, branching, or relationships are the actual question. large repository graphs start collapsed to the selected behavior's neighborhood.

generated explanations are the primary review content. show a compact confidence score beside the relevant claim or case and expand its basis on demand. keep this at the decision/case level rather than decorating code lines. unknown confidence stays visibly unknown. paths should emphasize filenames while keeping directories available for disambiguation.

collapse recognized imports, formatting churn, and repeated edits into small summaries with counts and a keyboard reveal action. show how much is collapsed. anything with unresolved behavioral significance remains visible as a concern; the [decision review contract](decision-review.md#collapse-diff-noise) defines the boundary.

## performance is part of the interaction contract

these are **proposed targets**, not measurements or current guarantees. measure release builds on a named reference machine before accepting them.

| interaction | initial target |
| --- | --- |
| keyboard input to visible response, including under background analysis | p95 ≤ 50 ms |
| open jump palette with a loaded comparison | p95 ≤ 50 ms |
| select an already-loaded file to useful code | p95 ≤ 100 ms |
| reopen a locally cached comparison to useful code | p95 ≤ 250 ms |
| cold launch to first useful diff for the representative small review | p95 ≤ 1 s |
| open a cached decision or case | p95 ≤ 100 ms |
| scrolling | fit work within the display's frame budget; record stalls, not just average FPS |

“useful code” means readable diff content at the requested location with working navigation, before semantic indexing or AI completion. “useful decision” means a concise, complete card with case outcomes, confidence, and working evidence links. measure time to first useful decision separately from diff readiness; choose an inference latency target after measuring the ACP slice. separately report total comparison readiness for large reviews.

benchmark small, medium, and pathological fixtures: roughly 10 files/1k changed lines; 100 files/10k lines; 1k files/100k lines, including one giant file, generated files, renames, and long lines. record repository size, hardware, cold/warm state, memory, and background activity. these fixture sizes are starting points to tune against real projects.

product consequences: render visible content first, reuse unchanged decisions, bound ACP concurrency, and cancel obsolete analysis. network access, model calls, whole-repository indexing, and test execution must not delay navigation or available content. show cached cards or the diff immediately while generating decisions. new results must not rearrange the item being read. analysis for an older snapshot stays attached to that snapshot.
