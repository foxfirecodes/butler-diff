# butler diff

A native code review app for [GitButler](https://gitbutler.com/) stacks, built with Rust and [GPUI Kit](https://gpui-kit.com/).

- Review individual branches, entire stacks, or branch ranges.
- Syntax-highlighted split/unified diffs with automatic refresh.
- Regex file groups, filters, and per-group file/LOC counts.
- Persistent viewed files that reset when their diff changes.
- Remembers your repo, branch range, collapsed files/groups, view options, pane widths, and last file.
- Inline comments, replies, edit/delete/undo, and live agent collaboration over MCP.

## Run

Requires Rust and Git.

```sh
cargo run -- /path/to/repository
```

On macOS, `sh scripts/bundle-macos.sh` builds `target/debug/Butler Diff.app`.

Click a branch, click a stack’s **view all**, or **shift-click** another branch to select a range. Use **Groups…** to organize files. **⌘Enter** submits comments; **Escape** closes the composer and keeps a draft for this session.

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

Agents can read stacks/diffs and create, reply to, edit, delete, or resolve comments. Changes appear live in the app. Restart the MCP connection after rebuilding.

Reviews committed changes only. Settings, viewed state, and comments stay local in Git’s common directory (normally `.git`). Working files and Git history are untouched.
