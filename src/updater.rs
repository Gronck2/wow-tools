//! Addon updates through an addon manager the user already has: CurseForge, WowUp, WowUp-CF or
//! the Wago App (that one only WeakAuras and UI packs, see `wago.rs`).
//! Their APIs need per-developer keys, so instead of downloading ourselves the chosen app is
//! started without a window, left to install its updates, and closed again (only when we started
//! it). One app at a time: two managers over the same AddOns folder fight.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};
use sysinfo::{ProcessesToUpdate, System};

use crate::{curseforge, wago, wowup};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddonApp {
    #[serde(rename = "curseforge")]
    CurseForge,
    #[serde(rename = "wowup")]
    WowUp,
    #[serde(rename = "wowup_cf")]
    WowUpCf,
    #[serde(rename = "wago")]
    Wago,
}

impl AddonApp {
    pub const ALL: [AddonApp; 4] = [AddonApp::CurseForge, AddonApp::WowUp, AddonApp::WowUpCf, AddonApp::Wago];

    pub fn title(self) -> &'static str {
        match self {
            AddonApp::CurseForge => "CurseForge",
            AddonApp::WowUp => "WowUp",
            AddonApp::WowUpCf => "WowUp-CF",
            AddonApp::Wago => "Wago App",
        }
    }

    /// The installed app, if any.
    pub fn exe(self) -> Option<PathBuf> {
        match self {
            AddonApp::CurseForge => curseforge::exe(),
            AddonApp::WowUp => wowup::WowUp::FREE.exe_path(),
            AddonApp::WowUpCf => wowup::WowUp::CF.exe_path(),
            AddonApp::Wago => wago::exe(),
        }
    }

    pub fn installed() -> Vec<AddonApp> {
        AddonApp::ALL.into_iter().filter(|a| a.exe().is_some()).collect()
    }

    /// Starts the app hidden, waits for its updates, closes it. `wago`: what the Wago App updates
    /// (WeakAuras, UI packs).
    pub fn update(self, wago: (bool, bool)) -> Result<Outcome> {
        let exe = self.exe().with_context(|| tr!("addons.app_not_found", app = self.title()))?;
        match self {
            AddonApp::CurseForge => curseforge::update(&exe),
            AddonApp::WowUp => wowup::WowUp::FREE.update(&exe),
            AddonApp::WowUpCf => wowup::WowUp::CF.update(&exe),
            AddonApp::Wago => wago::update(&exe, wago.0, wago.1),
        }
    }

    /// What the app manages, for `status`: one line per WoW install it knows.
    pub fn describe(self) -> Result<Vec<String>> {
        match self {
            AddonApp::CurseForge => curseforge::describe(),
            AddonApp::WowUp => wowup::WowUp::FREE.describe(),
            AddonApp::WowUpCf => wowup::WowUp::CF.describe(),
            AddonApp::Wago => wago::describe(),
        }
    }
}

/// The app updates go through: the configured one while it is installed, else the first one
/// installed.
pub fn chosen(configured: Option<AddonApp>) -> Option<AddonApp> {
    pick(configured, &AddonApp::installed())
}

pub fn pick(configured: Option<AddonApp>, installed: &[AddonApp]) -> Option<AddonApp> {
    configured.filter(|a| installed.contains(a)).or_else(|| installed.first().copied())
}

pub struct Outcome {
    /// Names of the addons that got a new version.
    pub updated: Vec<String>,
    /// Addons still not on their newest version (auto-update off for them, or failed).
    pub outdated: Vec<String>,
    /// Something worth telling: the app was already open, the wait timed out, ...
    pub note: Option<String>,
}

impl Outcome {
    pub fn note(text: impl Into<String>) -> Outcome {
        Outcome {
            updated: Vec::new(),
            outdated: Vec::new(),
            note: Some(text.into()),
        }
    }

    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        match self.updated.len() {
            0 => lines.push(tr!("addons.none").to_string()),
            n => lines.push(tr!("addons.updated", n = n, names = self.updated.join(", "))),
        }
        if !self.outdated.is_empty() {
            lines.push(tr!("addons.left_outdated", names = self.outdated.join(", ")));
        }
        lines.extend(self.note.clone());
        lines.join("\n")
    }
}

// ---------------------------------------------------------------- processes

pub fn running(exe_name: &str) -> bool {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.processes().values().any(|p| p.name().eq_ignore_ascii_case(exe_name))
}

/// Runs a program without a window and without inheriting our (possibly dead) std handles.
pub fn quiet_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Closes an app: politely first, then for sure.
pub fn close(exe_name: &str) {
    close_within(exe_name, Duration::from_secs(15));
}

/// Like `close`, giving the app `grace` to quit by itself.
pub fn close_within(exe_name: &str, grace: Duration) {
    let _ = quiet_command("taskkill").args(["/IM", exe_name, "/T"]).status();
    let until = Instant::now() + grace;
    while running(exe_name) && Instant::now() < until {
        std::thread::sleep(Duration::from_secs(1));
    }
    if running(exe_name) {
        let _ = quiet_command("taskkill").args(["/IM", exe_name, "/T", "/F"]).status();
    }
}

pub fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Writes JSON the way Electron apps (`JSON.stringify(v, null, "\t")`) do, keeping key order
/// and numbers as they were (`preserve_order`, `arbitrary_precision`).
pub fn to_tab_json(value: &serde_json::Value) -> Result<String> {
    use serde::Serialize;
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(b"\t"));
    value.serialize(&mut ser)?;
    Ok(String::from_utf8(out)?)
}

// ---------------------------------------------------------------- last run

/// Where the time of the last update run is kept (the schedule catches up from it).
fn last_file() -> PathBuf {
    crate::config::data_dir().join("addon-update.last")
}

pub fn last_run() -> Option<chrono::NaiveDateTime> {
    let text = fs::read_to_string(last_file()).ok()?;
    chrono::NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%d %H:%M:%S").ok()
}

pub fn set_last_run(t: chrono::NaiveDateTime) {
    let _ = fs::create_dir_all(crate::config::data_dir());
    if let Err(e) = fs::write(last_file(), t.format("%Y-%m-%d %H:%M:%S").to_string()) {
        log::warn!("{}", tr!("addons.last_run_write_failed", error = e));
    }
}

// ---------------------------------------------------------------- history

/// How many past runs are kept.
const HISTORY_KEEP: usize = 20;

/// One update run, for the panel's history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub time: chrono::NaiveDateTime,
    /// The app's title ("CurseForge").
    pub app: String,
    pub ok: bool,
    /// The summary, or the error.
    pub text: String,
    /// Started from the menu or the panel, not by the schedule.
    pub manual: bool,
}

fn history_file() -> PathBuf {
    crate::config::data_dir().join("addon-updates.json")
}

/// Past runs, newest first.
pub fn history() -> Vec<Entry> {
    read_history(&history_file())
}

fn read_history(path: &Path) -> Vec<Entry> {
    fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn record(entry: Entry) {
    if let Err(e) = append_history(&history_file(), entry) {
        log::warn!("{}", tr!("addons.history_write_failed", error = format!("{e:#}")));
    }
}

fn append_history(path: &Path, entry: Entry) -> Result<()> {
    let mut list = read_history(path);
    list.insert(0, entry);
    list.truncate(HISTORY_KEEP);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, serde_json::to_string_pretty(&list)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_keeps_the_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("h.json");
        let at = |m| chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap().and_hms_opt(10, m, 0).unwrap();
        for m in 0..25 {
            let entry = Entry {
                time: at(m),
                app: "WowUp".into(),
                ok: m % 2 == 0,
                text: format!("run {m}"),
                manual: false,
            };
            append_history(&path, entry).unwrap();
        }
        let list = read_history(&path);
        assert_eq!(list.len(), HISTORY_KEEP);
        assert_eq!(list[0].text, "run 24");
        assert_eq!(list[0].time, at(24));
    }

    #[test]
    fn summary_text() {
        let o = Outcome {
            updated: vec!["BigWigs".into(), "Details!".into()],
            outdated: Vec::new(),
            note: None,
        };
        assert_eq!(o.summary(), "Обновлено 2: BigWigs, Details!");
    }

    #[test]
    fn app_names_in_config() {
        #[derive(Deserialize)]
        struct T {
            a: Vec<AddonApp>,
        }
        let t: T = toml::from_str("a = ['curseforge', 'wowup', 'wowup_cf', 'wago']").unwrap();
        assert_eq!(t.a, AddonApp::ALL);
    }

    #[test]
    fn tab_json_matches_electron() {
        let text = "{\n\t\"b\": {\n\t\t\"x\": 1.50,\n\t\t\"list\": [],\n\t\t\"o\": {}\n\t},\n\t\"a\": [\n\t\t\"п\"\n\t]\n}";
        let v: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(to_tab_json(&v).unwrap(), text);
    }
}
