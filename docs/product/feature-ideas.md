# feature ideas

candidate names are working labels. “smallest version” is a scope constraint, not an implementation estimate. all features below are proposals or extensions; current capabilities are listed in the [product overview](README.md#starting-point).

the chosen core is **keyboard review loop → before/after decision tables → behavior trails**. this is semantic review: choices, behaviors, and design described concisely, with relevant code alongside. the [decision review contract](decision-review.md) defines what/how/why, confidence scores, and diff-noise collapsing.

## navigation and continuity

| idea | why it helps | smallest version |
| --- | --- | --- |
| review queue | keeps the next useful action close | next/previous item, inspect + advance, next uncertainty or question; file/hunk actions remain available |
| review checkpoints | avoids rereading an entire evolving change | save the reviewed base/tip; list files whose comparisons changed |
| exact context peek | answers “what calls this?” without losing place | open a source excerpt from the selected snapshot, then jump or return |
| review by question | organizes investigation around intent | pin a question with several source links, e.g. “can cancellation leak capacity?” |
| stack context peek | separates this branch's contribution from its prerequisites | peek the predecessor or whole-stack comparison while keeping the current branch selected |
| resumable sessions | reduces the cost of interruption | restore location and show new commits, changed reviewed files, and unanswered questions |

### review checkpoints: “what changed since i looked?”

example: review `base A → tip B`, then an agent produces `tip C`. keep the original comparison available, show the current `A → C` review, and let the reviewer inspect `B → C` for files needing another look.

existing viewed state already distinguishes both file blobs, paths, and diff headers. build on that identity; checkpoints add the historical comparison and a return path. keep this separate from a blanket “viewed” boolean.

start with a single explicit checkpoint per comparison. expose added, changed, removed, and unchanged files. a removed file can still require review. preserve old questions and excerpts even when their anchors no longer match.

**hard case:** when the base also changes, `old tip → new tip` includes upstream movement. show old and new endpoint pairs and label that limitation. rename matching and rewritten-history correspondence are later improvements. identical text after a rebase can preserve reading progress, but changed dependencies may still deserve attention.

## transformations and understanding

| idea | why it helps | smallest version |
| --- | --- | --- |
| behavior trail | explains the experience and its branches | narrated user/system steps, confidence, and synchronized source |
| change walkthrough | offers a useful reading order | author- or agent-proposed stops with one sentence explaining each |
| before/after decision table | exposes choices, changed outcomes, and missing cases | inferred what/how/why plus concise cases, confidence, and synchronized source |
| lifecycle comparison | makes async ownership and cleanup understandable | a small before/after sequence for one operation |
| contract lens | makes compatibility changes explicit | compare one schema or signature, including omitted/null/default distinctions |
| repeated-change compression | focuses attention on exceptions in mechanical edits | group exact repeated edit patterns and enumerate all occurrences |
| relocation-aware reading | separates movement from edits inside moved code | pair one moved block with its source and highlight internal differences |
| structured value lens | makes structured changes readable | parsed JSON key/value changes; later formats added individually |
| UI artifact comparison | connects implementation to visible behavior | inspect supplied before/after screenshots or build artifacts alongside source |

### behavior trails

start from a decision or case and ask “what experience does this create?” get a compact account such as:

```text
choose Save → Save becomes unavailable → request completes
                                        ↘ success: clear draft and confirm
                                        ↘ failure: retain draft and allow retry
```

each step describes the behavior and mechanism concisely, with confidence and exact source alongside or available to inspect. missing dynamic dispatch or indirect subscription edges remain gaps. inferred steps describe possible execution; runtime evidence, when available, is a separate attachment.

**first experiment:** after the decision-table slice works, generate one small trail through transient Codex ACP inference and compare it with a human-annotated account. measure whether the reviewer can describe the experience and design accurately without having to reconstruct it from code. this can work before building symbol indexing or automatic graph generation.

### before/after decision tables

make a behavior change inspectable as a handful of cases. example, for a save-flow change:

| case | before | after | evidence to inspect |
| --- | --- | --- | --- |
| save succeeds | draft clears | draft clears | success branch + save test |
| save fails | draft clears; retyping required | draft remains; retry available | failure branch + failure-path test |
| user clicks Save twice | requests can overlap | Save stays disabled while pending | pending state + interaction test |

this is an illustrative table, not a finding about gander. the actual view includes compact confidence scores, with provenance attached at the cell or row where needed: source-derived, agent-inferred, observed in a test, or unknown. an inferred expected result must never look like a passing test.

start with an automatically inferred, editable what/how/why decision and case table. use transient ACP inference with `gpt-6-luna` low/medium; rationale can remain unknown until annotated. ask the agent to explain uncertainty, find a counterexample, or produce a targeted test for one row. keep invented cases visibly distinct from existing test coverage.

### collapse mechanical diff noise

fold recognized imports and other low-value syntax into compact summaries, with counts and a keyboard reveal action. distinguish mechanical churn from new dependencies, import-time effects, and other changes with behavioral significance. retain an explicit route to unexplained changes. see the [folding rules](decision-review.md#collapse-diff-noise).

### repeated-change compression

“this same replacement occurs in 46 places; these 3 edits differ.” show a representative edit, the matching occurrences, and exceptions. every occurrence remains reachable. grouping alone does not mark files viewed.

first support exact textual edit patterns. syntax-aware matching could later recognize renamed identifiers or equivalent structure. even an identical edit can behave differently in a different context, so present this as reading compression rather than a correctness claim.

### lifecycle and contract lenses

a lifecycle view can put “queued → running → cancelled → capacity released” next to the old sequence and link each transition to code. focus on one operation, especially failure, timeout, reset, and unmount paths. represent unknown transitions explicitly.

a contract view can show required fields, defaults, nullability, and writer/reader changes together. it is especially useful for staged deployments and persisted data. begin with one concrete schema format or a manually assembled comparison; generic cross-language compatibility analysis is a larger bet.

### structured values and visual artifacts

start deterministic: show JSON fields added, removed, or changed, with raw text one action away. schema-aware array matching, SQL plans, rendered Markdown, image diffs, and UI screenshots are separate format-specific experiments.

for visual artifacts, retain which revision, environment, and capture process produced each image. a screenshot's presence does not establish that the changed code produced it. prefer importing existing artifacts before adding a build/capture runner.

## collaboration and verification

| idea | why it helps | smallest version |
| --- | --- | --- |
| ask about this | resolves uncertainty in the semantic account | decision or selection + question + exact snapshot; transient ACP answer or existing local-agent thread |
| counterexample hunt | tests an actual concern rather than requesting a generic review | ask for one reachable failure scenario with source evidence |
| evidence attached to answers | makes claims cheap to verify | source links, test invocation/result, revision, and limitations |
| inspect the agent's follow-up | closes the request → change → rereview loop | show changes since the question's snapshot, then let the human resolve it |
| relevant checks near the code | avoids switching tools to interpret a failure | existing check summary and linked file annotations |
| portable review links | lets tools open the same context | repo/comparison/path/side/line link with clear handling of missing revisions |

an important variation: ask the agent to **explain a deliberate tradeoff** or **show the strongest counterargument**, not just find bugs. useful collaboration includes resolving uncertainty and learning the code.

## ideas to earn later

- automatic suggested reading order: first prove that curated walkthroughs help.
- test selection and execution: first import existing evidence; add execution for a concrete build-tool workflow.
- rich call graphs and state machines: first prove that small trails answer real review questions.
- working-tree review: valuable for agents, but requires explicit snapshots and GitButler attribution rules.
- broader forge support and sharing: first make one GitHub round trip reliable.
- multiple specialist agents: first make one scoped question and its answer easy to manage.

the scarce resource is reviewer attention. each addition should either remove navigation effort, expose a consequence, or make evidence easier to assess.
