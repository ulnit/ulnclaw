//! `ulnclaw gui` — desktop GUI launcher (lean port of hermes `cmd_gui` /
//! `hermes desktop`).
//!
//! Hermes builds and launches its Electron app; ulnclaw ships a Tauri 2
//! shell (`desktop/`), so this command resolves the built
//! `ulnclaw-desktop` binary and spawns it detached. Resolution order:
//! explicit `--binary`, `ULNCLAW_DESKTOP_BINARY`, the compile-time repo
//! root's release/debug artifacts. `--dev` runs `npm run tauri dev`
//! instead.

use std::path::{Path, PathBuf};

/// Compile-time repository root (`CARGO_MANIFEST_DIR` of this crate).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Candidate desktop binary paths in priority order.
pub fn binary_candidates(root: &Path) -> Vec<PathBuf> {
    let desktop = root.join("desktop").join("src-tauri").join("target");
    let exe = std::env::consts::EXE_SUFFIX;
    vec![
        desktop.join("release").join(format!("ulnclaw-desktop{exe}")),
        desktop.join("debug").join(format!("ulnclaw-desktop{exe}")),
    ]
}

/// Resolve the desktop binary: explicit override → env var → candidates.
pub fn resolve_binary(explicit: Option<&Path>, root: &Path) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        if path.exists() {
            return Ok(path.to_path_buf());
        }
        return Err(format!("desktop binary not found: {}", path.display()));
    }
    if let Some(env_path) = std::env::var_os("ULNCLAW_DESKTOP_BINARY") {
        let path = PathBuf::from(env_path);
        if path.exists() {
            return Ok(path);
        }
        return Err(format!(
            "ULNCLAW_DESKTOP_BINARY points to a missing file: {}",
            path.display()
        ));
    }
    for candidate in binary_candidates(root) {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "No built desktop GUI found. Build it first:\n{}\n\
         or point --binary / ULNCLAW_DESKTOP_BINARY at an existing binary.",
        build_instructions()
    ))
}

/// Build instructions shown when no desktop binary exists.
pub fn build_instructions() -> String {
    "  cd desktop\n  npm install\n  npm run tauri build".to_string()
}

/// Spawn the desktop GUI detached; returns the child PID.
pub fn launch(binary: &Path) -> Result<u32, String> {
    let child = std::process::Command::new(binary)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to launch {}: {e}", binary.display()))?;
    Ok(child.id())
}

/// Spawn `npm run tauri dev` in the desktop directory; returns the PID.
pub fn launch_dev(root: &Path) -> Result<u32, String> {
    let desktop = root.join("desktop");
    if !desktop.join("package.json").exists() {
        return Err(format!(
            "desktop/ source not found at {}",
            desktop.display()
        ));
    }
    let child = std::process::Command::new("npm")
        .args(["run", "tauri", "dev"])
        .current_dir(&desktop)
        .spawn()
        .map_err(|e| format!("failed to start npm dev server: {e}"))?;
    Ok(child.id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_point_into_src_tauri_target() {
        let cands = binary_candidates(Path::new("/repo"));
        assert_eq!(cands.len(), 2);
        assert!(cands[0].to_str().unwrap().contains("desktop/src-tauri/target/release"));
        assert!(cands[0].to_str().unwrap().contains("ulnclaw-desktop"));
        assert!(cands[1].to_str().unwrap().contains("target/debug"));
    }

    #[test]
    fn resolve_explicit_missing_errors() {
        let err = resolve_binary(Some(Path::new("/nonexistent/ulnclaw-desktop")), Path::new("/repo")).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn resolve_explicit_existing_wins() {
        let dir = std::env::temp_dir().join(format!("ulnclaw-gui-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("ulnclaw-desktop");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        let resolved = resolve_binary(Some(&bin), Path::new("/repo")).unwrap();
        assert_eq!(resolved, bin);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_falls_back_to_instructions() {
        let err = resolve_binary(None, Path::new("/nonexistent-repo-root")).unwrap_err();
        assert!(err.contains("npm run tauri build"), "{err}");
    }

    #[test]
    fn build_instructions_cover_full_flow() {
        let out = build_instructions();
        assert!(out.contains("npm install"));
        assert!(out.contains("npm run tauri build"));
    }

    #[test]
    fn launch_dev_requires_package_json() {
        let err = launch_dev(Path::new("/nonexistent-repo-root")).unwrap_err();
        assert!(err.contains("source not found"), "{err}");
    }
}
