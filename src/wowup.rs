//! Addon updates through WowUp (wowup.io) and its CurseForge build WowUp-CF: two apps from the
//! same source with separate data. `--hidden --quit` starts it without a window, runs its
//! auto-update once ("onAutoUpdateInterval" in its log) and quits by itself. Only addons with
//! `autoUpdateEnabled` are updated, and that flag is off by default, so for our run it is switched
//! on in `addons.json` for every addon that is not ignored (the app closed) and back afterwards.

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::updater::{Outcome, close, modified, quiet_command, running, to_tab_json};

/// The log line of the auto-update run.
const STARTED_MARK: &str = "onAutoUpdateInterval";
/// The free Wago access (a token from the ad in its window) did not come.
const NO_TOKEN_MARK: &str = "[wago] no token received";
/// The app counts as done once its addon list has not changed for this long after the run began
/// (it normally quits by itself sooner).
const QUIET: Duration = Duration::from_secs(90);
const TIMEOUT: Duration = Duration::from_secs(10 * 60);
const POLL: Duration = Duration::from_secs(3);
const AUTO_KEY: &str = "autoUpdateEnabled";

pub struct WowUp {
    pub title: &'static str,
    pub exe: &'static str,
    /// Under `%LOCALAPPDATA%\Programs`.
    program_dir: &'static str,
    /// Under `%APPDATA%`.
    data_dir: &'static str,
    /// For our own files.
    key: &'static str,
}

impl WowUp {
    pub const FREE: WowUp = WowUp {
        title: "WowUp",
        exe: "WowUp.exe",
        program_dir: "wowup",
        data_dir: "WowUp",
        key: "wowup",
    };
    pub const CF: WowUp = WowUp {
        title: "WowUp-CF",
        exe: "WowUp-CF.exe",
        program_dir: "wowup-cf",
        data_dir: "WowUpCf",
        key: "wowup-cf",
    };

    pub fn exe_path(&self) -> Option<PathBuf> {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
        Some(local.join("Programs").join(self.program_dir).join(self.exe)).filter(|p| p.is_file())
    }

    fn data(&self) -> PathBuf {
        std::env::var_os("APPDATA")
            .map_or_else(|| PathBuf::from("."), PathBuf::from)
            .join(self.data_dir)
    }

    fn addons_file(&self) -> PathBuf {
        self.data().join("addons.json")
    }

    fn log_file(&self) -> PathBuf {
        self.data().join(r"logs\main.log")
    }

    /// The addons whose auto-update we switched on and have to switch off again; kept on disk so
    /// a run cut short is undone next time.
    fn forced_file(&self) -> PathBuf {
        crate::config::data_dir().join(format!("{}-forced.json", self.key))
    }

    fn addons(&self) -> Result<Vec<Addon>> {
        let path = self.addons_file();
        let text = fs::read_to_string(&path).with_context(|| tr!("addons.not_set_up", app = self.title, path = path.display()))?;
        parse_addons(&text)
    }

    /// One line per WoW install the app knows, for `status`.
    pub fn describe(&self) -> Result<Vec<String>> {
        let addons = self.addons()?;
        let installs = installations(&fs::read_to_string(self.data().join("preferences.json")).unwrap_or_default());
        Ok(installs
            .iter()
            .map(|(id, location)| {
                let mine: Vec<&Addon> = addons.iter().filter(|a| a.installation == *id).collect();
                let auto = mine.iter().filter(|a| a.auto_update).count();
                let outdated: Vec<&str> = mine.iter().filter(|a| a.outdated()).map(|a| a.name.as_str()).collect();
                let fresh = match outdated.len() {
                    0 => tr!("addons.all_fresh").to_string(),
                    n => tr!("addons.waiting", n = n, names = outdated.join(", ")),
                };
                tr!("wowup.describe", location = location, n = mine.len(), auto = auto, fresh = fresh)
            })
            .collect())
    }

    /// Starts the app hidden, lets it run its auto-update and quit, then reports what changed.
    /// When it is already running it is left alone.
    pub fn update(&self, exe: &Path) -> Result<Outcome> {
        let before = self.addons()?;
        if before.is_empty() {
            bail!(tr!("wowup.no_addons", app = self.title));
        }
        if running(self.exe) {
            let note = if before.iter().all(|a| a.auto_update || a.ignored) {
                tr!("wowup.open_auto", app = self.title)
            } else {
                tr!("wowup.open_manual", app = self.title)
            };
            return Ok(Outcome::note(note));
        }

        self.restore_auto_update();
        let forced = self.force_auto_update()?;
        if !forced.is_empty() {
            log::info!("{}", tr!("wowup.forced_on", app = self.title, n = forced.len()));
        }
        let result = self.run_app(exe, before);
        self.restore_auto_update();
        result
    }

    fn run_app(&self, exe: &Path, before: Vec<Addon>) -> Result<Outcome> {
        log::info!("{}", tr!("wowup.starting", app = self.title));
        let log_from = fs::metadata(self.log_file()).map_or(0, |m| m.len());
        let started = Instant::now();
        let mut child = quiet_command(exe)
            .args(["--hidden", "--quit"])
            .spawn()
            .with_context(|| tr!("addons.start_failed", path = exe.display()))?;

        let mut last = modified(&self.addons_file());
        let mut quiet_since = Instant::now();
        let mut timed_out = true;
        while started.elapsed() < TIMEOUT {
            std::thread::sleep(POLL);
            // `--quit`: it exits by itself once the auto-update is done.
            if child.try_wait().ok().flatten().is_some() && !running(self.exe) {
                timed_out = false;
                break;
            }
            let now = modified(&self.addons_file());
            if now != last {
                last = now;
                quiet_since = Instant::now();
            }
            // It waits for its "updated" notification to be closed before quitting.
            if log_since(&self.log_file(), log_from).contains(STARTED_MARK) && quiet_since.elapsed() >= QUIET {
                timed_out = false;
                break;
            }
        }
        if running(self.exe) {
            close(self.exe);
        }
        log::info!("{}", tr!("addons.closed_after", app = self.title, secs = started.elapsed().as_secs()));

        let after = self.addons()?;
        let mut note = Vec::new();
        if timed_out {
            note.push(tr!("addons.timeout", app = self.title, min = TIMEOUT.as_secs() / 60));
        }
        // Addons it found nowhere ("Unknown") are ignored by it too; only the ones the user chose to
        // ignore are worth a word.
        let ignored: Vec<&str> = after.iter().filter(|a| a.ignored && a.known).map(|a| a.name.as_str()).collect();
        if !ignored.is_empty() {
            note.push(tr!("wowup.ignored", app = self.title, names = ignored.join(", ")));
        }
        let updated = updated(&before, &after);
        let log = log_since(&self.log_file(), log_from);
        let failed: Vec<String> = attempted(&log).into_iter().filter(|n| !updated.contains(n)).collect();
        if !failed.is_empty() {
            let why = if log.contains(NO_TOKEN_MARK) {
                tr!("wowup.wago_denied", app = self.title)
            } else {
                String::new()
            };
            if updated.is_empty() {
                bail!(tr!("wowup.failed_all", app = self.title, names = failed.join(", "), why = why));
            }
            note.push(tr!("wowup.failed_some", names = failed.join(", "), why = why));
        }
        Ok(Outcome {
            updated,
            outdated: after.iter().filter(|a| a.outdated()).map(|a| a.name.clone()).collect(),
            note: (!note.is_empty()).then(|| note.join("\n")),
        })
    }

    fn force_auto_update(&self) -> Result<Vec<String>> {
        let path = self.addons_file();
        let text = fs::read_to_string(&path)?;
        let (new_text, changed) = set_auto_update(&text, None, true)?;
        if changed.is_empty() {
            return Ok(changed);
        }
        // The file as it was, in case anything goes wrong.
        fs::copy(&path, path.with_extension("json.wow-tools.bak"))?;
        fs::create_dir_all(crate::config::data_dir())?;
        fs::write(self.forced_file(), serde_json::to_string(&changed)?)?;
        fs::write(&path, new_text)?;
        Ok(changed)
    }

    /// Switches auto-update off again where we switched it on (the app must be closed).
    fn restore_auto_update(&self) {
        let Ok(text) = fs::read_to_string(self.forced_file()) else { return };
        let ids: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
        let path = self.addons_file();
        let result = fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|t| set_auto_update(&t, Some(&ids), false))
            .and_then(|(t, _)| Ok(fs::write(&path, t)?));
        match result {
            Ok(()) => {
                let _ = fs::remove_file(self.forced_file());
                log::info!("{}", tr!("addons.auto_restored", app = self.title));
            }
            Err(e) => log::warn!("{}", tr!("addons.auto_restore_failed", app = self.title, error = format!("{e:#}"))),
        }
    }
}

/// Names of the addons it tried to update: `[AddonUpdate] Wago rkGrrgGy BugSack 'v1' -> 'v2'`.
fn attempted(log: &str) -> Vec<String> {
    log.lines()
        .filter_map(|l| l.split_once("[AddonUpdate] ").map(|(_, rest)| rest))
        // Provider, id, then the name up to the quoted versions.
        .filter_map(|rest| rest.splitn(3, ' ').nth(2))
        .filter_map(|rest| {
            rest.rsplit_once(" '")
                .and_then(|(head, _)| head.rsplit_once(" '"))
                .map(|(name, _)| name.trim().to_string())
        })
        .collect()
}

/// What the app has written to its log since `from` bytes.
fn log_since(path: &Path, from: u64) -> String {
    let Ok(bytes) = fs::read(path) else { return String::new() };
    // The log was rotated: all of it is new.
    let from = if from as usize > bytes.len() { 0 } else { from as usize };
    String::from_utf8_lossy(&bytes[from..]).into_owned()
}

#[derive(Debug)]
struct Addon {
    /// installationId, providerName, externalId: stable across the app's re-syncs, unlike `id`.
    key: String,
    name: String,
    installation: String,
    /// The installed release (`installedExternalReleaseId`, else `installedVersion`).
    installed: String,
    latest: String,
    auto_update: bool,
    ignored: bool,
    /// Matched to a site (`providerName` other than "Unknown").
    known: bool,
}

impl Addon {
    fn outdated(&self) -> bool {
        !self.ignored && !self.latest.is_empty() && self.installed != self.latest
    }
}

fn parse_addons(text: &str) -> Result<Vec<Addon>> {
    let json: serde_json::Value = serde_json::from_str(text).with_context(|| tr!("addons.list_unreadable", app = "WowUp"))?;
    let s = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
    let mut list: Vec<Addon> = json
        .as_object()
        .into_iter()
        .flat_map(|o| o.values())
        .map(|a| {
            let (installed, latest) = match a["installedExternalReleaseId"].as_str() {
                Some(id) => (id.to_string(), s(&a["externalLatestReleaseId"])),
                None => (s(&a["installedVersion"]), s(&a["latestVersion"])),
            };
            Addon {
                key: match a["externalId"].as_str().filter(|id| !id.is_empty()) {
                    Some(id) => format!("{}/{}/{id}", s(&a["installationId"]), s(&a["providerName"])),
                    None => format!("{}/?/{}", s(&a["installationId"]), s(&a["name"])),
                },
                name: s(&a["name"]),
                installation: s(&a["installationId"]),
                installed,
                latest,
                auto_update: a[AUTO_KEY].as_bool().unwrap_or(false),
                ignored: a["isIgnored"].as_bool().unwrap_or(false),
                known: !matches!(a["providerName"].as_str(), None | Some("" | "Unknown")),
            }
        })
        .collect();
    list.sort_by_key(|a| a.name.to_lowercase());
    Ok(list)
}

/// WoW installs from `preferences.json`: (id, location of the exe).
fn installations(text: &str) -> Vec<(String, String)> {
    let json: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    json["wow_installations"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|i| (i["id"].as_str().unwrap_or_default().into(), i["location"].as_str().unwrap_or_default().into()))
        .collect()
}

/// Addons whose installed release changed between two reads (ignored ones are never updated).
fn updated(before: &[Addon], after: &[Addon]) -> Vec<String> {
    let old: HashMap<&str, &str> = before.iter().map(|a| (a.key.as_str(), a.installed.as_str())).collect();
    after
        .iter()
        .filter(|a| !a.ignored)
        .filter(|a| old.get(a.key.as_str()).is_some_and(|i| *i != a.installed))
        .map(|a| a.name.clone())
        .collect()
}

/// Sets `autoUpdateEnabled` of the addons `ids` (or of every addon that has it off and is not
/// ignored, when `ids` is `None`) to `on`; returns the ids changed. The file keeps its tab-indented
/// layout, key order and numbers.
fn set_auto_update(text: &str, ids: Option<&[String]>, on: bool) -> Result<(String, Vec<String>)> {
    let mut json: serde_json::Value = serde_json::from_str(text).with_context(|| tr!("addons.list_unreadable", app = "WowUp"))?;
    let mut changed = Vec::new();
    for (id, a) in json.as_object_mut().into_iter().flatten() {
        let wanted = match ids {
            Some(list) => list.contains(id),
            None => a[AUTO_KEY].as_bool() == Some(false) && a["isIgnored"].as_bool() != Some(true),
        };
        if wanted && a.get(AUTO_KEY).is_some() && a[AUTO_KEY].as_bool() != Some(on) {
            a[AUTO_KEY] = serde_json::Value::Bool(on);
            changed.push(id.clone());
        }
    }
    Ok((to_tab_json(&json)?, changed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addons(details_installed: &str) -> String {
        format!(
            "{{\n\t\"a1\": {{\n\t\t\"id\": \"a1\",\n\t\t\"name\": \"Details\",\n\t\t\"autoUpdateEnabled\": false,\n\t\t\
             \"installedExternalReleaseId\": \"{details_installed}\",\n\t\t\"isIgnored\": false,\n\t\t\
             \"externalLatestReleaseId\": \"r2\",\n\t\t\"providerName\": \"Wago\",\n\t\t\"externalId\": \"e1\",\n\t\t\
             \"installationId\": \"i1\"\n\t}},\n\t\"a2\": {{\n\t\t\
             \"id\": \"a2\",\n\t\t\"name\": \"BugGrabber\",\n\t\t\"autoUpdateEnabled\": false,\n\t\t\
             \"installedExternalReleaseId\": \"x1\",\n\t\t\"isIgnored\": true,\n\t\t\"externalLatestReleaseId\": \"x2\",\n\t\t\
             \"providerName\": \"Wago\",\n\t\t\"externalId\": \"e2\",\n\t\t\"installationId\": \"i1\"\n\t}}\n}}"
        )
    }

    #[test]
    fn reads_addons_and_what_is_outdated() {
        let list = parse_addons(&addons("r1")).unwrap();
        assert_eq!(list.len(), 2);
        let outdated: Vec<&str> = list.iter().filter(|a| a.outdated()).map(|a| a.name.as_str()).collect();
        // The ignored one does not count.
        assert_eq!(outdated, ["Details"]);
    }

    #[test]
    fn finds_what_was_updated() {
        let before = parse_addons(&addons("r1")).unwrap();
        let after = parse_addons(&addons("r2")).unwrap();
        assert_eq!(updated(&before, &after), ["Details"]);
        assert!(!after.iter().any(Addon::outdated));
    }

    #[test]
    fn auto_update_is_switched_on_for_the_run_and_back() {
        let text = addons("r1");
        let (on, changed) = set_auto_update(&text, None, true).unwrap();
        // Ignored addons stay as they are.
        assert_eq!(changed, ["a1"]);
        assert_eq!(on, text.replacen("\"autoUpdateEnabled\": false", "\"autoUpdateEnabled\": true", 1));
        let (back, changed) = set_auto_update(&on, Some(&changed), false).unwrap();
        assert_eq!(changed, ["a1"]);
        assert_eq!(back, text);
    }

    #[test]
    fn unknown_addons_are_not_mixed_up() {
        // Addons it found on no site: provider "Unknown", no external id, ignored.
        let list = |v1: &str, v2: &str| {
            format!(
                r#"{{"u1":{{"name":"AltVault","providerName":"Unknown","externalId":"","installedVersion":"{v1}","latestVersion":"{v1}","isIgnored":true,"installationId":"i"}},
                    "u2":{{"name":"Paste","providerName":"Unknown","externalId":"","installedVersion":"{v2}","latestVersion":"{v2}","isIgnored":true,"installationId":"i"}}}}"#
            )
        };
        let (before, after) = (parse_addons(&list("4.0", "1.0")).unwrap(), parse_addons(&list("4.0", "1.0")).unwrap());
        assert!(updated(&before, &after).is_empty());
        assert!(!before[0].known);
        assert!(!before.iter().any(Addon::outdated));
    }

    #[test]
    fn attempted_updates_are_read_from_the_log() {
        let log = "[2026-10-12 22:17:26.967] [info]  [AddonUpdate] Wago rkGrrgGy BugSack 'v12.1.6' -> 'v12.1.7'\n\
                   [2026-10-12 22:17:26.968] [info]  [AddonUpdate] Curse 1234 Details! Damage Meter '1' -> '2'\n\
                   [2026-10-12 22:17:27.086] [info]  [download] response 401, content-length 0";
        assert_eq!(attempted(log), ["BugSack", "Details! Damage Meter"]);
    }

    #[test]
    fn installations_from_preferences() {
        let prefs = r#"{"wow_installations":[{"id":"i1","location":"C:\\WoW\\_retail_\\Wow.exe"}]}"#;
        assert_eq!(installations(prefs), [("i1".to_string(), r"C:\WoW\_retail_\Wow.exe".to_string())]);
    }
}

#[cfg(test)]
mod real_file {
    /// Reads (never writes) the real WowUp lists: a no-op pass must give them back byte for byte.
    /// `cargo test --release -- --ignored real_file`
    #[test]
    #[ignore]
    fn real_addons_files_round_trip() {
        for app in [super::WowUp::FREE, super::WowUp::CF] {
            let Ok(text) = std::fs::read_to_string(app.addons_file()) else { continue };
            let (same, changed) = super::set_auto_update(&text, Some(&[]), false).unwrap();
            assert!(changed.is_empty());
            assert!(same == text, "the real {} file would change", app.title);
        }
    }
}
