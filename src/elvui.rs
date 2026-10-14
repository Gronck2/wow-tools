//! ElvUI updates from its official site. ElvUI is not on CurseForge or Wago, so the addon
//! managers leave it alone. `api.tukui.org/v1/addon/elvui` gives the current version and a link
//! to one zip for every client (`ElvUI`, `ElvUI_Libraries`, `ElvUI_Options`). Those folders are
//! replaced whole, so files a new version dropped do not linger: a leftover `ElvUI_Mainline.toc`
//! of an older version would be read by the game in place of `ElvUI.toc`.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::wow::Flavor;

const API: &str = "https://api.tukui.org/v1/addon/elvui";

/// The folder whose `.toc` carries the version.
const MAIN: &str = "ElvUI";

/// Next to `AddOns`, on the same drive so the folders can be renamed in.
const STAGING: &str = ".wow-tools-elvui";

#[derive(Debug, Deserialize)]
struct Latest {
    version: String,
    url: String,
}

fn parse_latest(body: &[u8]) -> Result<Latest> {
    let latest: Latest = serde_json::from_slice(body).with_context(|| tr!("elvui.bad_answer"))?;
    if latest.version.trim().is_empty() || !latest.url.starts_with("https://") {
        bail!(tr!("elvui.bad_answer"));
    }
    Ok(latest)
}

fn fetch_latest() -> Result<Latest> {
    let (status, body) = crate::selfupdate::http::get(API, Some("application/json"))?;
    if status != 200 {
        bail!(tr!("elvui.http", status = status));
    }
    parse_latest(&body)
}

/// The `## Version:` of every `.toc` of the ElvUI folder, `ElvUI.toc` first; empty = not
/// installed. More than one version means files of an older one were left behind.
fn installed(addons: &Path) -> Vec<String> {
    let Ok(dir) = fs::read_dir(addons.join(MAIN)) else {
        return Vec::new();
    };
    let mut tocs: Vec<PathBuf> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toc")))
        .collect();
    tocs.sort_by_key(|p| (p.file_stem().is_none_or(|s| !s.eq_ignore_ascii_case(MAIN)), p.clone()));
    let mut versions: Vec<String> = Vec::new();
    for toc in tocs {
        let text = fs::read(&toc).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let version = text.lines().find_map(|l| {
            let (key, value) = l.trim_start_matches(['#', ' ']).split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case("Version")
                .then(|| value.trim().trim_start_matches(['v', 'V']).to_string())
        });
        if let Some(v) = version
            && !versions.contains(&v)
        {
            versions.push(v);
        }
    }
    versions
}

/// Updates ElvUI in every flavor that has it, when tukui.org has a newer version or the install
/// mixes versions. Flavors without ElvUI are left alone. Returns the history text: "Обновлён:
/// 15.26 → 16.00" (the oldest version replaced), or "reinstalled clean" when only a mixed install
/// was redone.
pub fn update(flavors: &[Flavor]) -> Result<Option<String>> {
    let targets: Vec<(&Flavor, PathBuf, Vec<String>)> = flavors
        .iter()
        .map(|fl| {
            let addons = fl.dir.join("Interface").join("AddOns");
            let versions = installed(&addons);
            (fl, addons, versions)
        })
        .filter(|(_, _, v)| !v.is_empty())
        .collect();
    if targets.is_empty() {
        log::info!("{}", tr!("elvui.not_installed"));
        return Ok(None);
    }
    let latest = fetch_latest()?;
    let mut zip: Option<Vec<u8>> = None;
    let mut oldest: Option<String> = None;
    for (fl, addons, versions) in targets {
        let current = &versions[0];
        let mixed = versions.len() > 1;
        if !mixed && !crate::selfupdate::newer(&latest.version, current) {
            log::info!("{}", tr!("elvui.fresh", version = current, flavor = fl.title()));
            continue;
        }
        if zip.is_none() {
            let (status, bytes) = crate::selfupdate::http::get(&latest.url, None)?;
            if status != 200 {
                bail!(tr!("elvui.http", status = status));
            }
            zip = Some(bytes);
        }
        install(zip.as_deref().unwrap_or_default(), &addons)?;
        let line = if mixed {
            tr!("elvui.repaired", versions = versions.join(", "), to = &latest.version, flavor = fl.title())
        } else {
            tr!("elvui.updated", from = current, to = &latest.version, flavor = fl.title())
        };
        log::info!("{line}");
        for v in &versions {
            if oldest.as_ref().is_none_or(|o| crate::selfupdate::newer(o, v)) {
                oldest = Some(v.clone());
            }
        }
    }
    Ok(oldest.map(|from| {
        if from == latest.version {
            tr!("elvui.history_redone", version = &latest.version)
        } else {
            tr!("elvui.history_updated", from = from, to = &latest.version)
        }
    }))
}

/// Unpacks the ElvUI zip next to `addons` and swaps its folders in, putting the old ones back if
/// a swap fails.
fn install(zip: &[u8], addons: &Path) -> Result<()> {
    let interface = addons.parent().context("AddOns")?;
    let staging = interface.join(STAGING);
    recover(&staging, addons);
    let new = staging.join("new");
    let old = staging.join("old");
    fs::create_dir_all(&new)?;
    fs::create_dir_all(&old)?;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| anyhow::anyhow!(tr!("elvui.bad_zip", error = e)))?;
    archive.extract(&new).map_err(|e| anyhow::anyhow!(tr!("elvui.bad_zip", error = e)))?;
    let mut folders = Vec::new();
    for entry in fs::read_dir(&new)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Only ElvUI's own folders: a zip that would put anything else into AddOns is not the one we want.
        if !entry.file_type()?.is_dir() || !name.starts_with(MAIN) {
            bail!(tr!("elvui.bad_zip", error = name));
        }
        folders.push(name);
    }
    if installed(&new).is_empty() {
        bail!(tr!("elvui.bad_zip", error = format!("{MAIN}/{MAIN}.toc")));
    }
    let mut moved: Vec<&str> = Vec::new();
    let mut swapped: Vec<&str> = Vec::new();
    let result = (|| -> Result<()> {
        for name in &folders {
            let target = addons.join(name);
            if target.exists() {
                fs::rename(&target, old.join(name)).with_context(|| tr!("elvui.swap_failed", path = target.display()))?;
                moved.push(name);
            }
            fs::rename(new.join(name), &target).with_context(|| tr!("elvui.swap_failed", path = target.display()))?;
            swapped.push(name);
        }
        Ok(())
    })();
    if let Err(e) = result {
        for name in swapped {
            let _ = fs::remove_dir_all(addons.join(name));
        }
        for name in moved {
            let _ = fs::rename(old.join(name), addons.join(name));
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    fs::remove_dir_all(&staging).with_context(|| tr!("error.write", path = staging.display()))?;
    Ok(())
}

/// After a run cut short: puts back old folders that are missing from AddOns, then drops the rest.
fn recover(staging: &Path, addons: &Path) {
    if let Ok(dir) = fs::read_dir(staging.join("old")) {
        for entry in dir.flatten() {
            let target = addons.join(entry.file_name());
            if !target.exists() {
                let _ = fs::rename(entry.path(), target);
            }
        }
    }
    let _ = fs::remove_dir_all(staging);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn toc(dir: &Path, name: &str, version: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), format!("## Interface: 120100\n## Title: ElvUI\n## Version: v{version}\n")).unwrap();
    }

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut out);
        for (name, text) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(text.as_bytes()).unwrap();
        }
        w.finish().unwrap();
        out.into_inner()
    }

    #[test]
    fn answer_is_read() {
        let body = br#"{"id":-2,"slug":"elvui","url":"https://api.tukui.org/v1/download/elvui/x","version":"16.00","patch":["12.1.0"]}"#;
        let l = parse_latest(body).unwrap();
        assert_eq!(l.version, "16.00");
        assert!(l.url.ends_with("/x"));
        assert!(parse_latest(br#"{"version":"1","url":"http://x"}"#).is_err());
        assert!(parse_latest(b"<html>").is_err());
    }

    #[test]
    fn versions_of_an_install() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path();
        assert!(installed(addons).is_empty());
        let elv = addons.join("ElvUI");
        toc(&elv, "ElvUI_Mainline.toc", "15.26");
        toc(&elv, "ElvUI.toc", "16.00");
        toc(&elv, "ElvUI_Mists.toc", "15.26");
        assert_eq!(installed(addons), ["16.00", "15.26"]);
    }

    #[test]
    fn install_replaces_the_folders_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("Interface").join("AddOns");
        toc(&addons.join("ElvUI"), "ElvUI.toc", "15.26");
        toc(&addons.join("ElvUI"), "ElvUI_Mainline.toc", "15.26");
        toc(&addons.join("ElvUI_WindTools"), "ElvUI_WindTools.toc", "4.0");
        let zip = zip_of(&[
            ("ElvUI/ElvUI.toc", "## Version: v16.00\n"),
            ("ElvUI/Core/init.lua", "-- new"),
            ("ElvUI_Options/ElvUI_Options.toc", "## Version: v16.00\n"),
        ]);
        install(&zip, &addons).unwrap();
        assert_eq!(installed(&addons), ["16.00"]);
        assert!(!addons.join("ElvUI/ElvUI_Mainline.toc").exists());
        assert!(addons.join("ElvUI/Core/init.lua").exists());
        assert!(addons.join("ElvUI_Options/ElvUI_Options.toc").exists());
        // A plugin is not part of the zip and stays.
        assert!(addons.join("ElvUI_WindTools/ElvUI_WindTools.toc").exists());
        assert!(!tmp.path().join("Interface").join(STAGING).exists());
    }

    #[test]
    fn a_foreign_zip_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("Interface").join("AddOns");
        toc(&addons.join("ElvUI"), "ElvUI.toc", "15.26");
        let zip = zip_of(&[("ElvUI/ElvUI.toc", "## Version: v16.00\n"), ("Other/Other.toc", "")]);
        assert!(install(&zip, &addons).is_err());
        assert!(install(b"not a zip", &addons).is_err());
        assert_eq!(installed(&addons), ["15.26"]);
        assert!(!addons.join("Other").exists());
    }

    #[test]
    fn a_cut_short_run_is_undone() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("Interface").join("AddOns");
        let staging = tmp.path().join("Interface").join(STAGING);
        // The old folder was moved aside, the new one never came in.
        toc(&staging.join("old").join("ElvUI"), "ElvUI.toc", "15.26");
        toc(&addons.join("ElvUI_Options"), "ElvUI_Options.toc", "16.00");
        toc(&staging.join("old").join("ElvUI_Options"), "ElvUI_Options.toc", "15.26");
        recover(&staging, &addons);
        assert_eq!(installed(&addons), ["15.26"]);
        assert!(fs::read_to_string(addons.join("ElvUI_Options/ElvUI_Options.toc")).unwrap().contains("16.00"));
        assert!(!staging.exists());
    }

    /// The real site: an old install in a temp folder gets the current version.
    #[test]
    #[ignore]
    fn live_tukui() {
        let tmp = tempfile::tempdir().unwrap();
        let fl = Flavor::new("_retail_", tmp.path().join("_retail_"));
        let addons = fl.dir.join("Interface").join("AddOns");
        toc(&addons.join("ElvUI"), "ElvUI.toc", "1.0");
        toc(&addons.join("ElvUI"), "ElvUI_Mainline.toc", "1.0");
        let name = update(&[fl]).unwrap().unwrap();
        let versions = installed(&addons);
        assert_eq!(versions.len(), 1);
        assert_eq!(name, format!("Обновлён: 1.0 → {}", versions[0]));
        assert!(addons.join("ElvUI_Libraries").exists() && addons.join("ElvUI_Options").exists());
        assert!(!addons.join("ElvUI/ElvUI_Mainline.toc").exists());
    }
}
