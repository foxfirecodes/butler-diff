use crate::{
    repository::{Repository, git},
    watch::RepoWatcher,
};
use gpui_kit::component::diff::DiffFile;
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

struct Fixture(PathBuf);
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "gander-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        git(&path, &["init", "--template=", "-q", "-b", "main"]).unwrap();
        git(&path, &["config", "user.name", "Test"]).unwrap();
        git(&path, &["config", "user.email", "test@example.invalid"]).unwrap();
        git(&path, &["config", "commit.gpgsign", "false"]).unwrap();
        git(&path, &["config", "core.hooksPath", "/dev/null"]).unwrap();
        Self(path)
    }
    fn commit(&self, file: &str, content: &str) -> String {
        std::fs::write(self.0.join(file), content).unwrap();
        git(&self.0, &["add", "--", file]).unwrap();
        git(&self.0, &["commit", "-qm", file]).unwrap();
        git(&self.0, &["rev-parse", "HEAD"]).unwrap().trim().into()
    }
    fn stacked() -> (Self, String, String, String) {
        let f = Self::new();
        let base = f.commit("base.txt", "base\n");
        git(&f.0, &["checkout", "-qb", "lower"]).unwrap();
        let lower = f.commit("lower file.txt", "lower only\n");
        git(&f.0, &["checkout", "-qb", "upper"]).unwrap();
        let upper = f.commit("upper.txt", "upper only\n");
        std::fs::create_dir_all(f.0.join(".git/gitbutler")).unwrap();
        std::fs::write(
            f.0.join(".git/gitbutler/virtual_branches.toml"),
            format!(
                r#"
[default_target]
sha = "{base}"
[branches.stack]
order = 0
in_workspace = true
[[branches.stack.heads]]
name = "lower"
head = {{ CommitId = "{lower}" }}
[[branches.stack.heads]]
name = "archived"
archived = true
head = {{ CommitId = "{base}" }}
[[branches.stack.heads]]
name = "upper"
head = {{ CommitId = "{upper}" }}
"#
            ),
        )
        .unwrap();
        (f, base, lower, upper)
    }
}

#[test]
fn mcp_setup_is_project_local_preserves_other_servers_and_is_repeatable() {
    use crate::mcp_setup::{self, Client};
    let fixture = Fixture::new();
    let project = fixture.0.join("nested-project");
    std::fs::create_dir_all(project.join(".codex")).unwrap();
    std::fs::write(
        project.join(".codex/config.toml"),
        "# my settings\nmodel = 'test-model'\n[mcp_servers.other]\ncommand = 'other-server'\n",
    )
    .unwrap();
    std::fs::write(
        project.join(".mcp.json"),
        r#"{"otherSetting":true,"mcpServers":{"other":{"command":"other-server"}}}"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for relative in [".codex/config.toml", ".mcp.json"] {
            std::fs::set_permissions(
                project.join(relative),
                std::fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
    }
    let executable = Path::new("/Applications/Gander.app/Contents/MacOS/gander");
    for client in [Client::Codex, Client::Claude] {
        let path = mcp_setup::install(&project, executable, client).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        mcp_setup::install(&project, executable, client).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(
            !first.contains(project.to_str().unwrap()),
            "project location must remain relative"
        );
    }
    let codex = std::fs::read_to_string(project.join(".codex/config.toml")).unwrap();
    assert!(codex.contains("# my settings"));
    let codex: toml::Value = toml::from_str(&codex).unwrap();
    assert_eq!(codex["model"].as_str(), Some("test-model"));
    assert_eq!(
        codex["mcp_servers"]["other"]["command"].as_str(),
        Some("other-server")
    );
    assert_eq!(codex["mcp_servers"]["gander"]["cwd"].as_str(), Some("."));
    assert_eq!(
        codex["mcp_servers"]["gander"]["args"].as_array().unwrap(),
        &vec![
            toml::Value::String("--mcp".into()),
            toml::Value::String(".".into())
        ]
    );
    let claude: serde_json::Value =
        serde_json::from_slice(&std::fs::read(project.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(claude["otherSetting"], true);
    assert_eq!(claude["mcpServers"]["other"]["command"], "other-server");
    assert_eq!(
        claude["mcpServers"]["gander"]["args"],
        serde_json::json!(["--mcp"])
    );
    assert_eq!(
        claude["mcpServers"]["gander"]["command"],
        executable.to_str().unwrap()
    );
    assert!(
        !fixture.0.join(".codex").exists(),
        "setup must stay in the selected project"
    );
}

#[test]
fn mcp_setup_rejects_invalid_existing_config_without_overwriting_it() {
    use crate::mcp_setup::{self, Client};
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.0.join(".codex")).unwrap();
    for (client, relative, invalid) in [
        (Client::Codex, ".codex/config.toml", "{"),
        (Client::Codex, ".codex/config.toml", "mcp_servers = []"),
        (Client::Claude, ".mcp.json", "{"),
        (Client::Claude, ".mcp.json", "null"),
        (Client::Claude, ".mcp.json", r#"{"mcpServers":[]}"#),
    ] {
        let path = fixture.0.join(relative);
        std::fs::write(&path, invalid).unwrap();
        assert!(mcp_setup::install(&fixture.0, Path::new("/gander"), client).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), invalid);
    }
}

#[test]
#[cfg(unix)]
fn copied_setup_commands_quote_executable_paths_without_running_shell_substitutions() {
    use crate::mcp_setup::{self, Client};
    use std::{os::unix::fs::PermissionsExt, process::Command};
    let fixture = Fixture::new();
    let executable = fixture
        .0
        .join("gander ' $(touch unexpected) `touch unexpected`");
    std::fs::write(&executable, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let command = mcp_setup::command(&executable, Client::Codex).unwrap();
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(&fixture.0)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "--setup-mcp\ncodex\n"
    );
    assert!(!fixture.0.join("unexpected").exists());
}

#[test]
fn review_windows_share_session_updates_and_serialize_saves_without_losing_other_repositories() {
    use crate::{Workspace, session::RepoState};
    let fixture = Fixture::new();
    let path = fixture.0.join("session.json");
    let first = Workspace::load(Some(path.clone()));
    let second = first.clone();
    first.session.borrow_mut().repositories.insert(
        "/first".into(),
        RepoState {
            wrap: true,
            ..Default::default()
        },
    );
    let store = first.store.as_ref().unwrap();
    let stale = store.next();
    let old_snapshot = first.session.borrow().clone();
    second.session.borrow_mut().repositories.insert(
        "/second".into(),
        RepoState {
            base_override: Some("main".into()),
            ..Default::default()
        },
    );
    let other_store = second.store.as_ref().unwrap();
    other_store
        .save(&second.session.borrow(), other_store.next())
        .unwrap();
    store.save(&old_snapshot, stale).unwrap();
    let saved = crate::session::Session::load(&path).unwrap();
    assert!(saved.repositories["/first"].wrap);
    assert_eq!(
        saved.repositories["/second"].base_override.as_deref(),
        Some("main")
    );
    assert_eq!(saved, *first.session.borrow());
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn paths(repo: &Repository, base: &str, tip: &str) -> Vec<String> {
    DiffFile::parse(&repo.patch(base, tip).unwrap())
        .unwrap()
        .into_iter()
        .map(|f| f.path().to_string())
        .collect()
}

#[test]
fn file_group_settings_persist_and_invalid_edits_do_not_overwrite_them() {
    use crate::file_groups::{CONFIG_FILE, GroupConfig, GroupDefinition, TEST_PATTERN};
    let (fixture, _, _, _) = Fixture::stacked();
    let repo = Repository::load(&fixture.0, None).unwrap();
    let path = repo.common_dir.join(CONFIG_FILE);
    assert_eq!(GroupConfig::load(&path).unwrap(), GroupConfig::default());
    let config = GroupConfig {
        enabled: false,
        groups: vec![GroupDefinition {
            name: "Tests".into(),
            pattern: TEST_PATTERN.into(),
        }],
    };
    config.save(&path).unwrap();
    assert_eq!(GroupConfig::load(&path).unwrap(), config);
    let before = std::fs::read(&path).unwrap();
    let mut invalid = config;
    invalid.groups[0].pattern = "[".into();
    assert!(invalid.save(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

fn parsed_files(repo: &Repository, base: &str, tip: &str) -> Vec<DiffFile> {
    DiffFile::parse(&repo.patch(base, tip).unwrap()).unwrap()
}

#[test]
fn review_threads_persist_reply_resolve_and_keep_immutable_anchors() {
    use crate::comments::{self, AnchorRequest, STATE_FILE, Side};
    let (fixture, base, lower, upper) = Fixture::stacked();
    let repo = Repository::load(&fixture.0, None).unwrap();
    let state = repo.common_dir.join(STATE_FILE);
    assert!(comments::list(&state).unwrap().is_empty());
    assert!(!state.exists());
    let anchor = comments::anchor(
        &repo,
        AnchorRequest {
            base: base.clone(),
            tip: lower.clone(),
            path: "lower file.txt".into(),
            side: Side::Modified,
            start_line: 1,
            end_line: 1,
        },
    )
    .unwrap();
    assert_eq!(anchor.excerpt, "lower only");
    assert!(comments::create(&state, &anchor, "Human", "  ").is_err());
    let thread = comments::create(&state, &anchor, "Human", "Is this intentional?").unwrap();
    let reply = comments::reply(&state, &thread.id, "Agent", "Yes, because…").unwrap();
    assert_eq!(reply.messages.len(), 2);
    assert_eq!(reply.messages[0].author, "Human");
    assert_eq!(reply.messages[1].author, "Agent");
    assert_ne!(reply.messages[0].id, reply.messages[1].id);
    assert!(
        comments::set_resolved(&state, &thread.id, true)
            .unwrap()
            .resolved
    );
    let reopened = comments::set_resolved(&state, &thread.id, false).unwrap();
    assert!(!reopened.resolved);
    assert_eq!(reopened.anchor.excerpt, anchor.excerpt);
    assert_eq!(reopened.messages.len(), 2);
    // The same file comparison is recognized across a wider stack range.
    let widened = parsed_files(&repo, &base, &upper);
    assert!(comments::matches(
        &thread,
        &base,
        &upper,
        widened
            .iter()
            .find(|file| file.path() == "lower file.txt")
            .unwrap()
    ));
    let changed = fixture.commit("lower file.txt", "different code\n");
    let files = parsed_files(&repo, &base, &changed);
    assert!(!comments::matches(
        &thread,
        &base,
        &changed,
        files
            .iter()
            .find(|file| file.path() == "lower file.txt")
            .unwrap()
    ));
    assert_eq!(
        comments::get(&state, &thread.id).unwrap().anchor.excerpt,
        "lower only"
    );
    assert!(comments::reply(&state, "missing", "Agent", "oops").is_err());
    assert!(comments::set_resolved(&state, "missing", true).is_err());
    assert_eq!(comments::list(&state).unwrap().len(), 1);
}

#[test]
fn review_anchors_validate_sides_lines_and_preserve_deleted_source() {
    use crate::comments::{self, AnchorRequest, Side};
    let (fixture, base, _, upper) = Fixture::stacked();
    git(&fixture.0, &["rm", "--", "base.txt"]).unwrap();
    git(&fixture.0, &["commit", "-qm", "delete source"]).unwrap();
    let tip = git(&fixture.0, &["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_owned();
    let repo = Repository::load(&fixture.0, None).unwrap();
    let request = AnchorRequest {
        base: upper,
        tip,
        path: "base.txt".into(),
        side: Side::Original,
        start_line: 1,
        end_line: 1,
    };
    assert_eq!(
        comments::anchor(&repo, request.clone()).unwrap().excerpt,
        "base"
    );
    let mut invalid = request.clone();
    invalid.side = Side::Modified;
    assert!(comments::anchor(&repo, invalid).is_err());
    for (start, end) in [(0, 1), (2, 1), (1, 502), (1, 2)] {
        let mut invalid = request.clone();
        invalid.start_line = start;
        invalid.end_line = end;
        assert!(comments::anchor(&repo, invalid).is_err());
    }
    let mut invalid = request;
    invalid.path = "missing.txt".into();
    assert!(comments::anchor(&repo, invalid).is_err());
    assert!(comments::resolve_commit(&repo, "--help").is_err());
    assert_eq!(comments::resolve_commit(&repo, &base).unwrap(), base);
}

#[test]
fn mcp_initializes_and_agents_can_create_read_reply_and_resolve() {
    use crate::{comments, mcp::Server};
    use serde_json::{Value, json};
    let (fixture, base, lower, _) = Fixture::stacked();
    let mut server = Server::new(fixture.0.clone(), None, None);
    let request = |id, method: &str, params: Value| json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
    assert_eq!(
        server.handle(request(0, "tools/list", json!({}))).unwrap()["error"]["code"],
        -32002
    );
    let init = server.handle(request(1, "initialize", json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test-agent","version":"1"}}))).unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(init["result"]["serverInfo"]["name"], "gander");
    assert!(
        server
            .handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .is_none()
    );
    let tools = server.handle(request(2, "tools/list", json!({}))).unwrap();
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 9);
    let call = |id, name: &str, arguments: Value| {
        request(id, "tools/call", json!({"name":name,"arguments":arguments}))
    };
    let stacks = server.handle(call(3, "list_stacks", json!({}))).unwrap();
    assert_eq!(stacks["result"]["isError"], false);
    let diff = server
        .handle(call(
            4,
            "get_diff",
            json!({"base":base,"tip":lower,"path":"lower file.txt"}),
        ))
        .unwrap();
    assert!(
        diff["result"]["structuredContent"]["patch"]
            .as_str()
            .unwrap()
            .contains("+lower only")
    );
    let created = server.handle(call(5,"create_thread",json!({"base":base,"tip":lower,"path":"lower file.txt","side":"modified","start_line":1,"end_line":1,"body":"Please explain this"}))).unwrap();
    assert_eq!(created["result"]["isError"], false, "{created}");
    let thread = &created["result"]["structuredContent"];
    let id = thread["id"].as_str().unwrap();
    assert_eq!(thread["messages"][0]["author"], "test-agent");
    let reply = server
        .handle(call(
            6,
            "reply_to_thread",
            json!({"thread_id":id,"body":"Here is why"}),
        ))
        .unwrap();
    assert_eq!(
        reply["result"]["structuredContent"]["messages"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let resolved = server
        .handle(call(
            7,
            "set_thread_resolved",
            json!({"thread_id":id,"resolved":true}),
        ))
        .unwrap();
    assert_eq!(resolved["result"]["structuredContent"]["resolved"], true);
    let listed = server
        .handle(call(8, "list_threads", json!({"resolved":false})))
        .unwrap();
    assert_eq!(listed["result"]["structuredContent"]["total"], 0);
    let message_id = thread["messages"][0]["id"].as_str().unwrap();
    let edited = server
        .handle(call(
            12,
            "edit_message",
            json!({"message_id":message_id,"body":"Edited by the agent"}),
        ))
        .unwrap();
    assert_eq!(
        edited["result"]["structuredContent"]["messages"][0]["body"],
        "Edited by the agent"
    );
    let deleted = server
        .handle(call(
            13,
            "set_message_deleted",
            json!({"message_id":message_id,"deleted":true}),
        ))
        .unwrap();
    assert_eq!(
        deleted["result"]["structuredContent"]["messages"][0]["body"],
        ""
    );
    assert!(!deleted["result"]["structuredContent"]["messages"][0]["deleted_at"].is_null());
    let restored = server
        .handle(call(
            14,
            "set_message_deleted",
            json!({"message_id":message_id,"deleted":false}),
        ))
        .unwrap();
    assert_eq!(
        restored["result"]["structuredContent"]["messages"][0]["body"],
        "Edited by the agent"
    );
    let invalid = server.handle(call(9,"create_thread",json!({"base":base,"tip":lower,"path":"lower file.txt","side":"modified","start_line":1,"end_line":1,"body":"bad","surprise":true}))).unwrap();
    assert_eq!(invalid["result"]["isError"], true);
    let repo = Repository::load(&fixture.0, None).unwrap();
    assert_eq!(
        comments::list(&repo.common_dir.join(comments::STATE_FILE))
            .unwrap()
            .len(),
        1
    );
    assert!(server.handle(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":10}})).is_none());
    assert_eq!(
        server.handle(request(11, "missing", json!({}))).unwrap()["error"]["code"],
        -32601
    );
}

#[test]
fn legacy_comments_migrate_and_edit_delete_restore_preserve_threads() {
    use crate::comments::{self, AnchorRequest, STATE_FILE, Side};
    let (fixture, base, lower, _) = Fixture::stacked();
    let repo = Repository::load(&fixture.0, None).unwrap();
    let state = repo.common_dir.join(STATE_FILE);
    let anchor = comments::anchor(
        &repo,
        AnchorRequest {
            base,
            tip: lower,
            path: "lower file.txt".into(),
            side: Side::Modified,
            start_line: 1,
            end_line: 1,
        },
    )
    .unwrap();
    {
        let db = Connection::open(&state).unwrap();
        db.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY,path TEXT NOT NULL,anchor TEXT NOT NULL,resolved INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
            CREATE TABLE messages(id TEXT PRIMARY KEY,thread_id TEXT NOT NULL REFERENCES threads(id),author TEXT NOT NULL,body TEXT NOT NULL,created_at INTEGER NOT NULL);
            PRAGMA user_version = 1;").unwrap();
        db.execute(
            "INSERT INTO threads VALUES ('thread','lower file.txt',?1,0,10,10)",
            [serde_json::to_string(&anchor).unwrap()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO messages VALUES ('message','thread','Original author','Original body',10)",
            [],
        )
        .unwrap();
    }
    let old = comments::get(&state, "thread").unwrap();
    assert!(old.messages[0].edited_at.is_none());
    assert!(old.messages[0].deleted_at.is_none());
    assert_eq!(
        Connection::open(&state)
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        1
    );
    assert!(comments::edit(&state, "message", "  ").is_err());
    let edited = comments::edit(&state, "message", "Corrected body").unwrap();
    assert_eq!(edited.messages[0].author, "Original author");
    assert_eq!(edited.messages[0].created_at, 10);
    assert_eq!(edited.messages[0].body, "Corrected body");
    assert!(edited.messages[0].edited_at.is_some());
    assert_eq!(
        Connection::open(&state)
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        2
    );
    comments::reply(&state, "thread", "Agent", "Reply survives deletion").unwrap();
    let deleted = comments::set_deleted(&state, "message", true).unwrap();
    assert!(deleted.messages[0].body.is_empty());
    assert!(deleted.messages[0].deleted_at.is_some());
    assert_eq!(deleted.messages.len(), 2);
    assert!(
        !serde_json::to_string(&deleted)
            .unwrap()
            .contains("Corrected body")
    );
    assert!(comments::edit(&state, "message", "Can't overwrite deleted text").is_err());
    assert!(comments::set_deleted(&state, "missing", true).is_err());
    let restored = comments::set_deleted(&state, "message", false).unwrap();
    assert_eq!(restored.messages[0].body, "Corrected body");
    assert_eq!(restored.messages[0].edited_at, edited.messages[0].edited_at);
    assert!(restored.messages[0].deleted_at.is_none());
    assert_eq!(restored.messages[1].body, "Reply survives deletion");
    assert_eq!(restored.anchor.file_version, old.anchor.file_version);
}

#[test]
fn concurrent_agent_replies_are_kept_and_notify_only_comments() {
    use crate::comments::{self, AnchorRequest, STATE_FILE, Side};
    let (fixture, base, lower, _) = Fixture::stacked();
    let repo = Repository::load(&fixture.0, None).unwrap();
    let watcher = RepoWatcher::new(&repo).unwrap();
    let state = repo.common_dir.join(STATE_FILE);
    let anchor = comments::anchor(
        &repo,
        AnchorRequest {
            base,
            tip: lower,
            path: "lower file.txt".into(),
            side: Side::Modified,
            start_line: 1,
            end_line: 1,
        },
    )
    .unwrap();
    let thread = comments::create(&state, &anchor, "Human", "Please investigate").unwrap();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..6)
            .map(|i| {
                let state = &state;
                let id = &thread.id;
                scope.spawn(move || {
                    comments::reply(state, id, &format!("agent-{i}"), &format!("Reply {i}"))
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
    });
    let saved = comments::get(&state, &thread.id).unwrap();
    assert_eq!(saved.messages.len(), 7);
    assert_eq!(
        saved
            .messages
            .iter()
            .map(|m| m.id.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        7
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !watcher.events.is_pending() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(30));
    }
    let changes = watcher.events.take_changes();
    assert!(changes.comments_changed());
    assert!(changes.without_local_settings().is_empty());
}

#[test]
fn group_viewed_batches_follow_filters_and_preserve_other_groups() {
    use crate::{
        file_groups::{self, GroupConfig, GroupDefinition},
        viewed::{self, FileVersion, STATE_FILE},
    };
    let (fixture, base, _, _) = Fixture::stacked();
    let tip = fixture.commit("outside.txt", "another group\n");
    let repo = Repository::load(&fixture.0, None).unwrap();
    let files = parsed_files(&repo, &base, &tip);
    let groups = GroupConfig {
        enabled: true,
        groups: vec![GroupDefinition {
            name: "Changed".into(),
            pattern: r"^(lower file|upper)".into(),
        }],
    }
    .compile()
    .unwrap();
    let state = repo.common_dir.join(STATE_FILE);
    let versions = |key: &str, query: &str| {
        file_groups::buckets(&files, query, &groups)
            .into_iter()
            .find(|bucket| bucket.key == key)
            .unwrap()
            .files
            .iter()
            .map(|file| {
                (
                    file.path().to_string(),
                    FileVersion::resolve(&repo, &base, &tip, file).unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    let unrelated = versions("ungrouped", "");
    let all = versions("group:Changed", "");
    let filtered = versions("group:Changed", "lower");
    assert_eq!(all.len(), 2);
    assert_eq!(filtered.len(), 1);
    viewed::set_many(&state, &unrelated, true).unwrap();
    viewed::set_many(&state, &filtered, true).unwrap();
    let saved = viewed::load(&repo, &base, &tip, &files).unwrap();
    assert!(saved.contains("lower file.txt"));
    assert!(!saved.contains("upper.txt"));
    assert!(saved.contains("outside.txt"));
    // Checking a partially viewed group completes it without disturbing other groups.
    viewed::set_many(&state, &all, true).unwrap();
    assert_eq!(viewed::load(&repo, &base, &tip, &files).unwrap().len(), 3);
    viewed::set_many(&state, &filtered, false).unwrap();
    let saved = viewed::load(&repo, &base, &tip, &files).unwrap();
    assert!(!saved.contains("lower file.txt"));
    assert!(saved.contains("upper.txt"));
    viewed::set_many(&state, &all, false).unwrap();
    assert_eq!(
        viewed::load(&repo, &base, &tip, &files).unwrap(),
        std::collections::HashSet::from(["outside.txt".into()])
    );
}

#[test]
fn group_viewed_batches_roll_back_on_mark_and_unmark_failures() {
    use crate::viewed::{self, FileVersion, STATE_FILE};
    let (fixture, base, _, tip) = Fixture::stacked();
    let repo = Repository::load(&fixture.0, None).unwrap();
    let files = parsed_files(&repo, &base, &tip);
    let versions: Vec<_> = files
        .iter()
        .map(|file| {
            (
                file.path().to_string(),
                FileVersion::resolve(&repo, &base, &tip, file).unwrap(),
            )
        })
        .collect();
    let state = repo.common_dir.join(STATE_FILE);
    viewed::set_many(&state, &[], true).unwrap();
    assert!(!state.exists());
    viewed::set_many(&state, &versions, true).unwrap();
    let db = Connection::open(&state).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_unmark BEFORE DELETE ON viewed_files
        WHEN OLD.path = 'upper.txt' BEGIN SELECT RAISE(ABORT, 'test write failure'); END;",
    )
    .unwrap();
    assert!(viewed::set_many(&state, &versions, false).is_err());
    assert_eq!(viewed::load(&repo, &base, &tip, &files).unwrap().len(), 2);
    db.execute_batch("DROP TRIGGER reject_unmark;").unwrap();
    viewed::set_many(&state, &versions, false).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_mark BEFORE INSERT ON viewed_files
        WHEN NEW.path = 'upper.txt' BEGIN SELECT RAISE(ABORT, 'test write failure'); END;",
    )
    .unwrap();
    assert!(viewed::set_many(&state, &versions, true).is_err());
    assert!(viewed::load(&repo, &base, &tip, &files).unwrap().is_empty());
}

#[test]
fn viewed_files_persist_unmark_and_survive_unrelated_commits() {
    use crate::viewed::{self, FileVersion, STATE_FILE};
    let (fixture, base, _, upper) = Fixture::stacked();
    let repo = Repository::load(&fixture.0, None).unwrap();
    let files = parsed_files(&repo, &base, &upper);
    let file = files
        .iter()
        .find(|f| f.path().as_str() == "lower file.txt")
        .unwrap();
    let version = FileVersion::resolve(&repo, &base, &upper, file).unwrap();
    assert!(
        FileVersion::from_patch(file).is_some(),
        "normal diffs must carry full blob IDs"
    );
    let state = repo.common_dir.join(STATE_FILE);
    assert!(
        viewed::load(&repo, &base, &upper, &files)
            .unwrap()
            .is_empty()
    );
    assert!(
        !state.exists(),
        "reading an unreviewed repo must not create state"
    );
    viewed::set(&state, file.path(), &version, true).unwrap();
    assert!(
        viewed::load(&repo, &base, &upper, &files)
            .unwrap()
            .contains("lower file.txt")
    );
    let tip = fixture.commit("unrelated.txt", "unrelated\n");
    let changed = parsed_files(&repo, &base, &tip);
    assert_eq!(viewed::load(&repo, &base, &tip, &changed).unwrap().len(), 1);
    viewed::set(&state, file.path(), &version, false).unwrap();
    assert!(
        viewed::load(&repo, &base, &tip, &changed)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn viewed_files_reset_when_either_side_of_the_comparison_changes() {
    use crate::viewed::{self, FileVersion, STATE_FILE};
    let (fixture, base, _, upper) = Fixture::stacked();
    let repo = Repository::load(&fixture.0, None).unwrap();
    let files = parsed_files(&repo, &base, &upper);
    let file = files
        .iter()
        .find(|f| f.path().as_str() == "lower file.txt")
        .unwrap();
    let version = FileVersion::resolve(&repo, &base, &upper, file).unwrap();
    viewed::set(
        &repo.common_dir.join(STATE_FILE),
        file.path(),
        &version,
        true,
    )
    .unwrap();
    let changed = fixture.commit("lower file.txt", "changed content\n");
    assert!(
        !viewed::load(
            &repo,
            &base,
            &changed,
            &parsed_files(&repo, &base, &changed)
        )
        .unwrap()
        .contains("lower file.txt")
    );
    // Same tip blob as the original review, but a different base changes its diff.
    assert!(
        !viewed::load(
            &repo,
            &changed,
            &upper,
            &parsed_files(&repo, &changed, &upper)
        )
        .unwrap()
        .contains("lower file.txt")
    );
}

#[test]
fn normal_viewed_checks_use_patch_ids_and_unviewed_files_need_no_hash_lookup() {
    use crate::viewed::{self, FileVersion, STATE_FILE};
    let (fixture, base, _, upper) = Fixture::stacked();
    let mut repo = Repository::load(&fixture.0, None).unwrap();
    let mut files = parsed_files(&repo, &base, &upper);
    let file = files
        .iter()
        .find(|f| f.path().as_str() == "lower file.txt")
        .unwrap();
    let version = FileVersion::resolve(&repo, &base, &upper, file).unwrap();
    viewed::set(
        &repo.common_dir.join(STATE_FILE),
        file.path(),
        &version,
        true,
    )
    .unwrap();
    let metadata_only = DiffFile::parse("diff --git a/old.txt b/new.txt\nsimilarity index 100%\nrename from old.txt\nrename to new.txt\n").unwrap();
    assert!(FileVersion::from_patch(&metadata_only[0]).is_none());
    files.extend(metadata_only);
    // Any accidental Git hash lookup fails here. Neither the marked normal
    // file nor the unmarked metadata-only file should make one.
    repo.root = fixture.0.join("no-such-repository");
    let viewed = viewed::load(&repo, &base, &upper, &files).unwrap();
    assert_eq!(
        viewed.into_iter().collect::<Vec<_>>(),
        vec!["lower file.txt"]
    );
}

#[test]
fn viewed_pure_renames_resolve_hashes_lazily_and_reset_for_new_content() {
    use crate::viewed::{self, FileVersion, STATE_FILE};
    let (fixture, _, _, original) = Fixture::stacked();
    let repo = Repository::load(&fixture.0, None).unwrap();
    git(
        &fixture.0,
        &["mv", "--", "lower file.txt", "renamed [file].txt"],
    )
    .unwrap();
    git(&fixture.0, &["commit", "-qm", "rename"]).unwrap();
    let renamed = git(&fixture.0, &["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_owned();
    let files = parsed_files(&repo, &original, &renamed);
    assert_eq!(files.len(), 1);
    assert!(FileVersion::from_patch(&files[0]).is_none());
    let version = FileVersion::resolve(&repo, &original, &renamed, &files[0]).unwrap();
    viewed::set(
        &repo.common_dir.join(STATE_FILE),
        files[0].path(),
        &version,
        true,
    )
    .unwrap();
    assert_eq!(
        viewed::load(&repo, &original, &renamed, &files)
            .unwrap()
            .len(),
        1
    );
    // Identical rename headers, different blobs on both sides.
    git(&fixture.0, &["checkout", "-q", &original]).unwrap();
    let base2 = fixture.commit("lower file.txt", "a different file body\n");
    git(
        &fixture.0,
        &["mv", "--", "lower file.txt", "renamed [file].txt"],
    )
    .unwrap();
    git(&fixture.0, &["commit", "-qm", "rename new content"]).unwrap();
    let tip2 = git(&fixture.0, &["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_owned();
    assert!(
        viewed::load(&repo, &base2, &tip2, &parsed_files(&repo, &base2, &tip2))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn viewed_metadata_and_binary_changes_have_distinct_versions() {
    use crate::viewed::{self, FileVersion, STATE_FILE};
    let (fixture, _, _, original) = Fixture::stacked();
    let repo = Repository::load(&fixture.0, None).unwrap();
    git(
        &fixture.0,
        &["update-index", "--chmod=+x", "--", "lower file.txt"],
    )
    .unwrap();
    git(&fixture.0, &["commit", "-qm", "mode only"]).unwrap();
    let mode = git(&fixture.0, &["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_owned();
    let files = parsed_files(&repo, &original, &mode);
    assert!(FileVersion::from_patch(&files[0]).is_none());
    let version = FileVersion::resolve(&repo, &original, &mode, &files[0]).unwrap();
    viewed::set(
        &repo.common_dir.join(STATE_FILE),
        files[0].path(),
        &version,
        true,
    )
    .unwrap();
    assert_eq!(
        viewed::load(&repo, &original, &mode, &files).unwrap().len(),
        1
    );
    assert!(
        viewed::load(
            &repo,
            &mode,
            &original,
            &parsed_files(&repo, &mode, &original)
        )
        .unwrap()
        .is_empty()
    );
    let binary = fixture.commit("binary.bin", "\0one\0");
    let files = parsed_files(&repo, &mode, &binary);
    let file = files
        .iter()
        .find(|f| f.path().as_str() == "binary.bin")
        .unwrap();
    assert!(file.is_binary());
    assert!(FileVersion::from_patch(file).is_some());
    let version = FileVersion::resolve(&repo, &mode, &binary, file).unwrap();
    viewed::set(
        &repo.common_dir.join(STATE_FILE),
        file.path(),
        &version,
        true,
    )
    .unwrap();
    let changed = fixture.commit("binary.bin", "\0two\0");
    assert!(
        !viewed::load(
            &repo,
            &mode,
            &changed,
            &parsed_files(&repo, &mode, &changed)
        )
        .unwrap()
        .contains("binary.bin")
    );
}

#[test]
fn branch_diffs_exclude_lower_branches_and_range_includes_both() {
    let (f, base, lower, upper) = Fixture::stacked();
    let repo = Repository::load(&f.0, None).unwrap();
    let branches = &repo.stacks[0].branches;
    assert_eq!(branches.len(), 2); // archived heads do not become branch boundaries
    assert_eq!(
        (branches[0].base().unwrap(), &branches[0].tip),
        (base.as_str(), &lower)
    );
    assert_eq!(
        (branches[1].base().unwrap(), &branches[1].tip),
        (lower.as_str(), &upper)
    );
    assert_eq!(paths(&repo, &lower, &upper), vec!["upper.txt"]);
    assert_eq!(
        paths(&repo, &base, &upper),
        vec!["lower file.txt", "upper.txt"]
    );
    assert_eq!(paths(&repo, &base, &lower), vec!["lower file.txt"]);
}

#[test]
fn resolves_live_refs_after_a_commit_and_refreshes_from_nested_directory() {
    let (f, _, lower, _) = Fixture::stacked();
    let tip = f.commit("upper.txt", "new content\n");
    std::fs::create_dir(f.0.join("nested")).unwrap();
    let repo = Repository::load(&f.0.join("nested"), None).unwrap();
    assert_eq!(repo.stacks[0].branches[1].tip, tip);
    assert_eq!(repo.stacks[0].branches[1].base().unwrap(), lower);
    let patch = repo.patch(&lower, &tip).unwrap();
    assert!(patch.contains("+new content"));
}

#[test]
fn migrated_repository_uses_sqlite_instead_of_stale_toml() {
    let (f, base, lower, upper) = Fixture::stacked();
    git(&f.0, &["config", "gitbutler.project.portedmeta", "true"]).unwrap();
    git(&f.0, &["config", "gitbutler.project.targetcommitid", &base]).unwrap();
    let db = Connection::open(f.0.join(".git/gitbutler/but.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE vb_stacks(id TEXT, sort_order INTEGER, in_workspace INTEGER);
        CREATE TABLE vb_stack_heads(stack_id TEXT, position INTEGER, name TEXT, head_sha TEXT, pr_number INTEGER, archived INTEGER);
        CREATE TABLE vb_branch_targets(stack_id TEXT, sha TEXT);
        INSERT INTO vb_stacks VALUES('actual', 0, 1);").unwrap();
    db.execute(
        "INSERT INTO vb_stack_heads VALUES('actual', 0, 'lower', ?1, 42, 0)",
        [&lower],
    )
    .unwrap();
    db.execute(
        "INSERT INTO vb_stack_heads VALUES('actual', 1, 'upper', ?1, NULL, 0)",
        [&upper],
    )
    .unwrap();
    let before = std::fs::read(f.0.join(".git/gitbutler/but.sqlite")).unwrap();
    let repo = Repository::load(&f.0, None).unwrap();
    assert_eq!(repo.source, "GitButler SQLite");
    assert_eq!(repo.stacks[0].id, "actual");
    assert_eq!(repo.stacks[0].branches[0].review.as_deref(), Some("#42"));
    assert_eq!(
        before,
        std::fs::read(f.0.join(".git/gitbutler/but.sqlite")).unwrap()
    );
    let signature = repo.current_metadata_signature().unwrap();
    db.execute_batch(
        "CREATE TABLE cache_noise(value TEXT); INSERT INTO cache_noise VALUES('heartbeat');",
    )
    .unwrap();
    assert_eq!(
        repo.current_metadata_signature().unwrap(),
        signature,
        "unrelated DB writes must not reload Git or redraw the UI"
    );
    db.execute(
        "UPDATE vb_stack_heads SET pr_number = 43 WHERE name = 'lower'",
        [],
    )
    .unwrap();
    assert_ne!(
        repo.current_metadata_signature().unwrap(),
        signature,
        "actual stack changes must still refresh"
    );
    std::fs::remove_file(f.0.join(".git/gitbutler/but.sqlite")).unwrap();
    assert!(Repository::load(&f.0, None).is_err()); // no fallback to stale metadata
}

#[test]
fn notify_detects_ref_changes() {
    let (f, _, _, _) = Fixture::stacked();
    let repo = Repository::load(&f.0, None).unwrap();
    let watcher = RepoWatcher::new(&repo).unwrap();
    let next = f.commit("upper.txt", "watch me\n");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !watcher.events.is_pending() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        watcher.events.is_pending(),
        "notify did not detect the commit"
    );
    assert_eq!(
        Repository::load(&f.0, None).unwrap().stacks[0].branches[1].tip,
        next
    );
}

#[test]
fn patch_handles_renames_binary_files_and_deleted_files() {
    let f = Fixture::new();
    let base = f.commit("old.txt", "same source\n");
    git(&f.0, &["mv", "old.txt", "new name.txt"]).unwrap();
    std::fs::write(f.0.join("binary.bin"), [0u8, 1, 2, 3]).unwrap();
    git(&f.0, &["add", "."]).unwrap();
    git(&f.0, &["commit", "-qm", "rename and binary"]).unwrap();
    let tip = git(&f.0, &["rev-parse", "HEAD"]).unwrap();
    let patch = git(
        &f.0,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--find-renames",
            &base,
            tip.trim(),
            "--",
        ],
    )
    .unwrap();
    let files = DiffFile::parse(&patch).unwrap();
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(DiffFile::is_binary));
    assert!(files.iter().any(|f| f.path().as_str() == "new name.txt"));
    std::fs::remove_file(f.0.join("new name.txt")).unwrap();
    git(&f.0, &["add", "-u"]).unwrap();
    git(&f.0, &["commit", "-qm", "delete"]).unwrap();
    let patch = git(
        &f.0,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            tip.trim(),
            "HEAD",
            "--",
        ],
    )
    .unwrap();
    assert_eq!(DiffFile::parse(&patch).unwrap()[0].modified_path(), None);
}

#[test]
fn ordinary_repository_lists_independent_local_branches_and_uses_merge_bases() {
    let f = Fixture::new();
    let base = f.commit("base.txt", "base\n");
    git(&f.0, &["checkout", "-qb", "feature"]).unwrap();
    let tip = f.commit("feature.txt", "feature\n");
    git(&f.0, &["checkout", "-q", "main"]).unwrap();
    f.commit("main-only.txt", "base advanced\n");
    git(&f.0, &["checkout", "-q", "feature"]).unwrap();
    git(
        &f.0,
        &["update-ref", "refs/remotes/origin/remote-only", &tip],
    )
    .unwrap();
    let repo = Repository::load(&f.0, None).unwrap();
    assert!(repo.is_local());
    assert_eq!(repo.target_ref, "refs/heads/main");
    assert_eq!(repo.stacks.len(), 2);
    assert!(repo.stacks.iter().all(|stack| stack.branches.len() == 1));
    let feature = &repo.stacks[0].branches[0];
    assert_eq!(feature.name, "feature"); // Initially select the checked-out branch.
    assert_eq!(feature.base().unwrap(), base);
    assert_eq!(feature.tip, tip);
    assert_eq!(
        paths(&repo, feature.base().unwrap(), &feature.tip),
        vec!["feature.txt"]
    );
    let refs = Repository::base_refs(&f.0).unwrap();
    assert!(refs.contains(&"refs/heads/main".into()));
    assert!(refs.contains(&"refs/remotes/origin/remote-only".into()));
    let json = serde_json::to_value(feature).unwrap();
    assert_eq!(json["base"], base); // Keep the existing MCP endpoint shape.
    assert!(json.get("error").is_none());
}

#[test]
fn local_base_override_changes_comparisons_and_invalid_bases_are_actionable() {
    let f = Fixture::new();
    let base = f.commit("base.txt", "base\n");
    git(&f.0, &["checkout", "-qb", "lower"]).unwrap();
    let lower = f.commit("lower.txt", "lower\n");
    git(&f.0, &["checkout", "-qb", "upper"]).unwrap();
    let upper = f.commit("upper.txt", "upper\n");
    let auto = Repository::load(&f.0, None).unwrap();
    assert_eq!(auto.stacks[0].branches[0].base().unwrap(), base);
    let selected = Repository::load(&f.0, Some("refs/heads/lower")).unwrap();
    assert_eq!(selected.target, lower);
    assert_eq!(selected.target_ref, "refs/heads/lower");
    assert_eq!(selected.stacks[0].branches[0].base().unwrap(), lower);
    assert_eq!(paths(&selected, &lower, &upper), vec!["upper.txt"]);
    let error = Repository::load(&f.0, Some("missing")).unwrap_err();
    assert!(format!("{error:#}").contains("Choose a base branch or pass --base"));
}

#[test]
fn automatic_local_base_prefers_remote_default_and_supports_packed_refs() {
    let f = Fixture::new();
    let base = f.commit("base.txt", "base\n");
    git(&f.0, &["checkout", "-qb", "feature"]).unwrap();
    let tip = f.commit("feature.txt", "feature\n");
    git(&f.0, &["update-ref", "refs/remotes/origin/trunk", &base]).unwrap();
    git(
        &f.0,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ],
    )
    .unwrap();
    git(&f.0, &["pack-refs", "--all"]).unwrap();
    let repo = Repository::load(&f.0, None).unwrap();
    assert_eq!(repo.target_ref, "refs/remotes/origin/trunk");
    assert_eq!(repo.stacks[0].branches[0].tip, tip);
    assert!(
        !Repository::base_refs(&f.0)
            .unwrap()
            .contains(&"refs/remotes/origin/HEAD".into())
    );
    assert_eq!(
        Repository::load(&f.0, Some("main")).unwrap().target_ref,
        "main"
    );
}

#[test]
fn local_repositories_support_unborn_and_detached_heads_and_nonstandard_bases() {
    let f = Fixture::new();
    let empty = Repository::load(&f.0, None).unwrap();
    assert!(empty.is_local());
    assert!(empty.stacks.is_empty());
    assert!(empty.target.is_empty());
    assert!(Repository::base_refs(&f.0).unwrap().is_empty());
    f.commit("a", "a\n");
    git(&f.0, &["branch", "-m", "trunk"]).unwrap();
    assert_eq!(
        Repository::load(&f.0, None).unwrap().target_ref,
        "refs/heads/trunk"
    );
    git(&f.0, &["checkout", "--detach", "-q"]).unwrap();
    assert_eq!(
        Repository::load(&f.0, None).unwrap().target_ref,
        "refs/heads/trunk"
    );
}

#[test]
fn unrelated_local_branch_does_not_block_other_reviews() {
    let f = Fixture::new();
    f.commit("main.txt", "main\n");
    git(&f.0, &["checkout", "--orphan", "unrelated", "-q"]).unwrap();
    git(&f.0, &["rm", "-q", "-rf", "."]).unwrap();
    f.commit("other.txt", "other\n");
    let repo = Repository::load(&f.0, None).unwrap();
    let unrelated = &repo.stacks[0].branches[0];
    assert_eq!(unrelated.name, "unrelated");
    assert!(
        unrelated
            .base()
            .unwrap_err()
            .to_string()
            .contains("shared history")
    );
    assert!(repo.stacks[1].branches[0].base().is_ok());
    let json = serde_json::to_value(unrelated).unwrap();
    assert!(json.get("base").is_none());
    assert!(json["error"].as_str().unwrap().contains("shared history"));
    assert!(
        Repository::load(&f.0, Some("unrelated")).unwrap().stacks[0].branches[0]
            .base()
            .is_ok()
    );
}

#[test]
fn invalid_gitbutler_metadata_does_not_silently_fall_back_to_local_branches() {
    let (f, _, _, _) = Fixture::stacked();
    std::fs::write(f.0.join(".git/gitbutler/virtual_branches.toml"), "{").unwrap();
    let error = Repository::load(&f.0, None).unwrap_err();
    assert!(format!("{error:#}").contains("Couldn't parse GitButler"));
}

#[test]
fn local_worktrees_watch_shared_refs_and_reconcile_branch_creation_and_deletion() {
    let f = Fixture::new();
    f.commit("base.txt", "base\n");
    let worktree = f.0.join("review-worktree");
    git(
        &f.0,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            worktree.to_str().unwrap(),
        ],
    )
    .unwrap();
    let repo = Repository::load(&worktree, None).unwrap();
    assert!(repo.is_local());
    assert_ne!(repo.git_dir, repo.common_dir);
    assert_eq!(repo.stacks[0].branches[0].name, "feature");
    let watcher = RepoWatcher::new(&repo).unwrap();
    git(&f.0, &["branch", "new-branch"]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !watcher.events.is_pending() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        watcher.events.is_pending(),
        "watcher missed a shared branch ref"
    );
    let updated = Repository::load(&worktree, None).unwrap();
    assert!(
        updated
            .stacks
            .iter()
            .any(|stack| stack.branches[0].name == "new-branch")
    );
    git(&f.0, &["branch", "-D", "new-branch"]).unwrap();
    let updated = Repository::load(&worktree, None).unwrap();
    assert!(
        !updated
            .stacks
            .iter()
            .any(|stack| stack.branches[0].name == "new-branch")
    );
}

#[test]
#[ignore = "set GANDER_TEST_REPO to smoke-test an existing repository read-only"]
fn live_repository_diffs() {
    let path = std::env::var_os("GANDER_TEST_REPO")
        .or_else(|| std::env::var_os("BUTLER_DIFF_TEST_REPO"))
        .expect("set GANDER_TEST_REPO");
    let repo = Repository::load(Path::new(&path), None).unwrap();
    let mut branches = 0;
    let mut files = 0;
    for stack in &repo.stacks {
        for branch in &stack.branches {
            git(
                &repo.root,
                &[
                    "merge-base",
                    "--is-ancestor",
                    branch.base().unwrap(),
                    &branch.tip,
                ],
            )
            .unwrap_or_else(|e| panic!("{} has a divergent lower boundary: {e}", branch.name));
            let patch = repo.patch(branch.base().unwrap(), &branch.tip).unwrap();
            let parsed = DiffFile::parse(&patch).unwrap_or_else(|e| panic!("{}: {e}", branch.name));
            branches += 1;
            files += parsed.len();
        }
        let first = stack.branches.first().unwrap();
        let last = stack.branches.last().unwrap();
        DiffFile::parse(&repo.patch(first.base().unwrap(), &last.tip).unwrap()).unwrap();
    }
    println!(
        "Validated {} stacks, {branches} branch diffs, {files} changed-file entries, and all combined stack diffs",
        repo.stacks.len()
    );
}
