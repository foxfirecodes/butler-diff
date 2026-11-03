# product direction

working notes · 2026-11-02 · first-principles exploration

## the bet

**review the choices being made and the resulting product experience, in concise language.**

this is **semantic review**: decisions, behaviors, and design lead, with synchronized code alongside them. the primary unit is a product or engineering decision: what changes, how it behaves, and why that choice was made. before/after decision tables make the consequences concrete; behavior trails explain the experience through a short sequence of narrated steps.

the everyday experience should stay small: open a comparison, move through decisions with the keyboard, inspect uncertainty, leave a question, keep going. imports and other mechanical diff noise collapse into accessible summaries. inference runs in the background through transient Codex ACP sessions, preferring `gpt-6-luna` at low/medium effort.

## experience goals

| goal | product consequence |
| --- | --- |
| streamlined | a complete keyboard review loop; remember position and context; complexity opens on demand |
| minimal visuals | concise decisions and outcomes lead; synchronized code stays focused; mechanical detail expands on demand |
| distinctive review | explain what/how/why, compare before/after cases, and narrate the resulting experience |
| AI-native collaboration | transient ACP inference produces the review; local agents can annotate rationale and investigate questions |
| seamless integration | carry comparison and location across GitButler, editors, agents, and GitHub |
| snappy as fuck | navigation works immediately; expensive analysis stays off the interaction path |

## starting point

checked against the current checkout, rather than assumed from earlier plans:

- branch, GitButler stack, and range comparisons; ordinary repositories have a selectable base.
- split/unified syntax-highlighted diffs, file filters and regex groups, a file/branch/stack jump palette, and navigation history.
- persisted viewed files tied to both sides of the file comparison, plus saved layout and selection.
- local review threads with immutable source anchors and live agent collaboration through MCP.
- automatic refresh and independent review windows.
- committed changes are the current review boundary. working-tree review is a separate product expansion.

sources: [app overview](../../README.md), [navigation](../../src/jump.rs), [viewed identity](../../src/viewed.rs), [comment anchors](../../src/comments.rs), [MCP tools](../../src/mcp.rs). this is a source-based inventory, not a runtime or performance audit.

## strongest feature bets

| candidate | the reviewer gets | smallest useful form |
| --- | --- | --- |
| keyboard review loop | flow through the review without hunting controls | file/hunk navigation now; decision/case navigation as the new surface arrives |
| before/after decision tables | understand what changes and how each case behaves | concise what/how/why card, case table, claim-level confidence, expandable evidence |
| behavior trails | understand the sequence of experiences and choices | narrated user/system steps with branches, confidence, and optional source peeks |
| diff-noise collapsing | spend less attention on imports and mechanical churn | fold recognized blocks into summaries; expose consequential exceptions |
| rationale annotations | understand why a choice was made | attributed author/agent notes, distinct from inferred motivation |
| supporting continuity | resume and investigate evolving work | checkpoints, questions, editor/forge links, and follow-up comparisons |

**chosen order: keyboard review loop → before/after decision tables → behavior trails.** noise reduction supports the reading loop; transient ACP inference is part of the decision-table slice. checkpoints and manual agent handoff are supporting features rather than prerequisites.

## whose review are we designing for?

working assumption: the primary loop is a developer reviewing evolving local work, often with a coding agent and GitButler. reviewing a teammate's PR should use the same reading and evidence tools.

this matters: a local review is an ongoing conversation with changing code. returning to a review and evaluating an agent's follow-up deserve as much care as the first read.

## how to use these notes

- [experience](experience.md): navigation, visual hierarchy, and proposed performance budgets.
- [decision review](decision-review.md): the primary what/how/why surface, confidence scores, and coverage.
- [feature ideas](feature-ideas.md): concrete possibilities, including transformations and visualizations.
- [agent collaboration and integrations](collaboration-and-integrations.md): context handoff, evidence, and tool boundaries.
- [sequencing and experiments](roadmap.md): small slices, experiments, and open product choices.

the review model, ordering, and ACP/model preference above reflect the current product direction. detailed interactions and implementation mechanisms remain hypotheses to validate.
