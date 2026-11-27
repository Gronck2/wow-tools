//! The last macro check on disk (`data\macro-report.json`), so the report and the panel's Macros tab show it again
//! after the tray or Windows restarted. Only this app's own data folder is written, never WoW's. A new check
//! overwrites the file; a file that is broken or of another `version` is ignored (logged), as if there were none.

use crate::macro_fix::MacroFix;
use crate::macros::Scan;
use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Bumped when the stored shape changes; a file of another version is not read.
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    at: DateTime<Local>,
    scan: Scan,
    /// The fixed texts once each, and per finding the one it shows (the findings of a macro share it).
    fixes: Vec<MacroFix>,
    links: Vec<Option<usize>>,
}

pub fn path() -> PathBuf {
    crate::config::data_dir().join("macro-report.json")
}

fn encode(scan: &Scan, at: DateTime<Local>) -> Result<String> {
    let mut seen: Vec<&Arc<MacroFix>> = Vec::new();
    let mut links = Vec::new();
    for f in &scan.findings {
        links.push(f.fixed.as_ref().map(|fix| match seen.iter().position(|s| Arc::ptr_eq(s, fix)) {
            Some(i) => i,
            None => {
                seen.push(fix);
                seen.len() - 1
            }
        }));
    }
    let stored = Stored {
        version: VERSION,
        at,
        scan: scan.clone(),
        fixes: seen.into_iter().map(|f| (**f).clone()).collect(),
        links,
    };
    Ok(serde_json::to_string(&stored)?)
}

fn decode(text: &str) -> Result<(Scan, DateTime<Local>)> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    let version = value["version"].as_u64();
    if version != Some(u64::from(VERSION)) {
        anyhow::bail!("format version {version:?}, expected {VERSION}");
    }
    let Stored {
        at, mut scan, fixes, links, ..
    } = serde_json::from_value(value)?;
    anyhow::ensure!(links.len() == scan.findings.len(), "findings and links differ");
    let fixes: Vec<Arc<MacroFix>> = fixes.into_iter().map(Arc::new).collect();
    for (f, link) in scan.findings.iter_mut().zip(links) {
        f.fixed = match link {
            Some(i) => Some(fixes.get(i).context("a link to a missing fixed text")?.clone()),
            None => None,
        };
    }
    Ok((scan, at))
}

/// Writes the check through a temp file, so a cut write never leaves half a report.
pub fn save_to(file: &Path, scan: &Scan, at: DateTime<Local>) -> Result<()> {
    let tmp = file.with_extension("json.tmp");
    std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")))?;
    std::fs::write(&tmp, encode(scan, at)?)?;
    std::fs::rename(&tmp, file)?;
    Ok(())
}

pub fn save(scan: &Scan, at: DateTime<Local>) {
    if let Err(e) = save_to(&path(), scan, at) {
        log::warn!("{}: {e:#}", path().display());
    }
}

/// The saved check and when it was made; none when there is none or it cannot be read.
pub fn load_from(file: &Path) -> Option<(Scan, DateTime<Local>)> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            log::warn!("{}: {e}", file.display());
            return None;
        }
    };
    match decode(&text) {
        Ok(saved) => Some(saved),
        Err(e) => {
            log::warn!("{}: ignored: {e:#}", file.display());
            None
        }
    }
}

pub fn load() -> Option<(Scan, DateTime<Local>)> {
    load_from(&path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macro_fix::Variant;
    use crate::macros::{Finding, How, Kind};

    fn finding(name: &str, kind: Kind, fixed: Option<Arc<MacroFix>>) -> Finding {
        Finding {
            flavor: "Retail".into(),
            scope: "Acc".into(),
            account: true,
            name: name.into(),
            line: 2,
            kind,
            owner: Default::default(),
            fixed,
        }
    }

    fn sample() -> Scan {
        let fix = Arc::new(MacroFix {
            variants: vec![Variant {
                lang: "enUS".into(),
                text: "/cast Огонь\n/run print((C_Spell.GetSpellInfo(1) or {}).name)".into(),
                chars: 55,
                over: false,
                rest: 1,
                marks: vec![0..3, 10..12],
                check: true,
            }],
        });
        Scan {
            findings: vec![
                finding(
                    "A",
                    Kind::Deprecated {
                        func: "GetSpellInfo".into(),
                        replacement: "C_Spell.GetSpellInfo".into(),
                        patch: "12.0.0".into(),
                        url: "https://example.com/x".into(),
                    },
                    Some(fix.clone()),
                ),
                finding(
                    "A",
                    Kind::WrongLanguage {
                        name: "Feuerball".into(),
                        lang: "deDE".into(),
                        fix: "Fireball".into(),
                        how: How::Client,
                    },
                    Some(fix),
                ),
                finding("B", Kind::Brackets, None),
            ],
            checked: 9,
            missing: vec!["Retail".into()],
            ..Default::default()
        }
    }

    fn at() -> DateTime<Local> {
        "2026-11-24T14:05:00+03:00"
            .parse::<DateTime<chrono::FixedOffset>>()
            .unwrap()
            .with_timezone(&Local)
    }

    #[test]
    fn a_saved_check_comes_back_the_same_with_the_texts_shared() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("macro-report.json");
        assert!(load_from(&file).is_none());
        let scan = sample();
        save_to(&file, &scan, at()).unwrap();
        assert!(!file.with_extension("json.tmp").exists());
        let (back, when) = load_from(&file).unwrap();
        assert_eq!(when, at());
        assert_eq!((back.checked, &back.missing), (9, &scan.missing));
        assert_eq!(format!("{:?}", back.findings.iter().map(|f| (&f.kind, &f.name, f.line)).collect::<Vec<_>>()), {
            format!("{:?}", scan.findings.iter().map(|f| (&f.kind, &f.name, f.line)).collect::<Vec<_>>())
        });
        let fixes: Vec<_> = back.findings.iter().map(|f| f.fixed.clone()).collect();
        assert_eq!(fixes[0].as_deref(), scan.findings[0].fixed.as_deref());
        // the two findings of one macro share one text again, so the report shows it once
        assert!(Arc::ptr_eq(fixes[0].as_ref().unwrap(), fixes[1].as_ref().unwrap()) && fixes[2].is_none());
        assert_eq!(crate::macros::first_fixes(&back.findings).iter().filter(|f| f.is_some()).count(), 1);
        assert_eq!(crate::macros::report(&back), crate::macros::report(&scan));
    }

    #[test]
    fn a_broken_or_old_file_is_no_report_and_a_new_check_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("macro-report.json");
        std::fs::write(&file, "{ not json").unwrap();
        assert!(load_from(&file).is_none());
        std::fs::write(&file, "").unwrap();
        assert!(load_from(&file).is_none());
        // another format version
        let old = encode(&sample(), at()).unwrap().replace("\"version\":1", "\"version\":0");
        std::fs::write(&file, old).unwrap();
        assert!(load_from(&file).is_none());
        let future = encode(&sample(), at()).unwrap().replace("\"version\":1", "\"version\":2");
        std::fs::write(&file, future).unwrap();
        assert!(load_from(&file).is_none());
        // a link to a text that is not there
        let bad = encode(&sample(), at()).unwrap().replace("\"links\":[0,0,null]", "\"links\":[5,0,null]");
        std::fs::write(&file, bad).unwrap();
        assert!(load_from(&file).is_none());
        // a new check replaces the old one
        save_to(&file, &sample(), at()).unwrap();
        let newer = Scan {
            checked: 3,
            ..Default::default()
        };
        save_to(&file, &newer, at()).unwrap();
        let (back, _) = load_from(&file).unwrap();
        assert_eq!((back.checked, back.findings.len()), (3, 0));
    }
}
