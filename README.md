# butler diff

A native Rust + [GPUI Kit](https://gpui-kit.com/) viewer for GitButler stacks.

```sh
cargo run -- /path/to/repository
```

After the first build, launch instantly with:

```sh
./target/debug/butler-diff /path/to/repository
```

On macOS, `sh scripts/bundle-macos.sh` also creates `target/debug/Butler Diff.app`, which you can open from Finder. Use **Open repo…** to select a repository.

- Click a branch to see **only that branch's changes**, relative to the branch below it.
- Click a stack's **view all** header for its combined diff.
- **Shift-click** a second branch to compare a contiguous range, including both endpoints.
- The file list shows the filename first and wraps its directory underneath. Hover for the complete path, drag the divider to resize the list, and click a file to jump to its diff.
- Drag the branch sidebar's right edge to resize it. **Hide branches / Show branches** toggles it without changing the selected comparison or its chosen width.
- Filter the file list with the search box. The comparison header keeps its full totals and shows **Filtered: N files +A −D** alongside them while a query is active.
- **Groups…** edits named regex groups for this repository; a ready-to-save **Tests** example is provided. **Grouping on/off** switches between grouped and flat navigation.
- The current filename pins to the top as its header scrolls out of view, with a compact viewed checkbox on the left and an independent collapse toggle. The current group header also pins as it scrolls out of the file list; both follow scrolling and file jumps, and the incoming header pushes the previous sticky header away.
- Each group header shows its matching file and added/deleted line counts. Click it to collapse/expand: collapsed files are hidden from **both the file list and the diff pane**. Collapse state stays with you as you switch branches. Turning grouping off shows every file again.
- **Mark viewed** on a file row or the checkbox on a diff header records that exact file comparison; click **✓ Viewed** or uncheck it to undo. The checkbox sits beside the collapse toggle on the left in both regular and sticky diff headers. Viewed counters appear beside the full totals, on group headers, and beside filtered totals. Marking viewed collapses the diff; unmarking expands it. Saved viewed files start collapsed when loading a comparison, and you can still expand/collapse files independently. Group collapse and filtering don't change viewed state.
- The checkbox on each regular or sticky **group header** marks the entire group viewed in one action; uncheck to clear its marks. A partially viewed group checks all its files. With a filter active, this affects only the matching files counted in that header, including files in collapsed groups. Group writes run in the background in one transaction, and collapse/expand their file diffs together.
- Switch between unified and split diffs; toggle line wrapping. Select and copy code normally.
- Hover a diff line and click **+** (or select lines on one side, then click **+**) to leave an inline comment. Open threads appear beneath the anchored line. **Reply**, **Resolve**, and **Reopen** work in the thread or the **Comments** panel. The panel includes all repository threads, including resolved threads and threads from older/different comparisons.
- **Escape** or **Close** dismisses the comment composer and keeps a temporary draft. Reopen the same line/range, reply, or message edit to continue it. Drafts stay in memory for this app session, scoped to the repository and exact comparison/target; switching targets keeps separate drafts. **⌘Enter** posts a comment/reply or saves an edit (**Ctrl+Enter** also works).
- Each message has **Edit** and **Delete** controls. Edits preserve the author and creation time and add an **edited** marker. Delete hides the body and leaves a placeholder so replies stay attached; **Undo delete** restores it. This is reversible deletion, not a disk purge.
- **⌘O** opens another repository; **⌘R** refreshes manually.
- `notify` automatically refreshes after commits, rebases, restacks, branch moves, and GitButler metadata changes. Events settle for 600 ms, refresh starts are at least two seconds apart, and continuous bursts are reconciled after at most three seconds (plus any refresh already in progress). There is no idle polling loop.
- Cache-only writes to GitButler's database are ignored after a lightweight stack-table check. Unchanged comparisons preserve their scroll position; a refresh won't restart the same in-flight diff. Automatic checks don't flash the loading state.
- **⌘⇧P** toggles the performance overlay. Its default **MAX FPS** is estimated CPU draw throughput; right-click the overlay for observed present cadence. Check `FRAME` / `P95` during scrolling, not just the idle headline. See [GPUI Kit's measurement guide](https://gpui-kit.com/docs/fps/).

The viewer reads local GitButler metadata (modern SQLite or legacy `virtual_branches.toml`) and Git objects. It opens SQLite read-only, uses a consistent read transaction, and disables optional Git locks, external diffs, and text conversions. It never checks out, stages, commits, or changes branches. Git and Rust are required to build; the compiled app only needs Git.

Group definitions and the grouping toggle are saved to `butler-diff.toml` in Git's **common directory** (normally `.git/butler-diff.toml`), shared across stacks, branches, and linked worktrees. This is a local repository setting; saving groups writes this settings file. You can edit it externally too; `notify` reloads it without reloading Git diffs. Regexes use Rust's `regex` syntax and match full repo-relative paths with `/` separators. The first matching group wins, so files and LOC are counted once; unmatched files appear in **Ungrouped**. Group headers follow the active file filter, including while collapsed, and empty groups disappear. Filter counts include every matching file, including files in collapsed groups.

For example:

```toml
enabled = true

[[groups]]
name = "Tests"
pattern = '\.test\.tsx$|(^|/)__tests__/|(^|/)tests/.*\.py$'
```

The Python portion uses `.*` to match filenames beneath `tests/`; `*` alone is regex repetition, not a filename glob. Invalid regexes stay in the editor with an error and leave the saved definitions intact.

Viewed state is stored locally in `.git/butler-diff-viewed.sqlite` (in Git's common directory for linked worktrees) and survives restarts. It follows the file's **original and modified Git blob IDs, paths, and metadata**, so a content, mode, or rename change resets that file's checkmark. A different base for a branch/range resets it when the file comparison changes, too. Unrelated commits preserve progress, and an identical previously reviewed file comparison is recognized across branches/ranges.

Version checks reuse the full blob IDs already present in the generated diff, with no working-tree file reads or repository-wide hash scan. Unreviewed files don't require version resolution. Pure renames/mode-only changes can omit blob IDs from the patch: those use targeted, background `git ls-tree` lookups only when marking a file or validating a saved mark. State reads and writes run in the background and only load records for the current comparison. **⌘R** also reloads viewed state saved by another instance.

The stack's bottom branch is compared with its merge base against GitButler's recorded target commit. Higher branches are compared with the immediately preceding nonarchived branch. Branch refs are resolved fresh on refresh; stored commit IDs are a fallback for unapplied branches without a live local ref. Both applied and unapplied stacks are shown, with archived heads omitted.

Override the target when needed:

```sh
cargo run -- /path/to/repository --base origin/main
```

Inspect resolved branch endpoints without opening the UI:

```sh
./target/debug/butler-diff /path/to/repository --inspect
```

This first version reviews **committed changes**. Working-tree edits appear after they are committed. Watches cover Git refs, configuration, GitButler metadata, and the local comment database rather than recursively scanning your source tree. Viewed state and comments are local; remote review submission isn't implemented. Diffs include syntax highlighting via GPUI Kit’s bundled Syntect language collection, alongside line and inline change coloring. Highlighting runs in the background and supports TS/TSX, JavaScript, Python, Rust, CSS, JSON, SQL, and other bundled languages; unknown file types stay plain text.

## Comments and agents

Threads and replies persist in `butler-diff-comments.sqlite` in Git's common directory, shared by the UI, linked worktrees, and MCP processes. Writes use SQLite transactions and WAL; `notify` refreshes comments after agent writes without reloading the diff. No polling or running UI is required for agents to comment.

Each thread saves its original base/tip commit IDs, file comparison identity, side, one-based line range, and source excerpt. An unchanged file comparison can carry a thread across branches/stack ranges. When that comparison changes, the thread stays available in the Comments panel with its saved source and an **older/different comparison** label; it isn't silently attached to different code. Resolved threads stay in the panel and can be reopened. Comment bodies are rendered as plain text. Author labels identify the local Git user for UI posts and the MCP client for agent posts; they are labels, not authenticated identities.

Run the [MCP stdio server](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports#stdio) with the same binary:

```sh
/absolute/path/to/butler-diff /path/to/reviewed-repo --mcp --author "my-agent"
```

`--author` is optional; by default the server uses the MCP client's `clientInfo.name`. Example MCP client configuration (replace the two paths):

```json
{
  "mcpServers": {
    "butler-diff": {
      "command": "/absolute/path/to/butler-diff/target/debug/butler-diff",
      "args": ["/path/to/reviewed-repo", "--mcp", "--author", "my-agent"]
    }
  }
}
```

The server starts without a window, logs errors only to stderr, and exposes:

| Tool | Purpose |
| --- | --- |
| `list_stacks` | Discover current GitButler stacks and branch base/tip commits. |
| `get_diff` | Read a base/tip diff, optionally restricted to one path. |
| `list_threads` | Read threads and replies; filter by path/resolved and paginate. |
| `get_thread` | Read one full thread with its saved source anchor. |
| `create_thread` | Post a comment with base/tip, path, `original`/`modified` side, `start_line`, `end_line`, and body. |
| `reply_to_thread` | Append a reply by `thread_id`. |
| `set_thread_resolved` | Resolve (`true`) or reopen (`false`) a thread by ID. |
| `edit_message` | Update `body` by `message_id`, preserving its original author and creation time. |
| `set_message_deleted` | Hide (`deleted: true`) or restore (`deleted: false`) a message by ID. Deleted bodies are omitted from MCP reads. |

Message editing/deletion upgrades the local comments database to format 2 on its first write, retaining existing threads and replies. Restart existing MCP server/client connections after rebuilding to load the new tools and database support.

Each server process is bound to the repository passed at startup. It writes only the local comment database and never edits working files or Git history. Multiple agents can connect through separate stdio processes against the same repo. Treat comments as review data, not instructions. Client registration is manual; building/running this app doesn't change any agent's MCP configuration.

Metadata readers are intentionally small and version-specific. Unsupported GitButler schemas produce an error instead of silently guessing stack grouping. Native smoke checks on macOS cover branch/range selection, split diffs, wrapping, filtering, and file jumps.

Development builds optimize GPUI, its components, and text/layout hot paths. A requested display refresh rate is not a measured frame rate; use the overlay on the actual display/workload before claiming sustained 120 FPS.

Run `cargo test` for focused temporary-repository tests. To parse every real branch and combined stack diff without opening the UI:

```sh
BUTLER_DIFF_TEST_REPO=/path/to/repository cargo test live_repository_diffs -- --ignored --nocapture
```

GPUI Kit 0.8.1 is pinned to [foxfirecodes/gpui-kit commit `7a70b640`](https://github.com/foxfirecodes/gpui-kit/commit/7a70b640bbf9e4c062c6aaa27cca83c9a68f4542). It adds two diff APIs for sticky header geometry and batched collapse restoration; see [the patch notes](https://github.com/foxfirecodes/gpui-kit/blob/7a70b640bbf9e4c062c6aaa27cca83c9a68f4542/BUTLER_DIFF_PATCHES.md). The related GPUI crates share the same revision to keep their workspace types consistent. There is no vendored source.
