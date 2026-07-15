use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::ProjectRef;

/// Resolve a path to a stable project identity.
///
/// Walks up from `path` looking for a `.git` entry; the directory containing it
/// is the project root. When not inside a repo, the canonicalized path itself is
/// the root. The id is the first 16 hex chars of sha256(canonical_root_path).
pub fn resolve(path: &Path) -> Result<ProjectRef> {
    let abs = canonicalize(path)?;
    let probe = if abs.is_file() {
        abs.parent().unwrap_or(&abs).to_path_buf()
    } else {
        abs.clone()
    };

    let root = find_project_root(&probe).unwrap_or(probe);
    let id = short_hash(&root.display().to_string());
    let name = root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string();

    Ok(ProjectRef {
        id,
        name,
        root_path: root.display().to_string(),
    })
}

fn canonicalize(path: &Path) -> Result<PathBuf> {
    // Prefer fs::canonicalize when the path exists; otherwise build an absolute
    // path by joining the current dir so ids are still stable.
    if path.exists() {
        return fs::canonicalize(path).with_context(|| format!("canonicalize {}", path.display()));
    }
    let cwd = std::env::current_dir().context("cwd")?;
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    Ok(abs)
}

fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut cur = Some(start);
    let mut marker_root = None;
    while let Some(p) = cur {
        if p.join(".git").exists() {
            if is_temp_root(p) && p != start {
                return marker_root;
            }
            return marker_root.or_else(|| Some(p.to_path_buf()));
        }
        if marker_root.is_none() && has_project_marker(p) {
            marker_root = Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    marker_root
}

fn is_temp_root(path: &Path) -> bool {
    fs::canonicalize(path)
        .ok()
        .zip(fs::canonicalize(std::env::temp_dir()).ok())
        .is_some_and(|(path, temp)| path == temp)
}

fn has_project_marker(path: &Path) -> bool {
    [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
        "deno.json",
        "deno.jsonc",
    ]
    .iter()
    .any(|name| path.join(name).is_file())
        || path.join(".voxd-project").exists()
}

pub fn short_hash(input: &str) -> String {
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    let out = h.finalize();
    hex::encode(out)[..16].to_string()
}
