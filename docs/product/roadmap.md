# sequencing and experiments

**chosen order: keyboard review loop → before/after decision tables → behavior trails.** the target experience is semantic review: choices, behaviors, and design lead, with synchronized code alongside. no delivery dates or effort estimates yet.

## 1. keyboard review loop

**slice:** complete next/previous navigation, inspect/viewed + advance, undo, open questions, evidence peeks, and discoverable commands. begin with files/hunks in the existing UI, then carry the same interaction pattern into decisions/cases in slice 2. retain the jump palette and navigation history.

**noise reduction:** fold reliably recognized import blocks and mechanical edits, with counts and keyboard reveal. choose a concrete syntax/language to support first. surface consequential dependency or initialization changes rather than assuming every import is irrelevant.

**done when:** a reviewer can complete a small review from the keyboard, including one question, one folded block, and one source detour, with predictable focus and no mouse rescue. undoing inspect-and-advance is straightforward. measure the proposed [performance budgets](experience.md#performance-is-part-of-the-interaction-contract) against the current baseline.

inventory what the existing diff component supports before adding app-level controls. this slice creates the interaction foundation for semantic review; it does not make line-by-line reading the end goal.

## 2. before/after decision tables

**slice:** when the reviewer chooses **Analyze**, infer a concise what/how/why decision and before/after cases for one comparison. navigation must not start inference. use transient Codex ACP sessions, preferring `gpt-6-luna` low/medium. show claim-level confidence, unknowns, and evidence. keep the relevant code synchronized alongside the semantic account.

**ACP integration spike:** verify model/effort selection, transient session creation, absence from normal history, cancellation, and cleanup on success/failure. use the actual adapter's capabilities; do not assume app-server or protocol behavior carries through unchanged. persist gander review artifacts separately from sessions.

**content boundary:** what and how are concise descriptions of observable behavior and mechanism. why is attributed rationale, clearly labeled inference, or unknown. rationale annotations can grow later and do not block this slice.

**done when:** for a real change, a reviewer can accurately explain the choices and resulting experience from the semantic view without reconstructing it from code. they can inspect a score's basis, open its supporting code, ask about an uncertain case, and identify changed regions still unexplained. navigation remains responsive while inference runs or fails.

use a compact output contract and validate evidence references. begin with a bounded review scope and make incomplete coverage visible rather than requiring whole-repository semantic indexing.

## 3. behavior trails

**slice:** expand a decision or case into a narrated sequence of user/system experiences, with branches, concise mechanisms, confidence, and supporting code. highlight which steps changed before/after.

**example:** “choose Save → Save becomes unavailable → success clears the draft / failure retains it and allows retry.” code symbols support the account when inspected.

**done when:** a reviewer can follow the normal, failure, and recovery paths and identify an unknown transition without tracing functions manually. selecting a step reveals the exact evidence without losing place. an AI-inferred edge remains distinguishable from a tested observation.

use the same transient ACP inference path and decision identity from slice 2. start with one bounded behavior rather than a repository-wide call graph.

## supporting work after the core slices

- **rationale annotations:** capture author/implementation-agent choices, alternatives, and tradeoffs, retaining attribution and revision scope.
- **checkpoints:** show what changed since the last reviewed comparison and refresh affected semantic claims.
- **repeated-change compression:** group exact edit patterns and inspect exceptions; retain all occurrences.
- **integration seams:** open GitButler/editor context, import GitHub PR context/checks, then validate anchor mapping before comment publication.
- **working-tree review:** a separate snapshot/attribution milestone, especially in multi-stack workspaces.
- **richer visual artifacts and targeted tests:** bring in existing evidence first, then add execution/capture for demonstrated workflows.

these can move earlier when they unblock a core slice, but neither checkpoints nor copied agent handoffs replace the chosen sequence.

## how we judge the product

use observed review sessions and reproducible local measurements first. instrument locally if needed; remote product analytics is a separate choice.

| dimension | evidence to collect |
| --- | --- |
| semantic understanding | can the reviewer accurately state what changed, how it behaves, and which rationale is known? |
| decision quality | does the view expose a meaningful design tradeoff or changed outcome? |
| confidence | are high-scored claims supported; do low scores identify real uncertainty; can reviewers interpret the score? |
| coverage | which changed regions remain unexplained; which meaningful consequences were omitted or wrongly folded? |
| navigation | actions to reach a decision/case/source; focus mistakes; mouse rescues |
| continuity | stale descriptions mistaken for current ones; effort to resume after new code |
| collaboration | ability to clarify rationale and verify an answer without reconstructing context |
| visual load | whether concise descriptions help or merely add another wall of text beside code |
| performance | input latency, first useful decision and diff, frame stalls, memory, inference latency/cost, background contention |

use a varied set of real reviews: a mechanical migration, async bug fix, API/schema change, UI change, and stacked feature. prepare human-reviewed descriptions and known cases to assess inference and confidence. fewer source detours is useful only if understanding remains accurate. generated finding counts and confidence scores alone do not establish review quality.

## implementation choices still open

- **ACP adapter:** exact transient-session mechanism and compatibility/version behavior, with end-to-end history verification.
- **inference scope:** how to discover enough related context while remaining quick; choose from measured examples.
- **confidence calibration:** begin with labeled model estimates and evidence bases; choose calibration work from observed errors.
- **folding syntax:** the first supported language/format and rules for consequential import exceptions.
- **local vs teammate review:** same semantic model, with forge priority driven by actual use.
- **keyboard model:** conventional shortcuts first; modal keymaps remain optional.
- **performance reference:** choose hardware and real repositories before making proposed budgets acceptance thresholds.

## ideas currently deferred

a full editor, git mutation workflow, permanent multi-agent dashboard, automatic whole-repository analysis, and a generalized plugin marketplace each create large new product surfaces. revisit one only when a concrete review problem makes it the smallest adequate solution.
