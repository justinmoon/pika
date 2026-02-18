use std::path::{Path, PathBuf};

use crate::cli::CliError;

pub fn find_workspace_root(start: &Path) -> Result<PathBuf, CliError> {
    let mut cur = start
        .canonicalize()
        .map_err(|e| CliError::operational(format!("failed to resolve cwd: {e}")))?;
    loop {
        if cur.join("rmp.toml").is_file() {
            return Ok(cur);
        }
        if !cur.pop() {
            break;
        }
    }
    Err(CliError::user(
        "could not find rmp.toml (searches current dir and parents)",
    ))
}

pub fn resolve_workspace_root(
    start: &Path,
    requested_root: Option<&Path>,
) -> Result<PathBuf, CliError> {
    if let Some(root) = requested_root {
        let candidate = if root.is_absolute() {
            root.to_path_buf()
        } else {
            start.join(root)
        };
        let canonical = candidate.canonicalize().map_err(|e| {
            CliError::user(format!(
                "failed to resolve --root path '{}': {e}",
                candidate.to_string_lossy()
            ))
        })?;
        if !canonical.join("rmp.toml").is_file() {
            return Err(CliError::user(format!(
                "--root '{}' does not contain rmp.toml",
                canonical.to_string_lossy()
            )));
        }
        return Ok(canonical);
    }

    find_workspace_root(start)
}

#[allow(dead_code)]
#[derive(serde::Deserialize, Debug, Clone)]
pub struct RmpToml {
    pub project: RmpProject,
    pub core: RmpCore,
    pub ios: Option<RmpIos>,
    pub android: Option<RmpAndroid>,
}

#[allow(dead_code)]
#[derive(serde::Deserialize, Debug, Clone)]
pub struct RmpProject {
    pub name: String,
    pub org: String,
}

#[allow(dead_code)]
#[derive(serde::Deserialize, Debug, Clone)]
pub struct RmpCore {
    #[serde(rename = "crate")]
    pub crate_: String,
    pub bindings: String,
}

#[allow(dead_code)]
#[derive(serde::Deserialize, Debug, Clone)]
pub struct RmpIos {
    pub bundle_id: String,
    pub scheme: Option<String>,
}

#[allow(dead_code)]
#[derive(serde::Deserialize, Debug, Clone)]
pub struct RmpAndroid {
    pub app_id: String,
    pub avd_name: Option<String>,
}

pub fn load_rmp_toml(root: &Path) -> Result<RmpToml, CliError> {
    let p = root.join("rmp.toml");
    let s = std::fs::read_to_string(&p)
        .map_err(|e| CliError::operational(format!("failed to read rmp.toml: {e}")))?;
    let cfg: RmpToml =
        toml::from_str(&s).map_err(|e| CliError::user(format!("failed to parse rmp.toml: {e}")))?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::{find_workspace_root, resolve_workspace_root};

    #[test]
    fn resolve_workspace_root_uses_explicit_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let app = tmp.path().join("apps/rapture");
        std::fs::create_dir_all(&app).expect("mkdir");
        std::fs::write(app.join("rmp.toml"), "[project]\nname='rapture'\norg='com.example'\n[core]\ncrate='rapture_core'\nbindings='uniffi'\n")
            .expect("write rmp.toml");

        let got = resolve_workspace_root(tmp.path(), Some(std::path::Path::new("apps/rapture")))
            .expect("resolve root");
        assert_eq!(got, app.canonicalize().expect("canonicalize"));
    }

    #[test]
    fn find_workspace_root_walks_up() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("rmp.toml"), "{}").expect("write rmp.toml");
        let nested = tmp.path().join("a/b/c");
        std::fs::create_dir_all(&nested).expect("mkdir nested");

        let got = find_workspace_root(&nested).expect("find root");
        assert_eq!(got, tmp.path().canonicalize().expect("canonicalize"));
    }
}
