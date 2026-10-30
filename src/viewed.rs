use crate::repository::{Repository, git};
use anyhow::{Context, Result, bail};
use gpui_kit::component::diff::DiffFile;
use rusqlite::{Connection, OpenFlags, params, params_from_iter};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

pub const STATE_FILE: &str = "butler-diff-viewed.sqlite";

// Identity belongs to the actual file comparison, not the containing commits.
// Both blobs matter: changing the base of a range can change the reviewed diff
// even when its tip file hasn't changed. Headers also cover modes and renames.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileVersion {
    original_path: Option<String>,
    modified_path: Option<String>,
    original_blob: Option<String>,
    modified_blob: Option<String>,
    headers: Vec<String>,
}

fn full_oid(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn index_blobs(file: &DiffFile) -> Option<(Option<String>, Option<String>)> {
    let index = file
        .extended_headers()
        .iter()
        .find_map(|line| line.strip_prefix("index "))?;
    let (before, after) = index.split_whitespace().next()?.split_once("..")?;
    if !full_oid(before) || !full_oid(after) {
        return None;
    }
    let blob = |oid: &str| (!oid.bytes().all(|byte| byte == b'0')).then(|| oid.to_owned());
    Some((blob(before), blob(after)))
}

fn blob_at(repo: &Repository, commit: &str, path: Option<&str>) -> Result<Option<String>> {
    let Some(path) = path else { return Ok(None) };
    // Query one literal path in an immutable tree. No file contents are read or
    // hashed, and wildcards/leading punctuation in filenames are not pathspecs.
    let entry = git(
        &repo.root,
        &[
            "ls-tree",
            "--full-tree",
            "-z",
            commit,
            "--",
            &format!(":(literal){path}"),
        ],
    )?;
    let oid = entry
        .split_once('\t')
        .and_then(|(header, _)| header.split_whitespace().nth(2))
        .filter(|oid| full_oid(oid))
        .with_context(|| format!("Couldn't resolve file version for {path}"))?;
    Ok(Some(oid.to_owned()))
}

impl FileVersion {
    fn with_blobs(
        file: &DiffFile,
        (original_blob, modified_blob): (Option<String>, Option<String>),
    ) -> Self {
        Self {
            original_path: file.original_path().map(|path| path.to_string()),
            modified_path: file.modified_path().map(|path| path.to_string()),
            original_blob,
            modified_blob,
            headers: file
                .extended_headers()
                .iter()
                .map(ToString::to_string)
                .collect(),
        }
    }

    pub fn from_patch(file: &DiffFile) -> Option<Self> {
        index_blobs(file).map(|blobs| Self::with_blobs(file, blobs))
    }

    pub fn resolve(repo: &Repository, base: &str, tip: &str, file: &DiffFile) -> Result<Self> {
        let original_path = file.original_path().map(|path| path.as_str());
        let modified_path = file.modified_path().map(|path| path.as_str());
        let (original_blob, modified_blob) = match index_blobs(file) {
            Some(blobs) => blobs,
            // Pure renames and mode-only diffs may omit `index`. Resolve only
            // this file, only when marking it or validating an existing mark.
            None => (
                blob_at(repo, base, original_path)?,
                blob_at(repo, tip, modified_path)?,
            ),
        };
        Ok(Self::with_blobs(file, (original_blob, modified_blob)))
    }

    pub fn key(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

fn open(path: &Path, write: bool) -> Result<Connection> {
    let flags = if write {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let db = Connection::open_with_flags(path, flags).context("Couldn't open viewed-file state")?;
    db.busy_timeout(std::time::Duration::from_secs(2))?;
    let version: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > 1 {
        bail!("Viewed-file state uses a newer format ({version}).");
    }
    if write {
        db.execute_batch("CREATE TABLE IF NOT EXISTS viewed_files(path TEXT NOT NULL, version TEXT NOT NULL, PRIMARY KEY(path, version)); PRAGMA user_version = 1;")?;
    }
    Ok(db)
}

#[cfg(test)]
pub fn set(path: &Path, file: &str, version: &FileVersion, viewed: bool) -> Result<()> {
    set_many(path, &[(file.to_owned(), version.clone())], viewed)
}

pub fn set_many(path: &Path, files: &[(String, FileVersion)], viewed: bool) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let mut db = open(path, true)?;
    let tx = db.transaction()?;
    {
        let sql = if viewed {
            "INSERT OR IGNORE INTO viewed_files(path, version) VALUES (?1, ?2)"
        } else {
            "DELETE FROM viewed_files WHERE path = ?1 AND version = ?2"
        };
        let mut stmt = tx.prepare(sql)?;
        for (file, version) in files {
            stmt.execute(params![file, version.key()?])?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn saved_versions(path: &Path, files: &[DiffFile]) -> Result<HashMap<String, HashSet<String>>> {
    if !path.try_exists()? || files.is_empty() {
        return Ok(HashMap::new());
    }
    let mut db = open(path, false)?;
    let tx = db.transaction()?;
    let mut versions: HashMap<String, HashSet<String>> = HashMap::new();
    // Only load records for this comparison; bound SQLite parameter counts even
    // for huge diffs. No hashing or Git calls occur in this database lookup.
    for chunk in files.chunks(400) {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(",");
        let mut stmt = tx.prepare(&format!(
            "SELECT path, version FROM viewed_files WHERE path IN ({placeholders})"
        ))?;
        let rows = stmt.query_map(
            params_from_iter(chunk.iter().map(|file| file.path().as_str())),
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        for row in rows {
            let (path, version) = row?;
            versions.entry(path).or_default().insert(version);
        }
    }
    tx.commit()?;
    Ok(versions)
}

pub fn load(
    repo: &Repository,
    base: &str,
    tip: &str,
    files: &[DiffFile],
) -> Result<HashSet<String>> {
    let saved = saved_versions(&repo.common_dir.join(STATE_FILE), files)?;
    let mut viewed = HashSet::new();
    for file in files {
        // New/unreviewed files require no version resolution at all.
        let Some(versions) = saved.get(file.path().as_str()) else {
            continue;
        };
        let version = FileVersion::resolve(repo, base, tip, file)?;
        if versions.contains(&version.key()?) {
            viewed.insert(file.path().to_string());
        }
    }
    Ok(viewed)
}
