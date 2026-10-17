// Derived from openrig packages/cli/src/lib/context-install.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Local directory install. Git and URL sources are refused before any write.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crewmux_api::rig_home;
use crewmux_spec::context::{assert_safe_pack_ref, parse_manifest};

use crate::error::Error;

pub(crate) fn add(source: &str, name: Option<&str>, git: bool, json: bool) -> Result<i32, Error> {
    if git || is_remote(source) {
        return Err(Error::User(
            "git and URL context sources are not available; add a local directory".to_owned(),
        ));
    }
    let source = PathBuf::from(source);
    let manifest_path = source.join("manifest.yaml");
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|err| Error::User(format!("failed to read {}: {err}", manifest_path.display())))?;
    let manifest = parse_manifest(&raw, &manifest_path.display().to_string())
        .map_err(|err| Error::User(err.to_string()))?;
    let install = name.unwrap_or(manifest.name.as_str());
    assert_safe_pack_ref(install).map_err(Error::User)?;
    let root = rig_home().join("context");
    let target = join_ref(&root, install);
    reject_namespace(install, &root, &target)?;
    if std::fs::symlink_metadata(&target).is_ok() {
        return Err(Error::User(format!(
            "A context pack named '{install}' already exists at {}. Remove it first or use --name to install under a different name.",
            target.display()
        )));
    }
    let staging = root.join(staging_name());
    let installed = (|| {
        if let Some(parent) = staging.parent() {
            std::fs::create_dir_all(parent)?;
        }
        copy_tree(&source, &staging)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&staging, &target)?;
        Ok::<(), std::io::Error>(())
    })();
    if let Err(err) = installed {
        drop(std::fs::remove_dir_all(&staging));
        return Err(Error::User(format!("install failed: {err}")));
    }
    let count = crate::context_view::sync_count()?;
    if json {
        crate::output::stdout_json(&serde_json::json!({
            "installedAt": target.display().to_string(),
            "count": count,
        }));
    } else {
        crate::output::stdout_line(&format!(
            "Installed at {}. Library now has {count} context pack(s).",
            target.display()
        ));
    }
    Ok(0)
}

fn is_remote(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    lower.contains("://") || lower.strip_suffix(".git").is_some() || lower.starts_with("git@")
}

fn join_ref(root: &Path, refer: &str) -> PathBuf {
    refer
        .split('/')
        .fold(root.to_path_buf(), |path, segment| path.join(segment))
}

fn reject_namespace(refer: &str, root: &Path, target: &Path) -> Result<(), Error> {
    let Some(parent) = target.parent() else {
        return Ok(());
    };
    if parent == root {
        return Ok(());
    }
    let mut cursor = root.to_path_buf();
    let Ok(rel) = parent.strip_prefix(root) else {
        return Ok(());
    };
    for segment in rel.components() {
        cursor.push(segment);
        let Ok(meta) = std::fs::symlink_metadata(&cursor) else {
            break;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(Error::User(format!(
                "context pack ref '{refer}' crosses a symlink or non-directory namespace segment '{}'",
                cursor.display()
            )));
        }
    }
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(std::io::Error::other(format!(
                "refusing to install symlink {}",
                entry.path().display()
            )));
        }
        let child = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&entry.path(), &child)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), child)?;
        }
    }
    Ok(())
}

fn staging_name() -> String {
    let millis = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis(),
        Err(_) => 0,
    };
    format!(".tmp-add-{millis}-{}", std::process::id())
}
