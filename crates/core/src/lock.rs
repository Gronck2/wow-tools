//! One writer per WoW folder: operations that change WTF or AddOns take a file lock named after the
//! flavor folder, so the tray, `watch`, the CLI, a scheduled task and any other shell never overlap.
//!
//! `%LOCALAPPDATA%\WoW Tools\locks\<sha256(lowercased flavor folder)[..16]>.lock` is locked with
//! `File::try_lock` (fail-fast; Windows drops it when the process dies, so a crash leaves nothing
//! stale). The path ignores portable mode on purpose: a portable and an installed copy exclude each
//! other. Locked bytes cannot be read by another handle on Windows, so the holder's `pid operation`
//! goes to `<name>.info` beside it, only to name the holder in the message. Take it at the outermost
//! entry point only: a second handle in the same process fails.

use crate::wow::Flavor;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

/// Holds the lock until dropped.
#[derive(Debug)]
pub struct Guard {
    _file: File,
}

/// Locks `fl`'s folder for the operation `what` (a short technical name such as "backup"); fails at once if another process holds it.
pub fn acquire(fl: &Flavor, what: &str) -> Result<Guard> {
    acquire_in(&locks_dir(), fl, what)
}

#[cfg(not(test))]
fn locks_dir() -> PathBuf {
    let root = directories::ProjectDirs::from("", "", crate::config::APP_NAME).and_then(|d| d.data_local_dir().parent().map(Path::to_path_buf));
    root.unwrap_or_else(|| PathBuf::from(".")).join("locks")
}

/// Tests never touch the real folder.
#[cfg(test)]
fn locks_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DIR.get_or_init(|| tempfile::tempdir().unwrap()).path().to_path_buf()
}

fn acquire_in(dir: &Path, fl: &Flavor, what: &str) -> Result<Guard> {
    fs::create_dir_all(dir).with_context(|| tr!("error.write", path = dir.display()))?;
    let key = Sha256::digest(fl.dir.to_string_lossy().to_lowercase().as_bytes());
    let name: String = key.iter().take(8).map(|b| format!("{b:02x}")).collect();
    let path = dir.join(format!("{name}.lock"));
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .with_context(|| tr!("error.write", path = path.display()))?;
    match file.try_lock() {
        Ok(()) => {
            // Best effort: only the message of a later, refused process reads it.
            let _ = fs::write(path.with_extension("info"), format!("{} {what}", std::process::id()));
            Ok(Guard { _file: file })
        }
        Err(TryLockError::WouldBlock) => {
            let holder = fs::read_to_string(path.with_extension("info")).unwrap_or_default();
            let holder = if holder.is_empty() { "?" } else { holder.trim() };
            bail!(tr!("app.busy", flavor = fl.title(), what = holder))
        }
        Err(TryLockError::Error(e)) => Err(e).with_context(|| tr!("error.write", path = path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flavor(dir: &Path) -> Flavor {
        Flavor {
            name: "_retail_".into(),
            dir: dir.to_path_buf(),
            version: None,
        }
    }

    #[test]
    fn second_acquire_fails_other_folder_and_drop_free() {
        let locks = tempfile::tempdir().unwrap();
        let (a, b) = (flavor(Path::new("C:/Games/WoW/_retail_")), flavor(Path::new("D:/WoW/_retail_")));
        let first = acquire_in(locks.path(), &a, "backup").unwrap();
        // Same folder in another case is the same key.
        let err = acquire_in(locks.path(), &flavor(Path::new("c:/games/wow/_RETAIL_")), "restore").unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("backup") && text.contains(&std::process::id().to_string()), "{text}");
        assert!(text.starts_with("другой WoW Tools"), "{text}");
        let _other = acquire_in(locks.path(), &b, "backup").unwrap();
        drop(first);
        acquire_in(locks.path(), &a, "restore").unwrap();
    }

    #[test]
    fn locks_dir_that_cannot_be_made_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("file");
        fs::write(&file, "x").unwrap();
        assert!(acquire_in(&file.join("locks"), &flavor(Path::new("C:/WoW/_retail_")), "backup").is_err());
    }

    /// The holder side of `other_process_holds_the_lock`: does nothing unless started by it.
    #[test]
    #[ignore]
    fn child_holds_lock() {
        let Ok(dir) = std::env::var("WT_LOCK_TEST_DIR") else { return };
        let dir = PathBuf::from(dir);
        let _guard = acquire_in(&dir.join("locks"), &flavor(Path::new("C:/WoW/_retail_")), "child-op").unwrap();
        fs::write(dir.join("ready"), "").unwrap();
        for _ in 0..300 {
            if dir.join("stop").exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    #[test]
    fn other_process_holds_the_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "lock::tests::child_holds_lock", "--ignored", "--nocapture"])
            .env("WT_LOCK_TEST_DIR", tmp.path())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        for _ in 0..100 {
            if tmp.path().join("ready").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let fl = flavor(Path::new("c:/wow/_RETAIL_"));
        let busy = acquire_in(&tmp.path().join("locks"), &fl, "backup");
        fs::write(tmp.path().join("stop"), "").unwrap();
        child.wait().unwrap();
        let text = format!("{:#}", busy.unwrap_err());
        assert!(text.contains(&child.id().to_string()) && text.contains("child-op"), "{text}");
        // The child is gone, so the lock is free (Windows drops it with the process).
        acquire_in(&tmp.path().join("locks"), &fl, "backup").unwrap();
    }
}
