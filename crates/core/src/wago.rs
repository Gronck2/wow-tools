//! WeakAuras and UI pack updates through the Wago App. It cannot update addons unattended (only
//! its "Update All" button does), but while it runs it writes WeakAuras updates into the
//! `WagoAppCompanion` addon and UI packs into `WagoUI_Storage` (applied in game). It has no hidden
//! start: `START_MINIMIZED` in the environment opens its window minimized. So it is started that
//! way, given time until the companion folders settle, and closed (closing the window quits it).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::updater::{Outcome, close_within, modified, quiet_command, running};

pub const EXE: &str = "Wago App.exe";
const WEAKAURAS: &str = "weakauras";
/// Companion folders and what they hold ("weakauras", "ui_packs"; see `label`).
const COMPANIONS: [(&str, &str); 2] = [("WagoAppCompanion", WEAKAURAS), ("WagoUI_Storage", "ui_packs")];

/// What a companion holds, for people.
fn label(what: &str) -> &'static str {
    if what == WEAKAURAS { "WeakAuras" } else { tr!("wago.ui_packs") }
}
/// Nothing written this long after the start means it has nothing to write.
const IDLE: Duration = Duration::from_secs(45);
/// After a write, this long without another one means done.
const QUIET: Duration = Duration::from_secs(15);
const TIMEOUT: Duration = Duration::from_secs(4 * 60);
const POLL: Duration = Duration::from_secs(3);

pub fn exe() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    Some(local.join(r"Programs\wago-app").join(EXE)).filter(|p| p.is_file())
}

fn config_file() -> PathBuf {
    std::env::var_os("APPDATA")
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join(r"wago-app\config.json")
}

/// The game folder Wago works with (`wowPath` in its settings).
fn wow_path(text: &str) -> Option<PathBuf> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    json["app-preferences-storage"]["state"]["gameInstallation"]["wowPath"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// The companion folders of every flavor: (what it holds, folder).
fn companion_dirs(root: &Path) -> Vec<(&'static str, PathBuf)> {
    let mut out = Vec::new();
    for fl in crate::wow::flavors(root) {
        for (dir, what) in COMPANIONS {
            out.push((what, fl.dir.join(r"Interface\AddOns").join(dir)));
        }
    }
    out
}

/// A hash of the files of a folder (names and contents).
fn content_hash(dir: &Path) -> u64 {
    let mut files: Vec<PathBuf> = walk(dir);
    files.sort();
    let mut h = DefaultHasher::new();
    for f in files {
        f.hash(&mut h);
        fs::read(&f).unwrap_or_default().hash(&mut h);
    }
    h.finish()
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

fn state(dirs: &[(&str, PathBuf)]) -> Vec<u64> {
    dirs.iter().map(|(_, d)| content_hash(d)).collect()
}

/// For `status`.
pub fn describe() -> Result<Vec<String>> {
    let text = fs::read_to_string(config_file()).with_context(|| tr!("wago.not_set_up"))?;
    let root = wow_path(&text).with_context(|| tr!("wago.no_game_folder"))?;
    let found: Vec<String> = companion_dirs(&root)
        .into_iter()
        .filter(|(_, d)| d.is_dir())
        .map(|(what, d)| {
            tr!(
                "wago.found_in",
                what = label(what),
                path = d.parent().and_then(Path::parent).and_then(Path::parent).unwrap_or(&d).display()
            )
        })
        .collect();
    Ok(vec![match found.is_empty() {
        true => tr!("wago.nothing_installed", path = root.display()),
        false => tr!("wago.installed", path = root.display(), list = found.join(", ")),
    }])
}

/// Starts the Wago App minimized, waits until it has written WeakAuras and UI pack updates, closes it.
/// It always writes both; the one switched off (`weakauras`, `ui_packs`) is put back as it was.
pub fn update(exe: &Path, weakauras: bool, ui_packs: bool) -> Result<Outcome> {
    if !weakauras && !ui_packs {
        return Ok(Outcome::note(tr!("wago.nothing")));
    }
    if running(EXE) {
        return Ok(Outcome::note(tr!("wago.open")));
    }
    let text = fs::read_to_string(config_file()).with_context(|| tr!("wago.not_set_up"))?;
    let root = wow_path(&text).with_context(|| tr!("wago.no_game_folder"))?;
    let dirs = companion_dirs(&root);
    let wanted = |what: &str| if what == WEAKAURAS { weakauras } else { ui_packs };

    restore_kept(&kept_dir());
    let off: Vec<&Path> = dirs.iter().filter(|(what, _)| !wanted(what)).map(|(_, d)| d.as_path()).collect();
    keep(&kept_dir(), &root, &off)?;
    let before = state(&dirs);
    let result = run_app(exe, &dirs);
    restore_kept(&kept_dir());
    let (last, timed_out) = result?;

    let mut updated: Vec<String> = Vec::new();
    for ((what, dir), (old, new)) in dirs.iter().zip(before.iter().zip(&last)) {
        let text = tr!("wago.apply_in_game", what = label(what));
        if wanted(what) && old != new && dir.is_dir() && !updated.contains(&text) {
            updated.push(text);
        }
    }
    let note = timed_out.then(|| tr!("addons.timeout", app = "Wago App", min = TIMEOUT.as_secs() / 60));
    Ok(Outcome {
        updated,
        outdated: Vec::new(),
        note,
    })
}

/// Runs the app until it has written the companion folders and then left them alone for a while;
/// returns their last state and whether it timed out.
fn run_app(exe: &Path, dirs: &[(&str, PathBuf)]) -> Result<(Vec<u64>, bool)> {
    log::info!("{}", tr!("wago.starting"));
    let started = Instant::now();
    let t0 = SystemTime::now();
    quiet_command(exe)
        .env("START_MINIMIZED", "1")
        .spawn()
        .with_context(|| tr!("addons.start_failed", path = exe.display()))?;

    let mut timed_out = true;
    while started.elapsed() < TIMEOUT {
        std::thread::sleep(POLL);
        // It writes once its window has loaded and again when updates come in from the site.
        let done = match last_write(dirs).filter(|t| *t > t0) {
            Some(t) => t.elapsed().unwrap_or_default() >= QUIET,
            None => started.elapsed() >= IDLE,
        };
        if done {
            timed_out = false;
            break;
        }
    }
    // It ignores a polite close while minimized; nothing of it is lost when it is killed.
    close_within(EXE, Duration::from_secs(3));
    log::info!("{}", tr!("addons.closed_after", app = "Wago App", secs = started.elapsed().as_secs()));
    Ok((state(dirs), timed_out))
}

/// When anything in the folders (or the folders themselves, which it swaps in whole) was last written.
fn last_write(dirs: &[(&str, PathBuf)]) -> Option<SystemTime> {
    dirs.iter()
        .flat_map(|(_, d)| std::iter::once(d.clone()).chain(walk(d)))
        .filter_map(|p| modified(&p))
        .max()
}

// ---------------------------------------------------------------- folders kept as they were

/// Copies of the companion folders that must not change; kept on disk so a run cut short is
/// undone next time.
fn kept_dir() -> PathBuf {
    crate::config::data_dir().join("wago-kept")
}

#[derive(Serialize, Deserialize)]
struct Kept {
    root: PathBuf,
    /// Relative to `root`, and whether the folder existed.
    dirs: Vec<(PathBuf, bool)>,
}

fn keep(stash: &Path, root: &Path, dirs: &[&Path]) -> Result<()> {
    if dirs.is_empty() {
        return Ok(());
    }
    let mut kept = Kept {
        root: root.to_path_buf(),
        dirs: Vec::new(),
    };
    for dir in dirs {
        let rel = dir.strip_prefix(root)?.to_path_buf();
        let existed = dir.is_dir();
        if existed {
            copy_dir(dir, &stash.join(&rel))?;
        }
        kept.dirs.push((rel, existed));
    }
    fs::create_dir_all(stash)?;
    fs::write(stash.join("kept.json"), serde_json::to_string(&kept)?)?;
    Ok(())
}

/// Puts the kept folders back (the app must be closed).
fn restore_kept(stash: &Path) {
    let Ok(text) = fs::read_to_string(stash.join("kept.json")) else { return };
    let result = (|| -> Result<()> {
        let kept: Kept = serde_json::from_str(&text)?;
        for (rel, existed) in &kept.dirs {
            let target = kept.root.join(rel);
            if target.exists() {
                fs::remove_dir_all(&target)?;
            }
            if *existed {
                copy_dir(&stash.join(rel), &target)?;
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            let _ = fs::remove_dir_all(stash);
            log::info!("{}", tr!("wago.kept"));
        }
        Err(e) => log::warn!("{}", tr!("wago.keep_failed", error = format!("{e:#}"))),
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_game_folder() {
        let text = r#"{"app-preferences-storage":{"state":{"gameInstallation":{"wowPath":"C:\\Games\\World of Warcraft"}}}}"#;
        assert_eq!(wow_path(text), Some(PathBuf::from(r"C:\Games\World of Warcraft")));
        assert_eq!(wow_path("{}"), None);
    }

    #[test]
    fn companion_changes_are_seen() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join(r"_retail_\Interface\AddOns");
        fs::create_dir_all(addons.join("WagoAppCompanion")).unwrap();
        fs::write(tmp.path().join(r"_retail_\Wow.exe"), "").unwrap();
        let dirs = companion_dirs(tmp.path());
        assert_eq!(dirs.len(), 2);
        let before = state(&dirs);
        fs::write(addons.join(r"WagoAppCompanion\data.lua"), "x").unwrap();
        assert_ne!(state(&dirs), before);
    }

    #[test]
    fn switched_off_folders_are_put_back() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, stash) = (tmp.path().join("wow"), tmp.path().join("stash"));
        let ui = root.join(r"_retail_\Interface\AddOns\WagoUI_Storage");
        let wa = root.join(r"_classic_\Interface\AddOns\WagoAppCompanion");
        fs::create_dir_all(&ui).unwrap();
        fs::write(ui.join("data.lua"), "old").unwrap();
        keep(&stash, &root, &[&ui, &wa]).unwrap();
        // The app rewrites one and creates the other.
        fs::write(ui.join("data.lua"), "new").unwrap();
        fs::write(ui.join("extra.lua"), "new").unwrap();
        fs::create_dir_all(&wa).unwrap();
        restore_kept(&stash);
        assert_eq!(fs::read_to_string(ui.join("data.lua")).unwrap(), "old");
        assert!(!ui.join("extra.lua").exists());
        assert!(!wa.exists(), "it did not exist before");
        assert!(!stash.exists());
    }
}
