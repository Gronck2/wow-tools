//! Addon updates through the CurseForge app. Its API needs a per-developer key, so instead of
//! downloading ourselves the app is started in the tray (`--minimized`, the flag its own "start
//! with Windows" uses): with "install updates automatically" on, it checks and installs updates
//! by itself. Its log says when the check is done; the installed list
//! (`agent\GameInstances\AddonGameInstance.json`) before and after says what was updated.

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::updater::{Outcome, close, modified, quiet_command, running};

pub const EXE: &str = "CurseForge.exe";
/// Log lines that mean the app is still working on updates.
const BUSY_MARKS: &[&str] = &["Install", "Unzip", "Download"];
const CHECKED_MARK: &str = "Completed checking for updates in instance";
/// The app counts as done once nothing was installed for this long after the check.
const QUIET: Duration = Duration::from_secs(60);
const TIMEOUT: Duration = Duration::from_secs(10 * 60);
const POLL: Duration = Duration::from_secs(5);

fn app_data() -> PathBuf {
    std::env::var_os("APPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from).join("CurseForge")
}

fn instances_file() -> PathBuf {
    app_data().join(r"agent\GameInstances\AddonGameInstance.json")
}

fn logs_dir() -> PathBuf {
    app_data().join(r"agent\logs\CurseClient")
}

/// The installed CurseForge app (the standalone one).
pub fn exe() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    [r"Programs\CurseForge Windows", r"Programs\CurseForge"]
        .iter()
        .map(|d| local.join(d).join(EXE))
        .find(|p| p.is_file())
}

/// One WoW flavor CurseForge manages, with its addons: project id -> (name, installed file id).
#[derive(Debug, Default)]
pub struct Instance {
    pub path: PathBuf,
    pub name: String,
    pub auto_update: bool,
    pub addons: HashMap<u64, (String, u64)>,
    /// Addons whose newest file is not installed.
    pub outdated: Vec<String>,
    /// Addons set to "update manually" in the app; they are left as they are.
    pub manual: Vec<String>,
}

fn parse_instances(text: &str) -> Result<Vec<Instance>> {
    let json: serde_json::Value = serde_json::from_str(text).with_context(|| tr!("addons.list_unreadable", app = "CurseForge"))?;
    let list = json.as_array().map(Vec::as_slice).unwrap_or_default();
    Ok(list
        .iter()
        // gameTypeID 1 is World of Warcraft.
        .filter(|i| i["gameTypeID"].as_u64() == Some(1) && i["isEnabled"].as_bool() != Some(false))
        .map(|i| {
            let mut inst = Instance {
                path: PathBuf::from(i["installPath"].as_str().unwrap_or_default()),
                name: i["name"].as_str().unwrap_or_default().to_string(),
                auto_update: i["preferenceAutoInstallUpdates"].as_bool().unwrap_or(false),
                ..Default::default()
            };
            for a in i["installedAddons"].as_array().map(Vec::as_slice).unwrap_or_default() {
                let (Some(id), Some(file)) = (a["addonID"].as_u64(), a["installedFile"]["id"].as_u64()) else {
                    continue;
                };
                let name = a["name"].as_str().unwrap_or_default();
                // "BigWigs_Cataclysm - The Cataclysm content pack for BigWigs" -> "BigWigs_Cataclysm".
                let name = name.split(" - ").next().unwrap_or(name).trim().to_string();
                // null = the instance's choice; false = this addon is updated by hand.
                if a["preferenceAutoInstallUpdates"].as_bool() == Some(false) {
                    inst.manual.push(name.clone());
                }
                if a["latestFile"]["id"].as_u64().is_some_and(|latest| latest != file) {
                    inst.outdated.push(name.clone());
                }
                inst.addons.insert(id, (name, file));
            }
            inst
        })
        .collect())
}

pub fn instances() -> Result<Vec<Instance>> {
    let path = instances_file();
    let text = fs::read_to_string(&path).with_context(|| tr!("addons.not_set_up", app = "CurseForge", path = path.display()))?;
    parse_instances(&text)
}

/// Addons whose installed file changed between two reads, as "Name" per line.
fn updated(before: &[Instance], after: &[Instance]) -> Vec<String> {
    let mut out = Vec::new();
    for inst in after {
        let old = before.iter().find(|b| b.path == inst.path);
        for (id, (name, file)) in &inst.addons {
            if old.and_then(|o| o.addons.get(id)).is_some_and(|(_, f)| f != file) {
                out.push(name.clone());
            }
        }
    }
    out.sort();
    out
}

/// What the app logged since `t0`: whether the update check finished, and how many lines
/// about installing there are (it changes while updates are going in).
fn log_state(t0: SystemTime) -> (bool, usize) {
    let mut checked = false;
    let mut busy = 0;
    let Ok(entries) = fs::read_dir(logs_dir()) else { return (false, 0) };
    for e in entries.flatten() {
        let fresh = e.metadata().and_then(|m| m.modified()).is_ok_and(|m| m >= t0);
        if !fresh {
            continue;
        }
        let Ok(text) = fs::read_to_string(e.path()) else { continue };
        checked |= text.contains(CHECKED_MARK);
        busy += text.lines().filter(|l| BUSY_MARKS.iter().any(|m| l.contains(m))).count();
    }
    (checked, busy)
}

/// One line per WoW install CurseForge manages, for `status`.
pub fn describe() -> Result<Vec<String>> {
    Ok(instances()?
        .into_iter()
        .map(|i| {
            let auto = if i.auto_update {
                tr!("curseforge.auto_on")
            } else {
                tr!("curseforge.auto_off")
            };
            let fresh = match i.outdated.len() {
                0 => tr!("addons.all_fresh").to_string(),
                n => tr!("addons.waiting", n = n, names = i.outdated.join(", ")),
            };
            tr!(
                "curseforge.describe",
                name = i.name,
                path = i.path.display(),
                n = i.addons.len(),
                auto = auto,
                fresh = fresh
            )
        })
        .collect())
}

/// Starts CurseForge in the tray, waits until it has checked and installed updates, closes it.
/// When it is already running it updates by itself and is left alone.
pub fn update(exe: &Path) -> Result<Outcome> {
    let before = instances()?;
    if before.is_empty() {
        bail!(tr!("curseforge.no_instances"));
    }
    if running(EXE) {
        let note = if before.iter().all(|i| i.auto_update) {
            tr!("curseforge.open_auto")
        } else {
            tr!("curseforge.open_manual")
        };
        return Ok(Outcome::note(note));
    }

    // With "install updates automatically" off the app only checks; switch it on for this run.
    restore_auto_update();
    let forced = force_auto_update()?;
    if !forced.is_empty() {
        log::info!("{}", tr!("addons.forced_on", app = "CurseForge"));
    }
    let result = run_app(exe, before);
    restore_auto_update();
    result
}

fn run_app(exe: &Path, before: Vec<Instance>) -> Result<Outcome> {
    log::info!("{}", tr!("curseforge.starting"));
    let t0 = SystemTime::now();
    let started = Instant::now();
    quiet_command(exe)
        .arg("--minimized")
        .spawn()
        .with_context(|| tr!("addons.start_failed", path = exe.display()))?;

    let mut last = (log_state(t0).1, modified(&instances_file()));
    let mut quiet_since = Instant::now();
    let mut timed_out = true;
    while started.elapsed() < TIMEOUT {
        std::thread::sleep(POLL);
        let (checked, busy) = log_state(t0);
        let now = (busy, modified(&instances_file()));
        if now != last {
            last = now;
            quiet_since = Instant::now();
        }
        if checked && quiet_since.elapsed() >= QUIET {
            timed_out = false;
            break;
        }
    }
    close(EXE);
    log::info!("{}", tr!("addons.closed_after", app = "CurseForge", secs = started.elapsed().as_secs()));

    let after = instances()?;
    let mut note = Vec::new();
    if timed_out {
        note.push(tr!("addons.timeout", app = "CurseForge", min = TIMEOUT.as_secs() / 60));
    }
    let manual: Vec<String> = after.iter().flat_map(|i| i.manual.iter().cloned()).collect();
    if !manual.is_empty() {
        note.push(tr!("curseforge.manual", names = manual.join(", ")));
    }
    Ok(Outcome {
        updated: updated(&before, &after),
        outdated: after.iter().flat_map(|i| i.outdated.iter().cloned()).collect(),
        note: (!note.is_empty()).then(|| note.join("\n")),
    })
}

// ---------------------------------------------------------------- the auto-update switch

const AUTO_KEY: &str = "preferenceAutoInstallUpdates";

/// The WoW instances (by guid) whose "install updates automatically" we switched on and have
/// to switch off again; kept on disk so a run cut short is undone next time.
fn forced_file() -> PathBuf {
    crate::config::data_dir().join("curseforge-forced.json")
}

/// Sets the instance-level auto-update of `guids` (or of every WoW instance that has it off,
/// when `guids` is `None`) to `on`; returns the guids changed. Only the app's own JSON is
/// touched, with key order and numbers kept as they are (`preserve_order`, `arbitrary_precision`).
fn set_auto_update(text: &str, guids: Option<&[String]>, on: bool) -> Result<(String, Vec<String>)> {
    let mut json: serde_json::Value = serde_json::from_str(text).with_context(|| tr!("addons.list_unreadable", app = "CurseForge"))?;
    let mut changed = Vec::new();
    for inst in json.as_array_mut().into_iter().flatten() {
        let guid = inst["guid"].as_str().unwrap_or_default().to_string();
        let wanted = match guids {
            Some(list) => list.contains(&guid),
            None => inst["gameTypeID"].as_u64() == Some(1) && inst[AUTO_KEY].as_bool() == Some(false),
        };
        if wanted && inst[AUTO_KEY].as_bool() != Some(on) {
            inst[AUTO_KEY] = serde_json::Value::Bool(on);
            changed.push(guid);
        }
    }
    Ok((serde_json::to_string(&json)?, changed))
}

fn force_auto_update() -> Result<Vec<String>> {
    let path = instances_file();
    let text = fs::read_to_string(&path)?;
    let (new_text, changed) = set_auto_update(&text, None, true)?;
    if changed.is_empty() {
        return Ok(changed);
    }
    // The file as it was, in case anything goes wrong.
    fs::copy(&path, path.with_extension("json.wow-tools.bak"))?;
    fs::create_dir_all(crate::config::data_dir())?;
    fs::write(forced_file(), serde_json::to_string(&changed)?)?;
    fs::write(&path, new_text)?;
    Ok(changed)
}

/// Switches auto-update off again where we switched it on (the app must be closed).
fn restore_auto_update() {
    let Ok(text) = fs::read_to_string(forced_file()) else { return };
    let guids: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
    let path = instances_file();
    let result = fs::read_to_string(&path)
        .map_err(anyhow::Error::from)
        .and_then(|t| set_auto_update(&t, Some(&guids), false))
        .and_then(|(t, _)| Ok(fs::write(&path, t)?));
    match result {
        Ok(()) => {
            let _ = fs::remove_file(forced_file());
            log::info!("{}", tr!("addons.auto_restored", app = "CurseForge"));
        }
        Err(e) => log::warn!("{}", tr!("addons.auto_restore_failed", app = "CurseForge", error = format!("{e:#}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(file_bigwigs: u64, latest_details: u64) -> String {
        format!(
            r#"[{{"gameTypeID":1,"isEnabled":true,"name":"Retail","installPath":"C:\\WoW\\_retail_\\",
                "preferenceAutoInstallUpdates":true,"installedAddons":[
                {{"addonID":1,"name":"BigWigs - Boss mods","installedFile":{{"id":{file_bigwigs}}},"latestFile":{{"id":{file_bigwigs}}}}},
                {{"addonID":2,"name":"Details! Damage Meter","installedFile":{{"id":100}},"latestFile":{{"id":{latest_details}}}}}]}},
               {{"gameTypeID":432,"isEnabled":true,"name":"Minecraft","installPath":"C:\\mc","installedAddons":[]}}]"#
        )
    }

    #[test]
    fn reads_wow_instances_only() {
        let list = parse_instances(&instance(10, 100)).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Retail");
        assert!(list[0].auto_update);
        assert_eq!(list[0].addons[&1].0, "BigWigs");
        assert!(list[0].outdated.is_empty());
        let stale = parse_instances(&instance(10, 101)).unwrap();
        assert_eq!(stale[0].outdated, ["Details! Damage Meter"]);
    }

    #[test]
    fn finds_what_was_updated() {
        let before = parse_instances(&instance(10, 100)).unwrap();
        let after = parse_instances(&instance(11, 100)).unwrap();
        assert_eq!(updated(&before, &after), ["BigWigs"]);
        assert!(updated(&before, &before).is_empty());
    }

    #[test]
    fn auto_update_is_switched_on_for_the_run_and_back() {
        // Compact like the app writes it; big numbers and dates must survive untouched.
        let text = concat!(
            r#"[{"guid":"a","gameTypeID":1,"isEnabled":true,"lastRefreshAttempt":"2026-10-11T16:15:12.1861256+04:00","#,
            r#""preferenceAutoInstallUpdates":false,"installedAddons":[{"addonID":45694,"fileLength":123456789012,"#,
            r#""preferenceAutoInstallUpdates":null,"latestFile":{"id":8961570}}]},"#,
            r#"{"guid":"b","gameTypeID":1,"preferenceAutoInstallUpdates":true,"installedAddons":[]},"#,
            r#"{"guid":"mc","gameTypeID":432,"preferenceAutoInstallUpdates":false,"installedAddons":[]}]"#
        );
        let (on, changed) = set_auto_update(text, None, true).unwrap();
        assert_eq!(changed, ["a"]);
        assert_eq!(
            on,
            text.replacen(r#""preferenceAutoInstallUpdates":false"#, r#""preferenceAutoInstallUpdates":true"#, 1)
        );
        assert!(parse_instances(&on).unwrap()[0].auto_update);
        // Back: only "a", the per-addon null and Minecraft stay as they were.
        let (back, changed) = set_auto_update(&on, Some(&changed), false).unwrap();
        assert_eq!(changed, ["a"]);
        assert_eq!(back, text);
    }

    #[test]
    fn manual_addons_are_listed() {
        let text = r#"[{"gameTypeID":1,"name":"Retail","installedAddons":[
            {"addonID":1,"name":"WeakAuras","preferenceAutoInstallUpdates":false,"installedFile":{"id":1},"latestFile":{"id":2}}]}]"#;
        let list = parse_instances(text).unwrap();
        assert_eq!(list[0].manual, ["WeakAuras"]);
        assert_eq!(list[0].outdated, ["WeakAuras"]);
    }
}

#[cfg(test)]
mod real_file {
    /// Reads (never writes) the real CurseForge list: a no-op pass must give it back byte for byte.
    /// `cargo test --release -- --ignored real_file`
    #[test]
    #[ignore]
    fn real_instances_file_round_trips() {
        let text = std::fs::read_to_string(super::instances_file()).expect("CurseForge list");
        let (same, changed) = super::set_auto_update(&text, Some(&[]), false).unwrap();
        assert!(changed.is_empty());
        assert!(same == text, "the real file would change");
    }
}
