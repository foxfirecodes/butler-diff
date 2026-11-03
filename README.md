# gander 🪿

<img src="assets/icon.svg" width="128" alt="Gander, a goose taking a look at a code review">

Take a gander at your code.

A native code review app for Git branches and [GitButler](https://gitbutler.com/) stacks, built with Rust and [GPUI Kit](https://gpui-kit.com/).

Product ideas and experience plans live in [docs/product](docs/product/README.md).

- Review individual branches, entire stacks, or branch ranges.
- Syntax-highlighted split/unified diffs with automatic refresh.
- Semantic review alongside the code: concise what/how/why, before/after cases, confidence details, and expandable behavior trails.
- Reversible folding of complete import-only hunks in Rust and JavaScript/TypeScript.
- Regex file groups, filters, per-group file/LOC counts, and collapse/expand-all controls.
- Persistent viewed files that reset when their diff changes.
- Remembers your repo, branch range, collapsed files/groups, view options, pane widths, and last file.
- Markdown comments, replies, edit/delete/undo, and live agent collaboration over MCP.
- Independent review windows for multiple repositories or comparisons.

## Run

Requires Git and Rust 1.98.1 (pinned in `rust-toolchain.toml`). Use `mise install` to set up the toolchain.

```sh
cargo run -- /path/to/repository
```

On macOS: `mise run build` builds debug, `mise run app` packages release, and `mise run install` builds/signs/copies it to `/Applications/Gander.app`.

`mise run dev /path/to/repository` rebuilds/restarts **Gander Dev** in the background, with its own app identity and saved UI state. Repo comments/viewed state remain shared. `mise run icon` regenerates the icon from the SVG.

Click a branch, click a stack’s **view all**, or **shift-click** another branch to select a range. Use **Groups…** to organize files. **⌘Enter** submits comments; **Escape** closes the composer and keeps a draft for this session.

Repositories without GitButler metadata show local branches instead of stacks. Each branch shows its committed changes since its merge-base with the selected base. Use **Base: …** to choose a local or remote base branch; the GUI remembers your choice per repository. You can also pass `--base <ref>` to the GUI, `--inspect`, or `--mcp`.

The automatic base for regular repositories is `origin/HEAD` when available, then local `main` or `master`, then the current branch (or the first local branch for a detached HEAD). GitButler repositories keep their configured target and stack boundaries. Stack/range selection applies to GitButler repositories.

Use **⌘K** to jump to a file, branch, or stack. **⌥← / ⌥→** or **⌥[ / ⌥]** goes back/forward through visited files.

The **Semantic** button (**⌘⇧S**) switches between semantic review and the file navigator. **⌘⇧[ / ⌘⇧]** moves between decisions (or files in the file navigator); **⌥↑ / ⌥↓** moves between code changes. **⌘⇧Enter** inspects the current decision and advances, or marks the current file viewed and advances after saving. **⌘⇧Z** undoes that inspection, and **⌘⌥U** jumps to the next uninspected item. **⌘⇧L** folds/reveals the current file's import-only hunks. These review shortcuts leave text-input and comment-composer editing alone.

Semantic analysis starts only when you click **Analyze**, using an installed, signed-in Codex CLI (`codex login`). Opening the pane, switching branches/ranges, refreshing, and reopening a repository do not start analysis. Changing comparisons cancels any analysis in progress; the next comparison waits for **Analyze**. Gander includes a small stdio ACP bridge to Codex App Server: it explicitly requests and verifies ephemeral, read-only threads, uses `gpt-6-luna` with low reasoning, and cancels the inference process tree when a comparison is replaced or analysis is cancelled. It does not create saved Codex conversations. Before creating a thread it reads configuration-layer metadata and explicitly disables inherited local plugins and MCP servers for that thread, preserving managed entries without changing the user's Codex configuration. This keeps local plugin hooks, tool services, and build watchers out of background inference. It discovers native Codex binaries behind npm/pnpm launchers so macOS app launches do not require Node on the GUI's PATH. `GANDER_CODEX` can specify an executable explicitly. Startup and idle failures include process status, the last protocol event, and a bounded stderr tail.

Successful semantic reviews are cached locally in Git's common directory as `gander-semantic.sqlite`, keyed by exact base/tip, model, reasoning effort, and review-format version. The cache retains which source ranges were supplied. Confidence scores are model estimates with an expandable evidence basis; rationale is labeled inferred or unknown. Source citations are checked against the committed lines actually returned to the agent. Review descriptions, progress updates, before/after cases, confidence details, and behavior trails render Markdown. Evidence within the diff jumps to its code; context outside the diff opens a syntax-highlighted committed-source preview with exact source line numbers and selectable code. Existing test files are evidence about intended behavior; analysis does not run them.

Analysis starts with a small changed-file inventory instead of embedding the patch in a prompt. The agent progressively pages per-file diffs, reads surrounding committed source, and searches the exact original/modified trees for callers, renderers, and tests. There is no total patch-size gate. Each source page is bounded to roughly 24 KiB and at most 200 lines; inventory and search results also paginate. Individual oversized or non-text lines are explicitly omitted and cannot support citations. The first pass targets a concise review brief with 8–16 source calls and a hard cap of 24. It prioritizes product choices over generated mirrors and mechanical plumbing; uncited changed files are listed automatically. Citation/shape corrections stay in the same transient session with its prior response and supplied evidence, use at most six additional reads, and receive exact invalid ranges plus available source coordinates. If correction fails, supported decisions remain visible in an explicitly partial review while unsupported choices are omitted; partial reviews are not cached. The completed review shows how many changed lines were actually read; unexplained-file accounting and confidence estimates do not prove exhaustive behavioral coverage.

The agent uses `gander_report_progress` for short, live updates about the behavior it is checking. The latest update stays visible, source-read activity appears separately, and **Show earlier updates** expands the status trail. Startup milestones and Codex retry errors use the same trail. Codex has a three-minute inactivity watchdog, and the host allows time for its diagnostic error to return before enforcing a separate five-minute bridge watchdog. Analysis stops after fifteen minutes overall. The bridge forwards activity during long reasoning/output phases without displaying private reasoning, and host disconnection cancels an in-flight request. The dev launcher captures status/errors in `target/debug/gander-dev.log` and cleans up stale inference groups on restart.

Import folding recognizes complete, simple, single-line import/use statements in import-only hunks; mixed or multiline hunks remain visible. Folded imports can still have dependency or initialization effects, so their original source is always available through **reveal**.

Use **⌘N** (or **⋯ → New window**) for another independent review window. **Open repo…** changes the repository in the active window; **⌘W** closes only that window. The **⋯** menu also contains branch-sidebar, split-diff, line-wrap, and sidebar-placement settings.

## MCP

Choose **⋯ → Set up MCP…**, copy the command for **Codex** or **Claude Code**, and run it from the project's directory. Gander adds or updates its own entry while preserving other servers and settings. You can also run the setup directly if `gander` is on your PATH:

```sh
gander --setup-mcp codex
# or
gander --setup-mcp claude
```

Codex setup writes `.codex/config.toml` and requires a trusted project. Claude Code setup writes `.mcp.json`. Repository resolution stays relative to the open project: Codex uses its current project directory, and Claude Code supplies its project root through `CLAUDE_PROJECT_DIR`. Restart the MCP connection after setup.

For other MCP clients, add this to their configuration, replacing both paths:

```json
{
  "mcpServers": {
    "gander": {
      "command": "/absolute/path/to/gander/target/debug/gander",
      "args": ["/path/to/repository", "--mcp"]
    }
  }
}
```

Agents can read stacks/local branches/diffs and create, reply to, edit, delete, or resolve comments. Changes appear live in the app. Restart the MCP connection after rebuilding.

Reviews committed changes only. Repo settings, viewed state, and comments live in Git’s common directory (normally `.git`); UI state lives in the app data directory. Working files and Git history are untouched.

Gander preserves existing Butler Diff settings and reviews. Repo data keeps its `butler-diff*` filenames, and existing app state directories remain in use. New installations use Gander's app state directory. `GANDER_STATE_DIR` and `GANDER_INSTALL_DIR` override the state and install locations; the previous `BUTLER_DIFF_*` names are also accepted.
