//! Characters in `WTF/Account/<ACCOUNT>/<Realm>/<Character>` and copying
//! settings from one to another.

use anyhow::{Result, bail};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Character {
    pub account: String,
    pub realm: String,
    pub name: String,
    pub dir: PathBuf,
}

impl fmt::Display for Character {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.account, self.realm, self.name)
    }
}

/// What can be copied between characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Bindings,
    Macros,
    Chat,
    Config,
    Layout,
    Addons,
    Savedvars,
}

/// By hand, so `--help` describes the parts in the interface language.
impl clap::ValueEnum for Part {
    fn value_variants<'a>() -> &'a [Self] {
        &Part::ALL
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        let (name, help) = match self {
            Part::Bindings => ("bindings", tr!("cli_part.bindings")),
            Part::Macros => ("macros", tr!("cli_part.macros")),
            Part::Chat => ("chat", tr!("cli_part.chat")),
            Part::Config => ("config", tr!("cli_part.config")),
            Part::Layout => ("layout", tr!("cli_part.layout")),
            Part::Addons => ("addons", tr!("cli_part.addons")),
            Part::Savedvars => ("savedvars", tr!("cli_part.savedvars")),
        };
        Some(clap::builder::PossibleValue::new(name).help(help))
    }
}

impl Part {
    pub const ALL: [Part; 7] = [
        Part::Bindings,
        Part::Macros,
        Part::Chat,
        Part::Config,
        Part::Layout,
        Part::Addons,
        Part::Savedvars,
    ];

    fn file(self) -> Option<&'static str> {
        match self {
            Part::Bindings => Some("bindings-cache.wtf"),
            Part::Macros => Some("macros-cache.txt"),
            Part::Chat => Some("chat-cache.txt"),
            Part::Config => Some("config-cache.wtf"),
            Part::Layout => Some("layout-local.txt"),
            Part::Addons => Some("AddOns.txt"),
            Part::Savedvars => None,
        }
    }
}

fn subdirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<_> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(name, _)| name != "SavedVariables")
        .collect();
    out.sort();
    out
}

pub fn list(wtf: &Path) -> Vec<Character> {
    let mut out = Vec::new();
    for (account, acc_dir) in subdirs(&wtf.join("Account")) {
        for (realm, realm_dir) in subdirs(&acc_dir) {
            for (name, dir) in subdirs(&realm_dir) {
                out.push(Character {
                    account: account.clone(),
                    realm: realm.clone(),
                    name,
                    dir,
                });
            }
        }
    }
    out
}

/// Finds a character by `Realm/Name` or `ACCOUNT/Realm/Name`, case-insensitive.
pub fn find<'a>(chars: &'a [Character], spec: &str) -> Result<&'a Character> {
    let parts: Vec<&str> = spec.split(['/', '\\']).collect();
    let (account, realm, name) = match parts.as_slice() {
        [r, n] => (None, *r, *n),
        [a, r, n] => (Some(*a), *r, *n),
        _ => bail!(tr!("chars.bad_spec", spec = spec)),
    };
    // Realm and character names are often Cyrillic, so ASCII-only folding is not enough.
    let same = |a: &str, b: &str| a.to_lowercase() == b.to_lowercase();
    let matches: Vec<&Character> = chars
        .iter()
        .filter(|c| same(&c.realm, realm) && same(&c.name, name) && account.is_none_or(|a| same(&c.account, a)))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => bail!(tr!("chars.not_found", spec = spec)),
        many => {
            let names: Vec<String> = many.iter().map(|c| c.to_string()).collect();
            bail!(tr!("chars.ambiguous", spec = spec, names = names.join(", ")))
        }
    }
}

/// Copies the selected parts. Returns the files copied (or, with
/// `dry_run`, the files that would be).
pub fn copy(from: &Character, to: &Character, parts: &[Part], dry_run: bool) -> Result<Vec<String>> {
    if from.dir == to.dir {
        bail!(tr!("chars.same", name = from));
    }
    let mut actions = Vec::new();
    let mut copy_file = |src: PathBuf, dst: PathBuf| -> Result<()> {
        let rel = src.strip_prefix(&from.dir).unwrap_or(&src).display().to_string();
        if !dry_run {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src, &dst)?;
        }
        actions.push(rel);
        Ok(())
    };

    for part in parts {
        match part.file() {
            Some(file) => {
                let src = from.dir.join(file);
                if src.is_file() {
                    copy_file(src, to.dir.join(file))?;
                }
            }
            None => {
                let src_dir = from.dir.join("SavedVariables");
                let Ok(entries) = fs::read_dir(&src_dir) else { continue };
                let mut files: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lua")))
                    .collect();
                files.sort();
                for src in files {
                    let dst = to.dir.join("SavedVariables").join(src.file_name().unwrap());
                    copy_file(src, dst)?;
                }
            }
        }
    }
    Ok(actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_char(wtf: &Path, acc: &str, realm: &str, name: &str) -> PathBuf {
        let dir = wtf.join("Account").join(acc).join(realm).join(name);
        fs::create_dir_all(dir.join("SavedVariables")).unwrap();
        dir
    }

    #[test]
    fn list_find_and_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let wtf = tmp.path();
        fs::create_dir_all(wtf.join("Account/ACC1/SavedVariables")).unwrap();
        let main = make_char(wtf, "ACC1", "Gordunni", "Main");
        let alt = make_char(wtf, "ACC1", "Gordunni", "Alt");
        make_char(wtf, "ACC2", "Gordunni", "Main");

        fs::write(main.join("bindings-cache.wtf"), "BIND").unwrap();
        fs::write(main.join("SavedVariables/Details.lua"), "x").unwrap();
        fs::write(main.join("SavedVariables/Details.lua.bak"), "old").unwrap();

        let chars = list(wtf);
        assert_eq!(chars.len(), 3);
        assert!(find(&chars, "gordunni/main").is_err(), "двое Main на разных аккаунтах");
        let from = find(&chars, "ACC1/Gordunni/Main").unwrap();
        let to = find(&chars, "Gordunni/Alt").unwrap();

        let planned = copy(from, to, &Part::ALL, true).unwrap();
        assert_eq!(planned.len(), 2);
        assert!(!alt.join("bindings-cache.wtf").exists(), "dry-run ничего не пишет");

        copy(from, to, &Part::ALL, false).unwrap();
        assert_eq!(fs::read_to_string(alt.join("bindings-cache.wtf")).unwrap(), "BIND");
        assert!(alt.join("SavedVariables/Details.lua").exists());
        assert!(!alt.join("SavedVariables/Details.lua.bak").exists());
    }

    #[test]
    fn find_cyrillic_any_case() {
        let tmp = tempfile::tempdir().unwrap();
        make_char(tmp.path(), "123#1", "Азурегос", "Гронки");
        let chars = list(tmp.path());
        assert_eq!(find(&chars, "азурегос/ГРОНКИ").unwrap().name, "Гронки");
    }
}
