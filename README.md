# butler diff

A native code review app for Git branches and [GitButler](https://gitbutler.com/) stacks, built with Rust and [GPUI Kit](https://gpui-kit.com/).

- Review individual branches, entire stacks, or branch ranges.
- Syntax-highlighted split/unified diffs with automatic refresh.
- Regex file groups, filters, per-group file/LOC counts, and collapse/expand-all controls.
- Persistent viewed files that reset when their diff changes.
- Remembers your repo, branch range, collapsed files/groups, view options, pane widths, and last file.
- Markdown comments, replies, edit/delete/undo, and live agent collaboration over MCP.

## Run

Requires Git and Rust 1.98.1 (pinned in `rust-toolchain.toml`). Use `mise install` to set up the toolchain.

```sh
cargo run -- /path/to/repository
```

On macOS: `mise run build` builds debug, `mise run app` packages release, and `mise run install` builds/signs/copies it to `/Applications/Butler Diff.app`.

`mise run dev /path/to/repository` rebuilds/restarts **Butler Diff Dev** in the background, with its own app identity and saved UI state. Repo comments/viewed state remain shared. `mise run icon` regenerates the icon from the SVG.

Click a branch, click a stack’s **view all**, or **shift-click** another branch to select a range. Use **Groups…** to organize files. **⌘Enter** submits comments; **Escape** closes the composer and keeps a draft for this session.

Repositories without GitButler metadata show local branches instead of stacks. Each branch shows its committed changes since its merge-base with the selected base. Use **Base: …** to choose a local or remote base branch; the GUI remembers your choice per repository. You can also pass `--base <ref>` to the GUI, `--inspect`, or `--mcp`.

The automatic base for regular repositories is `origin/HEAD` when available, then local `main` or `master`, then the current branch (or the first local branch for a detached HEAD). GitButler repositories keep their configured target and stack boundaries. Stack/range selection applies to GitButler repositories.

Use **⌘K** to jump to a file, branch, or stack. **⌥← / ⌥→** or **⌥[ / ⌥]** goes back/forward through visited files.

## MCP

Add this to your MCP client configuration, replacing both paths:

```json
{
  "mcpServers": {
    "butler-diff": {
      "command": "/absolute/path/to/butler-diff/target/debug/butler-diff",
      "args": ["/path/to/repository", "--mcp"]
    }
  }
}
```

Agents can read stacks/local branches/diffs and create, reply to, edit, delete, or resolve comments. Changes appear live in the app. Restart the MCP connection after rebuilding.

Reviews committed changes only. Repo settings, viewed state, and comments live in Git’s common directory (normally `.git`); UI state lives in the app data directory. Working files and Git history are untouched.
