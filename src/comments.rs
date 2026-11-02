use crate::{
    repository::{Repository, git},
    viewed::FileVersion,
};
use anyhow::{Context, Result, bail};
use gpui_kit::component::diff::{DiffFile, DiffSide};
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

// Keep the on-disk identity stable so Gander can read existing reviews.
pub const STATE_FILE: &str = "butler-diff-comments.sqlite";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Original,
    Modified,
}

impl From<DiffSide> for Side {
    fn from(side: DiffSide) -> Self {
        match side {
            DiffSide::Original => Self::Original,
            DiffSide::Modified => Self::Modified,
        }
    }
}
impl From<Side> for DiffSide {
    fn from(side: Side) -> Self {
        match side {
            Side::Original => Self::Original,
            Side::Modified => Self::Modified,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct AnchorRequest {
    pub base: String,
    pub tip: String,
    pub path: String,
    pub side: Side,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Anchor {
    #[serde(flatten)]
    pub location: AnchorRequest,
    pub file_version: String,
    pub excerpt: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Message {
    pub id: String,
    pub author: String,
    pub body: String,
    pub created_at: i64,
    #[serde(default)]
    pub edited_at: Option<i64>,
    #[serde(default)]
    pub deleted_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Thread {
    pub id: String,
    pub anchor: Anchor,
    pub resolved: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub messages: Vec<Message>,
}

pub fn resolve_commit(repo: &Repository, reference: &str) -> Result<String> {
    Ok(git(
        &repo.root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{reference}^{{commit}}"),
        ],
    )?
    .trim()
    .to_owned())
}

pub fn anchor(repo: &Repository, mut request: AnchorRequest) -> Result<Anchor> {
    if request.start_line == 0
        || request.end_line < request.start_line
        || request.end_line - request.start_line > 500
    {
        bail!("Choose a one-based line range of at most 501 lines on one side of the diff.");
    }
    request.base = resolve_commit(repo, &request.base)?;
    request.tip = resolve_commit(repo, &request.tip)?;
    let patch = repo.patch(&request.base, &request.tip)?;
    let files = DiffFile::parse(&patch).map_err(|e| anyhow::anyhow!("Couldn't parse diff: {e}"))?;
    let file = files
        .iter()
        .find(|file| file.path().as_str() == request.path)
        .context("File is not in this comparison. Use get_diff to find a changed path.")?;
    if file.is_binary() {
        bail!("Inline comments need a text file.");
    }
    let (commit, source_path) = match request.side {
        Side::Original => (&request.base, file.original_path()),
        Side::Modified => (&request.tip, file.modified_path()),
    };
    let source_path = source_path.context("That side of the file does not exist.")?;
    let source = git(&repo.root, &["show", &format!("{commit}:{source_path}")])?;
    let lines: Vec<_> = source.lines().collect();
    if request.end_line > lines.len() {
        bail!("The file has {} lines on that side.", lines.len());
    }
    let excerpt = lines[request.start_line - 1..request.end_line].join("\n");
    if excerpt.len() > 64 * 1024 {
        bail!("Selected source exceeds 64 KiB; choose fewer lines.");
    }
    Ok(Anchor {
        file_version: FileVersion::resolve(repo, &request.base, &request.tip, file)?.key()?,
        location: request,
        excerpt,
    })
}

pub fn matches(thread: &Thread, base: &str, tip: &str, file: &DiffFile) -> bool {
    thread.anchor.location.path == file.path().as_str()
        && FileVersion::from_patch(file)
            .and_then(|v| v.key().ok())
            .map_or_else(
                || thread.anchor.location.base == base && thread.anchor.location.tip == tip,
                |version| version == thread.anchor.file_version,
            )
}

fn open(path: &Path, write: bool) -> Result<Connection> {
    let flags = if write {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let mut db =
        Connection::open_with_flags(path, flags).context("Couldn't open review comments")?;
    db.busy_timeout(Duration::from_secs(3))?;
    db.pragma_update(None, "foreign_keys", true)?;
    let version: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > 2 {
        bail!("Review comments use a newer format ({version}).");
    }
    if write && version < 2 {
        db.pragma_update(None, "journal_mode", "WAL")?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: u32 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS threads (
            id TEXT PRIMARY KEY, path TEXT NOT NULL, anchor TEXT NOT NULL,
            resolved INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS threads_path ON threads(path);
            CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY, thread_id TEXT NOT NULL REFERENCES threads(id),
            author TEXT NOT NULL, body TEXT NOT NULL, created_at INTEGER NOT NULL,
            edited_at INTEGER, deleted_at INTEGER);
            CREATE INDEX IF NOT EXISTS messages_thread ON messages(thread_id);")?;
        if current == 1 {
            tx.execute_batch("ALTER TABLE messages ADD COLUMN edited_at INTEGER; ALTER TABLE messages ADD COLUMN deleted_at INTEGER;")?;
        }
        tx.pragma_update(None, "user_version", 2)?;
        tx.commit()?;
    }
    Ok(db)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn id(db: &Connection) -> Result<String> {
    Ok(db.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?)
}
fn validate_message(author: &str, body: &str) -> Result<()> {
    if author.trim().is_empty() || author.len() > 200 {
        bail!("Author must contain 1–200 bytes.");
    }
    if body.trim().is_empty() || body.len() > 64 * 1024 {
        bail!("Comment must contain 1–65536 bytes.");
    }
    Ok(())
}

pub fn create(path: &Path, anchor: &Anchor, author: &str, body: &str) -> Result<Thread> {
    validate_message(author, body)?;
    let mut db = open(path, true)?;
    let tx = db.transaction()?;
    let thread_id = id(&tx)?;
    let time = now();
    tx.execute(
        "INSERT INTO threads(id,path,anchor,created_at,updated_at) VALUES (?1,?2,?3,?4,?4)",
        params![
            thread_id,
            anchor.location.path,
            serde_json::to_string(anchor)?,
            time
        ],
    )?;
    tx.execute(
        "INSERT INTO messages(id,thread_id,author,body,created_at) VALUES (?1,?2,?3,?4,?5)",
        params![id(&tx)?, thread_id, author.trim(), body.trim(), time],
    )?;
    tx.commit()?;
    get(path, &thread_id)
}

pub fn reply(path: &Path, thread_id: &str, author: &str, body: &str) -> Result<Thread> {
    validate_message(author, body)?;
    let mut db = open(path, true)?;
    let tx = db.transaction()?;
    let time = now();
    if tx.execute(
        "UPDATE threads SET updated_at=?2 WHERE id=?1",
        params![thread_id, time],
    )? == 0
    {
        bail!("Unknown thread: {thread_id}");
    }
    tx.execute(
        "INSERT INTO messages(id,thread_id,author,body,created_at) VALUES (?1,?2,?3,?4,?5)",
        params![id(&tx)?, thread_id, author.trim(), body.trim(), time],
    )?;
    tx.commit()?;
    get(path, thread_id)
}

pub fn set_resolved(path: &Path, thread_id: &str, resolved: bool) -> Result<Thread> {
    let db = open(path, true)?;
    if db.execute(
        "UPDATE threads SET resolved=?2, updated_at=?3 WHERE id=?1",
        params![thread_id, resolved, now()],
    )? == 0
    {
        bail!("Unknown thread: {thread_id}");
    }
    get(path, thread_id)
}

pub fn edit(path: &Path, message_id: &str, body: &str) -> Result<Thread> {
    validate_message("edit", body)?;
    let mut db = open(path, true)?;
    let tx = db.transaction()?;
    let time = now();
    let thread: String = tx
        .query_row(
            "SELECT thread_id FROM messages WHERE id=?1 AND deleted_at IS NULL",
            [message_id],
            |row| row.get(0),
        )
        .context("Message doesn't exist or has been deleted")?;
    tx.execute(
        "UPDATE messages SET body=?2, edited_at=?3 WHERE id=?1",
        params![message_id, body.trim(), time],
    )?;
    tx.execute(
        "UPDATE threads SET updated_at=?2 WHERE id=?1",
        params![thread, time],
    )?;
    tx.commit()?;
    get(path, &thread)
}

pub fn set_deleted(path: &Path, message_id: &str, deleted: bool) -> Result<Thread> {
    let mut db = open(path, true)?;
    let tx = db.transaction()?;
    let time = now();
    let thread: String = tx
        .query_row(
            "SELECT thread_id FROM messages WHERE id=?1",
            [message_id],
            |row| row.get(0),
        )
        .context("Message doesn't exist")?;
    tx.execute(
        "UPDATE messages SET deleted_at=?2 WHERE id=?1",
        params![message_id, deleted.then_some(time)],
    )?;
    tx.execute(
        "UPDATE threads SET updated_at=?2 WHERE id=?1",
        params![thread, time],
    )?;
    tx.commit()?;
    get(path, &thread)
}

pub fn list(path: &Path) -> Result<Vec<Thread>> {
    if !path.try_exists()? {
        return Ok(vec![]);
    }
    let mut db = open(path, false)?;
    let tx = db.transaction()?;
    let mut threads = Vec::new();
    let mut indices = HashMap::new();
    {
        let mut stmt = tx.prepare(
            "SELECT id,anchor,resolved,created_at,updated_at FROM threads ORDER BY created_at,id",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            indices.insert(id.clone(), threads.len());
            threads.push(Thread {
                id,
                anchor: serde_json::from_str(&row.get::<_, String>(1)?)?,
                resolved: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                messages: vec![],
            });
        }
        let version: u32 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let mut stmt = tx.prepare(if version >= 2 {
            "SELECT id,thread_id,author,CASE WHEN deleted_at IS NULL THEN body ELSE '' END,created_at,edited_at,deleted_at FROM messages ORDER BY created_at,rowid"
        } else {
            "SELECT id,thread_id,author,body,created_at,NULL,NULL FROM messages ORDER BY created_at,rowid"
        })?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            if let Some(&index) = indices.get(&row.get::<_, String>(1)?) {
                threads[index].messages.push(Message {
                    id: row.get(0)?,
                    author: row.get(2)?,
                    body: row.get(3)?,
                    created_at: row.get(4)?,
                    edited_at: row.get(5)?,
                    deleted_at: row.get(6)?,
                });
            }
        }
    }
    tx.commit()?;
    Ok(threads)
}

pub fn get(path: &Path, thread_id: &str) -> Result<Thread> {
    list(path)?
        .into_iter()
        .find(|thread| thread.id == thread_id)
        .with_context(|| format!("Unknown thread: {thread_id}"))
}
