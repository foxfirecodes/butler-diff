use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Repository {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub target: String,
    pub target_ref: String,
    pub source: String,
    pub stacks: Vec<Stack>,
    #[serde(skip)]
    pub metadata_signature: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Stack {
    pub id: String,
    pub applied: bool,
    pub branches: Vec<Branch>, // Base to tip, matching GitButler's metadata order.
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Branch {
    pub name: String,
    #[serde(flatten)]
    pub comparison: BranchComparison,
    pub tip: String,
    pub review: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum BranchComparison {
    Ready { base: String },
    Unavailable { error: String },
}

impl Branch {
    pub fn base(&self) -> Result<&str> {
        match &self.comparison {
            BranchComparison::Ready { base } => Ok(base),
            BranchComparison::Unavailable { error } => bail!("{error}"),
        }
    }
}

#[derive(Default, Deserialize, Hash)]
struct Metadata {
    #[serde(default)]
    branches: BTreeMap<String, RawStack>,
    default_target: Option<Target>,
    #[serde(default)]
    branch_targets: BTreeMap<String, Target>,
}

#[derive(Deserialize, Hash)]
struct Target {
    sha: String,
}

#[derive(Deserialize, Hash)]
struct RawStack {
    #[serde(default)]
    order: usize,
    #[serde(default = "yes")]
    in_workspace: bool,
    #[serde(default)]
    heads: Vec<RawHead>,
}

#[derive(Deserialize, Hash)]
struct RawHead {
    name: String,
    #[serde(default)]
    archived: bool,
    head: BTreeMap<String, String>,
    pr_number: Option<u64>,
    review_id: Option<String>,
}

fn yes() -> bool {
    true
}

fn signature(metadata: &Metadata) -> u64 {
    let mut hasher = DefaultHasher::new();
    metadata.hash(&mut hasher);
    hasher.finish()
}

pub fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("--no-pager")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .context("Couldn't run git. Install Git and ensure it is on PATH.")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn config(root: &Path, key: &str) -> Option<String> {
    git(root, &["config", "--get", key])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

pub fn short_ref(reference: &str) -> &str {
    reference
        .strip_prefix("refs/heads/")
        .or_else(|| reference.strip_prefix("refs/remotes/"))
        .unwrap_or(reference)
}

fn resolve(root: &Path, revision: &str) -> Result<String> {
    Ok(git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{revision}^{{commit}}"),
        ],
    )?
    .trim()
    .to_owned())
}

// New GitButler versions migrate TOML into SQLite. Never read the stale TOML after migration.
fn read_sqlite(path: &Path) -> Result<Metadata> {
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .context("Couldn't open GitButler's metadata database read-only")?;
    db.busy_timeout(std::time::Duration::from_secs(2))?;
    let tx = db
        .transaction()
        .context("Couldn't start a GitButler metadata read transaction")?;
    let mut meta = Metadata::default();
    {
        let mut stmt = tx
            .prepare("SELECT id, sort_order, in_workspace FROM vb_stacks ORDER BY sort_order, id")
            .context("Couldn't read GitButler stacks from SQLite")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, usize>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })?;
        for row in rows {
            let (id, order, in_workspace) = row?;
            meta.branches.insert(
                id,
                RawStack {
                    order,
                    in_workspace,
                    heads: vec![],
                },
            );
        }
        let mut stmt = tx.prepare("SELECT stack_id, name, head_sha, pr_number, archived FROM vb_stack_heads ORDER BY stack_id, position")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                RawHead {
                    name: r.get(1)?,
                    head: BTreeMap::from([("CommitId".into(), r.get(2)?)]),
                    pr_number: r.get(3)?,
                    archived: r.get(4)?,
                    review_id: None,
                },
            ))
        })?;
        for row in rows {
            let (id, head) = row?;
            if let Some(stack) = meta.branches.get_mut(&id) {
                stack.heads.push(head);
            }
        }
        let mut stmt = tx.prepare("SELECT stack_id, sha FROM vb_branch_targets")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, Target { sha: r.get(1)? }))
        })?;
        for row in rows {
            let (id, target) = row?;
            meta.branch_targets.insert(id, target);
        }
    }
    tx.commit()?;
    Ok(meta)
}

impl Repository {
    pub fn is_local(&self) -> bool {
        self.source == "Git local branches"
    }

    pub fn base_refs(path: &Path) -> Result<Vec<String>> {
        Ok(git(
            path,
            &[
                "for-each-ref",
                "--format=%(refname)%00%(symref)",
                "refs/heads",
                "refs/remotes",
            ],
        )?
        .lines()
        .filter_map(|line| {
            let (name, symbolic_target) = line.split_once('\0')?;
            symbolic_target.is_empty().then(|| name.to_owned())
        })
        .collect())
    }

    // GitButler writes caches/analytics into the same database. A DB file event
    // alone is not evidence that any stack changed. Probe only stack tables.
    pub fn current_metadata_signature(&self) -> Result<u64> {
        let dir = self.common_dir.join("gitbutler");
        let meta = if self.source == "GitButler SQLite" {
            read_sqlite(&dir.join("but.sqlite"))?
        } else {
            toml::from_str(&std::fs::read_to_string(dir.join("virtual_branches.toml"))?)?
        };
        Ok(signature(&meta))
    }

    pub fn load(path: &Path, target_override: Option<&str>) -> Result<Self> {
        let root = PathBuf::from(git(path, &["rev-parse", "--show-toplevel"])?.trim());
        let git_dir = PathBuf::from(git(&root, &["rev-parse", "--absolute-git-dir"])?.trim());
        let common_dir = PathBuf::from(
            git(
                &root,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            )?
            .trim(),
        );
        let dir = common_dir.join("gitbutler");
        let migrated = config(&root, "gitbutler.project.portedmeta").as_deref() == Some("true");
        let (meta, source) = if migrated {
            (read_sqlite(&dir.join("but.sqlite"))?, "GitButler SQLite")
        } else if dir.join("virtual_branches.toml").exists() {
            let text = std::fs::read_to_string(dir.join("virtual_branches.toml"))
                .context("Couldn't read GitButler stack metadata")?;
            (
                toml::from_str::<Metadata>(&text)
                    .context("Couldn't parse GitButler stack metadata")?,
                "GitButler TOML",
            )
        } else {
            return Self::load_local(root, git_dir, common_dir, target_override);
        };
        let metadata_signature = signature(&meta);
        let target = target_override
            .map(str::to_owned)
            .or_else(|| config(&root, "gitbutler.project.targetcommitid"))
            .or_else(|| meta.default_target.as_ref().map(|t| t.sha.clone()))
            .or_else(|| config(&root, "gitbutler.project.targetref"))
            .context(
                "GitButler has no target branch. Pass --base <ref> (e.g. --base origin/main).",
            )?;
        let target_ref = target;
        let target = resolve(&root, &target_ref)
            .context("Couldn't resolve the stack base. Pass --base <ref>.")?;
        let mut raw: Vec<_> = meta.branches.into_iter().collect();
        raw.sort_by(|(a_id, a), (b_id, b)| {
            (!a.in_workspace, a.order, a_id).cmp(&(!b.in_workspace, b.order, b_id))
        });
        let mut stacks = vec![];
        for (id, stack) in raw {
            let heads: Vec<_> = stack.heads.into_iter().filter(|h| !h.archived).collect();
            if heads.is_empty() {
                continue;
            }
            let target = if target_override.is_none() {
                meta.branch_targets
                    .get(&id)
                    .map(|t| t.sha.as_str())
                    .unwrap_or(&target)
            } else {
                &target
            };
            let mut branches = vec![];
            let mut previous: Option<String> = None;
            for head in heads {
                let tip = resolve(&root, &format!("refs/heads/{}", head.name))
                    .or_else(|_| {
                        let stored = head.head.get("CommitId").context("Missing branch head")?;
                        resolve(&root, stored)
                    })
                    .with_context(|| format!("Couldn't resolve branch {}", head.name))?;
                let base = match previous {
                    Some(base) => base,
                    None => git(&root, &["merge-base", target, &tip])?.trim().to_owned(),
                };
                previous = Some(tip.clone());
                branches.push(Branch {
                    name: head.name,
                    comparison: BranchComparison::Ready { base },
                    tip,
                    review: head
                        .review_id
                        .or_else(|| head.pr_number.map(|n| format!("#{n}"))),
                });
            }
            stacks.push(Stack {
                id,
                applied: stack.in_workspace,
                branches,
            });
        }
        Ok(Self {
            root,
            git_dir,
            common_dir,
            target,
            target_ref,
            source: source.into(),
            stacks,
            metadata_signature,
        })
    }

    fn load_local(
        root: PathBuf,
        git_dir: PathBuf,
        common_dir: PathBuf,
        target_override: Option<&str>,
    ) -> Result<Self> {
        let current = git(&root, &["symbolic-ref", "--quiet", "HEAD"])
            .ok()
            .map(|name| name.trim().to_owned());
        let refs = git(
            &root,
            &[
                "for-each-ref",
                "--format=%(refname)%00%(objectname)",
                "refs/heads",
            ],
        )?;
        let mut branches: Vec<_> = refs
            .lines()
            .filter_map(|line| line.split_once('\0'))
            .collect();
        branches.sort_by_key(|(name, _)| (Some(*name) != current.as_deref(), *name));
        let remote_head = git(
            &root,
            &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
        )
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| resolve(&root, name).is_ok());
        let target_ref = target_override
            .map(str::to_owned)
            .or(remote_head)
            .or_else(|| {
                ["refs/heads/main", "refs/heads/master"]
                    .into_iter()
                    .find(|candidate| branches.iter().any(|(name, _)| name == candidate))
                    .map(str::to_owned)
            })
            .or_else(|| branches.first().map(|(name, _)| (*name).to_owned()))
            .unwrap_or_default();
        let target = if target_ref.is_empty() {
            String::new() // A new repository has no committed branches to compare yet.
        } else {
            resolve(&root, &target_ref).with_context(|| {
                format!(
                    "Couldn't resolve base {target_ref}. Choose a base branch or pass --base <ref>."
                )
            })?
        };
        let mut stacks = Vec::with_capacity(branches.len());
        for (reference, tip) in branches {
            let name = reference.strip_prefix("refs/heads/").unwrap().to_owned();
            let comparison = match git(&root, &["merge-base", &target, tip]) {
                Ok(base) => BranchComparison::Ready {
                    base: base.trim().to_owned(),
                },
                Err(error) => BranchComparison::Unavailable {
                    error: format!(
                        "Couldn't compare {name} with {target_ref}. Choose a base branch with shared history. {error:#}"
                    ),
                },
            };
            // Local branches are independent comparisons, never a synthetic ordered stack.
            stacks.push(Stack {
                id: reference.to_owned(),
                applied: false,
                branches: vec![Branch {
                    name,
                    comparison,
                    tip: tip.to_owned(),
                    review: None,
                }],
            });
        }
        Ok(Self {
            root,
            git_dir,
            common_dir,
            target,
            target_ref,
            source: "Git local branches".into(),
            stacks,
            metadata_signature: 0,
        })
    }

    // This endpoint pair also supports a whole stack or a contiguous branch range.
    pub fn patch(&self, base: &str, tip: &str) -> Result<String> {
        git(
            &self.root,
            &[
                "-c",
                "core.quotepath=false",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--full-index",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                "--find-renames",
                "--unified=8",
                base,
                tip,
                "--",
            ],
        )
    }
}
