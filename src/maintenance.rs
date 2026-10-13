//! Keeping a WoW install tidy, and noticing game patches.
//!
//! - Patches: the client version of every flavor (from Battle.net, `wow::all_flavors`) is kept in
//!   `data\game-versions.json`. A new version seen while that client is closed means Battle.net
//!   has just patched it and the game has not run on it yet, so WTF and AddOns still hold the
//!   pre-patch state: `check_patches` snapshots them (label `pre-patch`) and lists the addons
//!   whose `.toc` was not made for the new version (`outdated`).
//! - Cleanup (`scan` / `clean`): settings of addons that are no longer installed, WoW's own
//!   `.bak`/`.old` copies in WTF, combat logs older than a week, screenshots (moved out of the
//!   game folder into Pictures, by date: `config::ShotsBy`) and the `Cache`/`Errors` folders. Removing
//!   anything from WTF takes a `pre-cleanup` snapshot first. Only the cache of a running flavor is left
//!   alone: the rest are files the game does not hold open (orphaned settings belong to addons it has
//!   not loaded; it rewrites its own copies on exit anyway).

use crate::app::{self, Ctx};
use crate::config::{Config, ShotsBy};
use crate::snapshot::{self, Kind, Outcome};
use crate::wow::{self, Flavor};
use anyhow::{Result, bail};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use walkdir::WalkDir;

/// Combat logs younger than this stay (an upload to Warcraft Logs may still be pending).
const LOG_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Addon settings files from this size on are listed as big.
const BIG_SV: u64 = 5 * 1024 * 1024;

// ---------------------------------------------------------------- versions and patches

/// "12.1.0.69933" -> 120100, the number addons write after `## Interface:`.
pub fn interface_of(version: &str) -> Option<u32> {
    let mut parts = version.split('.').map(|p| p.parse::<u32>().ok());
    let (major, minor, patch) = (parts.next()??, parts.next()??, parts.next().flatten().unwrap_or(0));
    Some(major * 10_000 + minor * 100 + patch)
}

/// The `## Interface` numbers of an addon folder, from all its `.toc` files (`Name.toc`,
/// `Name_Mainline.toc`, `Name-Classic.toc`, ...). A line may hold several: `110205, 50500, 11507`.
fn toc_interfaces(dir: &Path) -> Vec<u32> {
    let Some(name) = dir.file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let file = e.file_name().to_string_lossy().to_lowercase();
        if !file.ends_with(".toc") || !file.starts_with(&name) {
            continue;
        }
        let Ok(text) = fs::read_to_string(e.path()) else { continue };
        for line in text.lines().take(60) {
            let line = line.trim_start_matches('\u{feff}').trim();
            // "## Interface:" and the per-flavor "## Interface-Classic:".
            let Some(rest) = line.strip_prefix("##").map(str::trim) else { continue };
            if !rest.to_ascii_lowercase().starts_with("interface") {
                continue;
            }
            let Some((_, numbers)) = rest.split_once(':') else { continue };
            out.extend(numbers.split(',').filter_map(|n| n.trim().parse::<u32>().ok()));
        }
    }
    out
}

/// Retail is 8.0 and later; Classic flavors stay on one major version each (1.x, 2.x, 5.x).
const RETAIL_FROM: u32 = 80_000;

/// An addon not made for `current`, judged by major and minor version (patch-level bumps do not
/// count). Only the numbers for this game are compared: on Retail every Retail-era number (an
/// 11.x addon is outdated on 12.x), on Classic the same major version. An addon with no number for
/// this game is for another flavor (or a library) and is not judged.
fn is_outdated(interfaces: &[u32], current: u32) -> bool {
    let this_game = |n: &u32| {
        if current >= RETAIL_FROM {
            *n >= RETAIL_FROM
        } else {
            n / 10_000 == current / 10_000
        }
    };
    let numbers: Vec<u32> = interfaces.iter().copied().filter(this_game).collect();
    !numbers.is_empty() && numbers.iter().all(|n| n / 100 < current / 100)
}

/// Addons of `fl` not made for its client version (by `## Interface`), by folder name.
pub fn outdated(fl: &Flavor) -> Vec<String> {
    let Some(current) = fl.version.as_deref().and_then(interface_of) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(fl.dir.join(r"Interface\AddOns")) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(name, _)| !name.starts_with("Blizzard_"))
        .filter(|(_, dir)| is_outdated(&toc_interfaces(dir), current))
        .map(|(name, _)| name)
        .collect();
    out.sort_by_key(|n| n.to_lowercase());
    out
}

fn versions_path() -> PathBuf {
    crate::config::data_dir().join("game-versions.json")
}

fn load_versions() -> BTreeMap<String, String> {
    fs::read_to_string(versions_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_versions(v: &BTreeMap<String, String>) {
    let _ = fs::create_dir_all(crate::config::data_dir());
    if let Ok(text) = serde_json::to_string_pretty(v) {
        let _ = fs::write(versions_path(), text);
    }
}

/// A flavor Battle.net patched.
#[derive(Debug, Clone)]
pub struct Patch {
    pub title: String,
    pub from: String,
    pub to: String,
    /// Snapshot ids taken before the first start on the new version (empty: the game already ran).
    pub saved: Vec<String>,
    pub outdated: Vec<String>,
}

impl Patch {
    pub fn summary(&self) -> String {
        let mut s = tr!(
            "patch.updated",
            flavor = &self.title,
            from = short_version(&self.from),
            to = short_version(&self.to)
        );
        s.push('\n');
        s.push_str(if self.saved.is_empty() { tr!("patch.not_saved") } else { tr!("patch.saved") });
        if !self.outdated.is_empty() {
            s.push('\n');
            s.push_str(&tr!("patch.outdated", n = self.outdated.len(), names = names_text(&self.outdated, 5)));
        }
        s
    }
}

/// "12.1.0.69933" -> "12.1.0".
pub fn short_version(v: &str) -> String {
    v.split('.').take(3).collect::<Vec<_>>().join(".")
}

/// "A, B, C and 4 more".
pub fn names_text(names: &[String], max: usize) -> String {
    let mut s = names.iter().take(max).cloned().collect::<Vec<_>>().join(", ");
    if names.len() > max {
        s.push_str(&tr!("patch.more", n = names.len() - max));
    }
    s
}

/// Compares the client versions with the last ones seen; snapshots a patched flavor whose client
/// has not run yet. The first sight of a flavor is only remembered.
pub fn check_patches(ctx: &Ctx) -> Vec<Patch> {
    let Ok(flavors) = ctx.flavors(None) else { return Vec::new() };
    let mut known = load_versions();
    let games = wow::running_games();
    let store = ctx.store();
    let mut out = Vec::new();
    let mut changed = false;
    for fl in &flavors {
        let Some(version) = fl.version.clone() else { continue };
        let previous = known.insert(fl.name.clone(), version.clone());
        changed |= previous.as_deref() != Some(version.as_str());
        let Some(from) = previous.filter(|p| *p != version) else { continue };
        let mut saved = Vec::new();
        if wow::running_in(&games, fl).is_none() {
            let mut kinds = vec![Kind::Wtf];
            if ctx.cfg.include_addons {
                kinds.push(Kind::Addons);
            }
            for kind in kinds {
                match store.backup(fl, kind, true, Some("pre-patch")) {
                    Ok(Outcome::Created(s)) => saved.push(s.id),
                    Ok(_) => {}
                    Err(e) => log::error!("{}: {e:#}", tr!("patch.snapshot_failed", flavor = fl.title())),
                }
            }
        }
        let patch = Patch {
            title: fl.title(),
            from,
            to: version,
            saved,
            outdated: outdated(fl),
        };
        log::info!("{}", patch.summary().replace('\n', " | "));
        out.push(patch);
    }
    if changed {
        save_versions(&known);
    }
    out
}

// ---------------------------------------------------------------- cleanup

/// What can be tidied up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Item {
    /// Settings (SavedVariables) of addons that are no longer installed.
    Orphans,
    /// WoW's `.bak` / `.old` copies in WTF.
    Backups,
    /// Combat logs older than a week.
    Logs,
    /// Screenshots, moved to Pictures (`screenshots_dir`).
    Screenshots,
    /// `Cache` and `Errors` (crash reports); the game rebuilds the cache.
    Cache,
}

impl Item {
    pub const ALL: [Item; 5] = [Item::Orphans, Item::Backups, Item::Logs, Item::Screenshots, Item::Cache];

    pub fn key(self) -> &'static str {
        match self {
            Item::Orphans => "orphans",
            Item::Backups => "backups",
            Item::Logs => "logs",
            Item::Screenshots => "screenshots",
            Item::Cache => "cache",
        }
    }

    pub fn from_key(key: &str) -> Option<Item> {
        Item::ALL.into_iter().find(|i| i.key() == key)
    }

    pub fn title(self) -> &'static str {
        match self {
            Item::Orphans => tr!("cleanup.orphans"),
            Item::Backups => tr!("cleanup.backups"),
            Item::Logs => tr!("cleanup.logs"),
            Item::Screenshots => tr!("cleanup.screenshots"),
            Item::Cache => tr!("cleanup.cache"),
        }
    }

    /// Changes what a WTF snapshot holds: one comes first. `.bak`/`.old` copies are left out of
    /// snapshots by default (`exclude_ext`), and then removing them needs none.
    fn touches_wtf(self, cfg: &Config) -> bool {
        let excluded = |ext: &str| cfg.exclude_ext.iter().any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(ext));
        match self {
            Item::Orphans => true,
            Item::Backups => !(excluded("bak") && excluded("old")),
            _ => false,
        }
    }

    /// Files the game holds open: left alone in a running flavor. Everything else goes while it
    /// runs: orphaned settings belong to addons it has not loaded, `.bak` copies are made again on
    /// exit, old logs and screenshots are never read back.
    fn needs_closed_game(self) -> bool {
        self == Item::Cache
    }
}

/// What `scan` found of one item, over all flavors.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Found {
    pub files: usize,
    pub bytes: u64,
    /// Addon names (orphans) or the biggest settings files, for the subtitle.
    pub names: Vec<String>,
    paths: Vec<PathBuf>,
}

impl Found {
    /// A result with no paths behind it, for pictures of the window.
    pub fn sample(files: usize, bytes: u64, names: &[&str]) -> Found {
        Found {
            files,
            bytes,
            names: names.iter().map(|s| s.to_string()).collect(),
            paths: Vec::new(),
        }
    }
}

/// Everything the cleanup tab shows.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub items: BTreeMap<Item, Found>,
    /// The biggest addon settings files ("Details 34 MB"), biggest first; only shown.
    pub big: Vec<(String, u64)>,
    /// Addons whose settings are orphans, with their size, biggest first.
    pub orphans: Vec<(String, u64)>,
}

impl Scan {
    pub fn get(&self, item: Item) -> Found {
        self.items.get(&item).cloned().unwrap_or_default()
    }
}

/// The lists of the cleanup tab too long for a row, opened over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    Orphans,
    Big,
}

impl Scan {
    /// The entries of a list: name and size, biggest first.
    pub fn list(&self, list: List) -> &[(String, u64)] {
        match list {
            List::Orphans => &self.orphans,
            List::Big => &self.big,
        }
    }
}

/// Where screenshots go: `screenshots_dir` of the config, or `Pictures\World of Warcraft`.
pub fn screenshots_dir(cfg: &Config) -> PathBuf {
    if let Some(dir) = &cfg.screenshots_dir {
        return dir.clone();
    }
    let pictures = directories::UserDirs::new()
        .and_then(|d| d.picture_dir().map(Path::to_path_buf))
        .unwrap_or_else(|| crate::config::data_dir().join("Screenshots"));
    pictures.join("World of Warcraft")
}

fn installed_addons(fl: &Flavor) -> Option<HashSet<String>> {
    let entries = fs::read_dir(fl.dir.join(r"Interface\AddOns")).ok()?;
    Some(entries.flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect())
}

/// "Foo.lua" / "Foo.lua.bak" -> "Foo".
fn sv_addon(file: &str) -> Option<&str> {
    let lower = file.to_ascii_lowercase();
    let len = if lower.ends_with(".lua.bak") || lower.ends_with(".lua.old") {
        8
    } else if lower.ends_with(".lua") {
        4
    } else {
        return None;
    };
    Some(&file[..file.len() - len])
}

fn size_of(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn dir_files(dir: &Path) -> Vec<PathBuf> {
    WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect()
}

fn add(found: &mut Found, path: PathBuf) {
    found.files += 1;
    found.bytes += size_of(&path);
    found.paths.push(path);
}

/// Looks through the flavors; nothing is changed.
pub fn scan(flavors: &[Flavor]) -> Scan {
    let mut scan = Scan::default();
    let now = SystemTime::now();
    let mut orphan_sizes: BTreeMap<String, u64> = BTreeMap::new();
    let mut big: BTreeMap<String, u64> = BTreeMap::new();
    for fl in flavors {
        // WTF: addon settings and WoW's copies of them.
        let addons = installed_addons(fl);
        for e in WalkDir::new(fl.wtf_dir()).into_iter().flatten().filter(|e| e.file_type().is_file()) {
            let file = e.file_name().to_string_lossy().into_owned();
            let in_sv = e
                .path()
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|p| p.eq_ignore_ascii_case("SavedVariables"));
            let addon = in_sv.then(|| sv_addon(&file)).flatten();
            let lower = file.to_ascii_lowercase();
            match (addon, &addons) {
                // Settings of an addon that is not installed (Blizzard's own UI keeps its files).
                (Some(name), Some(installed)) if !installed.contains(&name.to_lowercase()) && !name.to_lowercase().starts_with("blizzard") => {
                    *orphan_sizes.entry(name.to_string()).or_default() += size_of(e.path());
                    add(scan.items.entry(Item::Orphans).or_default(), e.into_path());
                }
                _ if lower.ends_with(".bak") || lower.ends_with(".old") => add(scan.items.entry(Item::Backups).or_default(), e.into_path()),
                (Some(name), _) if lower.ends_with(".lua") => {
                    let size = size_of(e.path());
                    let entry = big.entry(name.to_string()).or_default();
                    *entry = (*entry).max(size);
                }
                _ => {}
            }
        }
        // Combat logs older than a week.
        if let Ok(entries) = fs::read_dir(fl.dir.join("Logs")) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                let old = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| now.duration_since(t).unwrap_or_default() >= LOG_AGE);
                if name.starts_with("wowcombatlog") && name.ends_with(".txt") && old {
                    add(scan.items.entry(Item::Logs).or_default(), e.path());
                }
            }
        }
        // Screenshots lying in the game folder.
        if let Ok(entries) = fs::read_dir(fl.dir.join("Screenshots")) {
            for e in entries.flatten().filter(|e| e.path().is_file()) {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                if [".jpg", ".jpeg", ".tga", ".png"].iter().any(|x| name.ends_with(x)) {
                    add(scan.items.entry(Item::Screenshots).or_default(), e.path());
                }
            }
        }
        // Cache and crash reports.
        for dir in ["Cache", "Errors"] {
            for path in dir_files(&fl.dir.join(dir)) {
                add(scan.items.entry(Item::Cache).or_default(), path);
            }
        }
    }
    if let Some(f) = scan.items.get_mut(&Item::Orphans) {
        f.names = orphan_sizes.keys().cloned().collect();
    }
    let mut big: Vec<(String, u64)> = big.into_iter().filter(|(_, s)| *s >= BIG_SV).collect();
    big.sort_by_key(|(_, s)| std::cmp::Reverse(*s));
    scan.big = big;
    let mut orphans: Vec<(String, u64)> = orphan_sizes.into_iter().collect();
    orphans.sort_by_key(|(_, s)| std::cmp::Reverse(*s));
    scan.orphans = orphans;
    scan
}

/// Does `item` for every flavor it may touch now (`Item::needs_closed_game`); returns a summary.
pub fn clean(ctx: &Ctx, item: Item) -> Result<String> {
    let flavors = ctx.flavors(None)?;
    let games = wow::running_games();
    let (closed, running): (Vec<Flavor>, Vec<Flavor>) = flavors
        .into_iter()
        .partition(|fl| !item.needs_closed_game() || wow::running_in(&games, fl).is_none());
    if closed.is_empty() {
        bail!(tr!("cleanup.game_running"));
    }
    let skipped = |s: &mut String| {
        if !running.is_empty() {
            let titles: Vec<String> = running.iter().map(Flavor::title).collect();
            s.push('\n');
            s.push_str(&tr!("cleanup.skipped_running", flavors = titles.join(", ")));
        }
    };
    let found = scan(&closed).get(item);
    if found.files == 0 {
        let mut s = tr!("cleanup.nothing").to_string();
        skipped(&mut s);
        log::info!("{}: {}", item.key(), s.replace('\n', " | "));
        return Ok(s);
    }
    let safety = item.touches_wtf(&ctx.cfg);
    if safety {
        for fl in &closed {
            if let Some(id) = app::safety_snapshot(ctx, fl, "pre-cleanup")? {
                log::info!("{}", tr!("cleanup.saved", flavor = fl.title(), id = id));
            }
        }
    }
    let mut done = Found::default();
    let mut failed = 0;
    for path in &found.paths {
        let size = size_of(path);
        let result = match item {
            Item::Screenshots => move_screenshot(path, &closed, &ctx.cfg),
            _ => fs::remove_file(path).map_err(Into::into),
        };
        match result {
            Ok(()) => {
                done.files += 1;
                done.bytes += size;
            }
            Err(e) => {
                failed += 1;
                log::warn!("{}: {e:#}", path.display());
            }
        }
    }
    if safety {
        for fl in &closed {
            app::safety_done(ctx, fl);
        }
    }
    if item == Item::Cache {
        // Empty folders left behind; the game makes them again.
        for fl in &closed {
            for dir in ["Cache", "Errors"] {
                remove_empty_dirs(&fl.dir.join(dir));
            }
        }
    }
    let mut s = match item {
        Item::Screenshots => tr!("cleanup.moved", n = done.files, size = snapshot::human_size(done.bytes)),
        _ => tr!("cleanup.removed", n = done.files, size = snapshot::human_size(done.bytes)),
    };
    if failed > 0 {
        s.push('\n');
        s.push_str(&tr!("cleanup.failed_files", n = failed));
    }
    skipped(&mut s);
    log::info!("{}: {}", item.key(), s.replace('\n', " | "));
    Ok(s)
}

/// When a screenshot was taken: from WoW's name (`WoWScrnShot_MMDDYY_HHMMSS.jpg`), else the file time.
fn shot_date(path: &Path) -> Option<chrono::NaiveDate> {
    let stem = path.file_stem()?.to_string_lossy();
    let named = stem
        .split('_')
        .nth(1)
        .filter(|d| d.len() == 6)
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%m%d%y").ok());
    named.or_else(|| {
        let t = fs::metadata(path).and_then(|m| m.modified()).ok()?;
        Some(chrono::DateTime::<chrono::Local>::from(t).date_naive())
    })
}

/// `<flavor>\Screenshots\x.jpg` -> `<screenshots_dir>\[<flavor title>\][<date folder>\]x.jpg`.
fn move_screenshot(path: &Path, flavors: &[Flavor], cfg: &Config) -> Result<()> {
    let mut dir = screenshots_dir(cfg);
    if cfg.screenshots_per_flavor {
        let title = flavors
            .iter()
            .find(|fl| path.starts_with(&fl.dir))
            .map_or_else(|| "World of Warcraft".to_string(), Flavor::title);
        dir.push(title);
    }
    let format = match cfg.screenshots_by {
        ShotsBy::Year => Some("%Y"),
        ShotsBy::Month => Some("%Y-%m"),
        ShotsBy::Day => Some("%Y-%m-%d"),
        ShotsBy::None => None,
    };
    if let Some(format) = format {
        dir.push(shot_date(path).map_or_else(|| "unknown".to_string(), |d| d.format(format).to_string()));
    }
    fs::create_dir_all(&dir)?;
    let name = path.file_name().unwrap_or_default();
    let mut target = dir.join(name);
    // Never overwrite: "x (2).jpg".
    let mut n = 2;
    while target.exists() {
        let stem = Path::new(name).file_stem().unwrap_or_default().to_string_lossy();
        let ext = Path::new(name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        target = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    // A rename fails across drives: copy, then remove.
    if fs::rename(path, &target).is_err() {
        fs::copy(path, &target)?;
        fs::remove_file(path)?;
    }
    Ok(())
}

fn remove_empty_dirs(dir: &Path) {
    let mut dirs: Vec<PathBuf> = WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.into_path())
        .collect();
    // Deepest first.
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs.iter().filter(|d| d.as_path() != dir) {
        let _ = fs::remove_dir(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn bak_cleanup_needs_a_snapshot_only_when_snapshots_keep_them() {
        let mut cfg = Config::default();
        assert!(!Item::Backups.touches_wtf(&cfg), "left out of snapshots by default");
        cfg.exclude_ext = vec![".bak".into()];
        assert!(Item::Backups.touches_wtf(&cfg), ".old copies are in snapshots now");
        assert!(Item::Orphans.touches_wtf(&cfg));
        assert!(!Item::Logs.touches_wtf(&cfg));
    }

    fn touch(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    fn set_age(path: &Path, days: u64) {
        let t = SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
        File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
    }

    #[test]
    fn interface_numbers() {
        assert_eq!(interface_of("12.1.0.69933"), Some(120100));
        assert_eq!(interface_of("1.15.7.61582"), Some(11507));
        assert_eq!(interface_of("1.60.1.70009"), Some(16001));
        assert_eq!(interface_of("x"), None);
        // Same game, older minor: outdated; a newer or equal minor (any patch) is fine.
        assert!(is_outdated(&[110205], 120100));
        assert!(is_outdated(&[120000], 120100));
        assert!(!is_outdated(&[120100], 120105));
        assert!(is_outdated(&[120001, 11507], 120100));
        assert!(!is_outdated(&[120100, 11507], 120100));
        // Nothing for this game: not judged.
        assert!(!is_outdated(&[11507], 120100));
        assert!(!is_outdated(&[], 120100));
        // Classic Era: only 1.x numbers count.
        assert!(!is_outdated(&[120100, 11507], 11507));
        assert!(is_outdated(&[11404], 11507));
        assert!(!is_outdated(&[120100], 11507));
    }

    #[test]
    fn outdated_addons_are_found_by_toc() {
        let dir = tempfile::tempdir().unwrap();
        let mut fl = Flavor::new("_retail_", dir.path().join("_retail_"));
        fl.version = Some("12.1.0.69933".into());
        let addons = fl.dir.join(r"Interface\AddOns");
        let toc = |name: &str, file: &str, text: &str| {
            fs::create_dir_all(addons.join(name)).unwrap();
            fs::write(addons.join(name).join(file), text).unwrap();
        };
        toc("Fresh", "Fresh.toc", "## Interface: 120100\n## Title: Fresh");
        toc("Old", "Old.toc", "\u{feff}## Interface: 110207\n");
        toc("Multi", "Multi_Mainline.toc", "## Interface: 120100\n");
        toc("Multi", "Multi_Vanilla.toc", "## Interface: 11507\n");
        toc("ClassicOnly", "ClassicOnly.toc", "## Interface: 11507\n");
        toc("Blizzard_Thing", "Blizzard_Thing.toc", "## Interface: 10000\n");
        assert_eq!(outdated(&fl), ["Old"]);
    }

    #[test]
    fn scan_finds_what_can_go_and_clean_takes_it() {
        let dir = tempfile::tempdir().unwrap();
        let fl = Flavor::new("_retail_", dir.path().join("_retail_"));
        let acc = fl.wtf_dir().join(r"Account\ACC");
        fs::create_dir_all(fl.dir.join(r"Interface\AddOns\Details")).unwrap();
        touch(&acc.join(r"SavedVariables\Details.lua"), 6 * 1024 * 1024);
        touch(&acc.join(r"SavedVariables\Details.lua.bak"), 10);
        touch(&acc.join(r"SavedVariables\Gone.lua"), 100);
        touch(&acc.join(r"SavedVariables\Gone.lua.bak"), 100);
        touch(&acc.join(r"SavedVariables\Blizzard_Console.lua"), 10);
        touch(&acc.join(r"Realm\Char\SavedVariables\Gone.lua"), 50);
        touch(&acc.join(r"Realm\Char\config-cache.wtf"), 10);
        touch(&acc.join(r"Realm\Char\config-cache.old"), 10);
        touch(&fl.dir.join(r"Logs\WoWCombatLog-101226_200000.txt"), 1000);
        touch(&fl.dir.join(r"Logs\WoWCombatLog-091226_200000.txt"), 2000);
        set_age(&fl.dir.join(r"Logs\WoWCombatLog-091226_200000.txt"), 30);
        touch(&fl.dir.join(r"Logs\FrameXML.log"), 5);
        touch(&fl.dir.join(r"Screenshots\WoWScrnShot_101226_200000.jpg"), 300);
        touch(&fl.dir.join(r"Cache\WDB\enUS\creaturecache.wdb"), 400);
        touch(&fl.dir.join(r"Errors\crash.dmp"), 500);

        let s = scan(std::slice::from_ref(&fl));
        let orphans = s.get(Item::Orphans);
        assert_eq!((orphans.files, orphans.bytes, orphans.names.clone()), (3, 250, vec!["Gone".to_string()]));
        assert_eq!((s.get(Item::Backups).files, s.get(Item::Backups).bytes), (2, 20));
        assert_eq!((s.get(Item::Logs).files, s.get(Item::Logs).bytes), (1, 2000));
        assert_eq!(s.get(Item::Screenshots).files, 1);
        assert_eq!((s.get(Item::Cache).files, s.get(Item::Cache).bytes), (2, 900));
        assert_eq!(s.big, [("Details".to_string(), 6 * 1024 * 1024)]);

        // Removing goes by the paths found.
        for path in &s.get(Item::Orphans).paths {
            fs::remove_file(path).unwrap();
        }
        assert!(acc.join(r"SavedVariables\Details.lua").exists());
        assert!(!acc.join(r"SavedVariables\Gone.lua").exists());
        assert!(acc.join(r"SavedVariables\Blizzard_Console.lua").exists());
    }

    #[test]
    fn screenshots_move_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let fl = Flavor::new("_retail_", dir.path().join("_retail_"));
        let shot = fl.dir.join(r"Screenshots\a.jpg");
        touch(&shot, 3);
        let month = chrono::Local::now().format("%Y-%m").to_string();
        let root = dir.path().join("Pictures");
        let mut cfg = Config {
            screenshots_dir: Some(root.clone()),
            ..Config::default()
        };
        let target = root.join("Retail").join(&month);
        touch(&target.join("a.jpg"), 1);
        move_screenshot(&shot, std::slice::from_ref(&fl), &cfg).unwrap();
        assert!(!shot.exists());
        assert_eq!(fs::read(target.join("a.jpg")).unwrap().len(), 1);
        assert_eq!(fs::read(target.join("a (2).jpg")).unwrap().len(), 3);

        // By day, no flavor folder; the date comes from WoW's file name.
        cfg.screenshots_by = ShotsBy::Day;
        cfg.screenshots_per_flavor = false;
        let shot = fl.dir.join(r"Screenshots\WoWScrnShot_042426_234920.jpg");
        touch(&shot, 2);
        move_screenshot(&shot, std::slice::from_ref(&fl), &cfg).unwrap();
        assert!(root.join(r"2026-04-24\WoWScrnShot_042426_234920.jpg").exists());
        // All in one folder.
        cfg.screenshots_by = ShotsBy::None;
        let shot = fl.dir.join(r"Screenshots\b.png");
        touch(&shot, 2);
        move_screenshot(&shot, std::slice::from_ref(&fl), &cfg).unwrap();
        assert!(root.join("b.png").exists());
    }

    #[test]
    fn saved_variables_names() {
        assert_eq!(sv_addon("Details.lua"), Some("Details"));
        assert_eq!(sv_addon("Details.lua.bak"), Some("Details"));
        assert_eq!(sv_addon("bindings-cache.wtf"), None);
    }
}
