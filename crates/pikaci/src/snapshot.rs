use std::fs;
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SnapshotMetadata {
    pub source_root: String,
    pub snapshot_dir: String,
    pub git_head: Option<String>,
    pub git_dirty: Option<bool>,
    pub created_at: String,
}

pub fn create_snapshot(
    source_root: &Path,
    snapshot_dir: &Path,
    created_at: &str,
) -> anyhow::Result<SnapshotMetadata> {
    copy_tree(source_root, snapshot_dir, true)?;
    let metadata = SnapshotMetadata {
        source_root: source_root.display().to_string(),
        snapshot_dir: snapshot_dir.display().to_string(),
        git_head: git_head(source_root),
        git_dirty: git_dirty(source_root),
        created_at: created_at.to_string(),
    };
    write_json(snapshot_dir.join("pikaci-snapshot.json"), &metadata)?;
    Ok(metadata)
}

fn copy_tree(source: &Path, destination: &Path, root: bool) -> anyhow::Result<()> {
    fs::create_dir_all(destination)
        .with_context(|| format!("create snapshot dir {}", destination.display()))?;

    for entry in fs::read_dir(source).with_context(|| format!("read {}", source.display()))? {
        let entry = entry?;
        let source_path = entry.path();
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if should_skip(&file_name, root) {
            continue;
        }

        let destination_path = destination.join(file_name.as_ref());
        let metadata = fs::symlink_metadata(&source_path)
            .with_context(|| format!("stat {}", source_path.display()))?;
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&source_path)
                .with_context(|| format!("read symlink {}", source_path.display()))?;
            unix_fs::symlink(&target, &destination_path).with_context(|| {
                format!(
                    "recreate symlink {} -> {}",
                    destination_path.display(),
                    target.display()
                )
            })?;
        } else if metadata.is_dir() {
            copy_tree(&source_path, &destination_path, false)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "copy file {} -> {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        } else {
            return Err(anyhow!(
                "unsupported filesystem entry: {}",
                source_path.display()
            ));
        }
    }

    Ok(())
}

fn should_skip(name: &str, root: bool) -> bool {
    matches!(name, ".git" | ".pikaci" | ".direnv")
        || name == "target"
        || (!root && name == "node_modules")
}

fn git_head(source_root: &Path) -> Option<String> {
    run_git(source_root, &["rev-parse", "HEAD"])
}

fn git_dirty(source_root: &Path) -> Option<bool> {
    let output = run_git(
        source_root,
        &["status", "--short", "--untracked-files=normal"],
    )?;
    Some(!output.trim().is_empty())
}

fn run_git(source_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(source_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn write_json(path: PathBuf, value: &impl Serialize) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).context("encode snapshot metadata")?;
    fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::should_skip;

    #[test]
    fn snapshot_skip_filters_ignore_expected_paths() {
        assert!(should_skip(".git", true));
        assert!(should_skip(".pikaci", true));
        assert!(should_skip(".direnv", true));
        assert!(should_skip("target", true));
        assert!(should_skip("node_modules", false));
        assert!(!should_skip("Cargo.toml", true));
        assert!(!should_skip("src", false));
    }
}
