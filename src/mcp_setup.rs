use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
pub enum Client {
    Codex,
    Claude,
}

impl Client {
    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            _ => bail!("Choose codex or claude for --setup-mcp."),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
        }
    }

    pub fn flag(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

pub fn command(executable: &Path, client: Client) -> Result<String> {
    let executable = executable
        .to_str()
        .context("The executable path isn't valid UTF-8")?;
    Ok(format!(
        "'{}' --setup-mcp {}",
        executable.replace('\'', "'\"'\"'"),
        client.flag()
    ))
}

pub fn install(project: &Path, executable: &Path, client: Client) -> Result<PathBuf> {
    // Validate Git membership, but keep configuration scoped to the directory
    // where setup was run (which may be a project inside a monorepo).
    crate::repository::git(project, &["rev-parse", "--show-toplevel"])?;
    let executable = executable
        .to_str()
        .context("The executable path isn't valid UTF-8")?;
    let path = project.join(match client {
        Client::Codex => ".codex/config.toml",
        Client::Claude => ".mcp.json",
    });
    let previous = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("Couldn't read {}", path.display()));
        }
    };
    let text = match client {
        Client::Codex => {
            let mut config: toml_edit::DocumentMut = previous
                .parse()
                .context("Couldn't parse .codex/config.toml; no changes made")?;
            if !config.contains_key("mcp_servers") {
                config["mcp_servers"] = toml_edit::Item::Table(toml_edit::Table::new());
            }
            let servers = config["mcp_servers"]
                .as_table_like_mut()
                .context("mcp_servers must be a table; no changes made")?;
            let mut server = toml_edit::Table::new();
            server["command"] = toml_edit::value(executable);
            let mut args = toml_edit::Array::new();
            args.push("--mcp");
            args.push(".");
            server["args"] = toml_edit::value(args);
            // Keep the server tied to the current Codex project's directory.
            server["cwd"] = toml_edit::value(".");
            servers.insert("gander", toml_edit::Item::Table(server));
            config.to_string()
        }
        Client::Claude => {
            let mut config: Value = if previous.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(&previous)
                    .context("Couldn't parse .mcp.json; no changes made")?
            };
            let object = config
                .as_object_mut()
                .context(".mcp.json must be an object; no changes made")?;
            let servers = object
                .entry("mcpServers")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .context("mcpServers must be an object; no changes made")?;
            // Claude provides CLAUDE_PROJECT_DIR to stdio servers. Gander reads
            // it at startup, so this config also survives moving the project.
            servers.insert(
                "gander".into(),
                json!({"type":"stdio","command":executable,"args":["--mcp"]}),
            );
            format!("{}\n", serde_json::to_string_pretty(&config)?)
        }
    };
    if text == previous {
        return Ok(path);
    }
    let permissions = fs::metadata(&path)
        .ok()
        .map(|metadata| metadata.permissions());
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".gander-mcp-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        if let Some(permissions) = permissions {
            fs::set_permissions(&temporary, permissions)?;
        }
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("Couldn't configure {}", path.display()))?;
    Ok(path)
}
