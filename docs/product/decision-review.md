# semantic review: choices, behaviors, and design

## the primary review object

a decision describes a choice encoded by the change and its consequences. it can span several files, and one file can support several decisions. frame it in the user's or system consumer's terms: “failed saves retain your draft,” “retries stop after three attempts,” or “callers must now supply a workspace.”

technical and design decisions still belong here. describe their observable contract, alternatives, and tradeoff in plain language rather than listing implementation symbols. keep the relevant code alongside the semantic account, with synchronized selection and optional focus modes.

every decision answers:

- **what:** what experience, outcome, or contract changes; who is affected and under which conditions.
- **how:** the concise mechanism, including important branches, ordering, failure, and recovery behavior.
- **why:** the rationale and tradeoff, with attribution. author-stated, agent-annotated, inferred, and unknown motivations remain distinguishable.

the implementation can establish what it does without establishing why the author wanted it. inferred rationale must be labeled even when plausible. future agent annotations can capture reasons during implementation, when that context is available; conflicting reasons and behavior should remain visible for review.

## example decision

illustrative product change; neither the behavior nor the scores below are findings about gander.

> **failed saves retain your draft**
>
> **what · 95/100:** if saving fails, your text stays in the editor and you can retry.
>
> **how · 91/100:** clear the draft after a successful save. on failure, keep the draft and show the error. disable Save while a request is pending.
>
> **why · unknown:** no rationale annotation yet. possible motivation: avoid losing work during a network failure. this is an inference, not the author's stated reason.

| situation | before | after | confidence |
| --- | --- | --- | --- |
| save succeeds | draft clears | draft clears | 96/100 |
| save fails | draft clears; retry requires retyping | draft remains; retry is available | 94/100 |
| user clicks Save twice quickly | overlapping requests are possible | Save stays disabled while pending | 87/100 |
| user navigates away during save | not established | not established | unknown |

the card describes the decision in words while its relevant code can appear alongside it. selecting a confidence score reveals its basis and missing evidence. selecting a case can open a behavior trail once that feature exists.

## confidence scores

include scores for substantive inferred claims: what, how, an inferred why, individual case outcomes when confidence differs, and behavior-trail steps or edges. score **confidence that the description matches the identified revision and stated scope**. product desirability, behavioral correctness, and test coverage are separate questions.

start with a compact 0–100 estimate plus a short basis: direct source evidence, supporting tests, unresolved dynamic behavior, missing context, or conflicting evidence. semantic grouping and folding inferences also retain a score and basis, accessible from their summary. label model-estimated scores as estimates; a score of 90 is not a measured 90% probability until calibration supports that interpretation. avoid false precision beyond whole numbers.

unknown is a distinct state rather than zero. allow abstention. a well-supported “rationale is not documented” should not be converted into high confidence in a guessed rationale. author attribution establishes who stated a reason, not that the resulting implementation fulfills it.

keep confidence local: one clear happy path must not mask an uncertain failure path. runtime evidence can support the particular exercised case; a passing test does not establish the entire table.

evaluate scoring against labeled examples over time: are low-confidence claims actually more error-prone, and are high-confidence descriptions supported? until then, use scores to guide investigation alongside their evidence basis. never use a score alone to hide changes or mark them reviewed.

## collapse diff noise

collapse as much low-value syntax as practical: recognized import blocks, formatting-only changes, repeated mechanical edits, and generated churn. preserve a one-action keyboard route to every original hunk and show what was folded.

an import summary might read “imports reorganized in 8 files; 1 new runtime dependency needs attention.” import execution, module initialization, dependency changes, stylesheet order, or registration side effects can affect behavior. surface those choices in the decision view; classification as an import does not establish irrelevance.

use deterministic parsing/diff facts for mechanical folding where available. use transient ACP inference for semantic classification and explanation. unsupported syntax or uncertain consequences remain inspectable and explicitly unresolved. import-folding itself must not wait for inference to make the rest of the review usable.

track a minimal accounting of the diff: each changed region is represented by a decision, included in a labeled mechanical group, or left as an unexplained change. expose gaps compactly. this measures representation, not proof that all consequences have been discovered.

## behavior trails, after decision tables

start from a decision or a case, and explain the resulting sequence in words:

```text
you choose Save
  → Save becomes unavailable while the request runs
  → success: the editor clears and confirms the save
  → failure: your draft stays, an error appears, and Save becomes available
```

each step has a concise mechanism description, confidence, and expandable evidence. show changed steps, alternative branches, and unknown transitions. product descriptions lead; functions and files appear when the reviewer asks how we know.

## inference and correction

start inference only when the reviewer chooses **Analyze** for the selected comparison. opening the pane or navigating branches/ranges must not start a review. once requested, infer decisions prioritizing the current review area. use the [transient Codex ACP plan](collaboration-and-integrations.md#transient-acp-inference); persist useful review artifacts separately from agent sessions. keep exact revision identity and evidence references with the result.

reviewers can correct a description, annotate rationale, split/merge decisions, or ask for supporting evidence. retain attribution and snapshot identity. refresh affected claims after code changes and indicate when prior human annotations need another look.

the quality test: can someone explain the choices and resulting experience accurately without opening source? code inspection remains a fast option for verification, ambiguity, and technical depth.
