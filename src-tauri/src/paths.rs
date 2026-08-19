//! Containment helpers for user-supplied filesystem paths.

use std::path::{Component, Path, PathBuf};

/// Resolve `user_path` under `root`, rejecting traversal and absolute paths.
pub fn resolve_under(root: &Path, user_path: &str) -> anyhow::Result<PathBuf> {
    let trimmed = user_path.trim();
    if trimmed.is_empty() {
        anyhow::bail!("path must not be empty");
    }
    if trimmed.contains('\0') {
        anyhow::bail!("path contains NUL");
    }
    let relative = Path::new(trimmed);
    if relative.is_absolute() {
        anyhow::bail!("absolute paths are not allowed");
    }
    for component in relative.components() {
        if matches!(component, Component::ParentDir) {
            anyhow::bail!("path must not contain ..");
        }
    }
    std::fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let candidate = root.join(relative);
    let resolved = if candidate.exists() {
        candidate.canonicalize()?
    } else {
        let parent = candidate
            .parent()
            .map(|p| {
                std::fs::create_dir_all(p)?;
                p.canonicalize()
            })
            .transpose()?
            .unwrap_or_else(|| root.clone());
        let name = candidate.file_name().map(PathBuf::from).unwrap_or_default();
        parent.join(name)
    };
    if !resolved.starts_with(&root) {
        anyhow::bail!("path escapes recordings root");
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_absolute() {
        let dir = std::env::temp_dir().join(format!("pulsescope-paths-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(resolve_under(&dir, "../etc/passwd").is_err());
        assert!(resolve_under(&dir, "/etc/passwd").is_err());
        assert!(resolve_under(&dir, "ok/file.cf32").is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
