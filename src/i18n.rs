//! Interface languages. Texts live in `locales/<code>.toml` (embedded at build time) under
//! `[section]` tables: `tr!("panel.backup_now")`, or with arguments `tr!("panel.last_backup",
//! time = t)` for a text holding `{time}`. A key missing in a language falls back to English,
//! then to the key itself. Tests check that every language has every key with the same arguments
//! and that every key the code uses exists.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Ru,
    En,
    De,
    Fr,
    Es,
}

impl Lang {
    pub const ALL: [Lang; 5] = [Lang::Ru, Lang::En, Lang::De, Lang::Fr, Lang::Es];

    /// The language's name in itself, for the language picker.
    pub fn native(self) -> &'static str {
        match self {
            Lang::Ru => "Русский",
            Lang::En => "English",
            Lang::De => "Deutsch",
            Lang::Fr => "Français",
            Lang::Es => "Español",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Lang::Ru => include_str!("../locales/ru.toml"),
            Lang::En => include_str!("../locales/en.toml"),
            Lang::De => include_str!("../locales/de.toml"),
            Lang::Fr => include_str!("../locales/fr.toml"),
            Lang::Es => include_str!("../locales/es.toml"),
        }
    }

    /// The Windows display language, when it is one of ours; English otherwise.
    pub fn system() -> Lang {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
            // The primary language is the low 10 bits of the LANGID.
            match unsafe { GetUserDefaultUILanguage() } & 0x3FF {
                0x19 => Lang::Ru,
                0x07 => Lang::De,
                0x0C => Lang::Fr,
                0x0A => Lang::Es,
                _ => Lang::En,
            }
        }
        #[cfg(not(windows))]
        {
            let lang = std::env::var("LANG").unwrap_or_default();
            Lang::ALL
                .into_iter()
                .find(|l| lang.starts_with(&format!("{l:?}").to_lowercase()))
                .unwrap_or(Lang::En)
        }
    }
}

/// `Lang as u8`, or `UNSET`.
static CURRENT: AtomicU8 = AtomicU8::new(UNSET);
const UNSET: u8 = u8::MAX;

/// Sets the interface language; `None` follows Windows.
pub fn set(lang: Option<Lang>) {
    CURRENT.store(lang.unwrap_or_else(Lang::system) as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        // Tests assert Russian texts whatever the machine's language is.
        UNSET if cfg!(test) => Lang::Ru,
        UNSET => Lang::system(),
        n => Lang::ALL[n as usize],
    }
}

fn table(lang: Lang) -> &'static HashMap<String, String> {
    static TABLES: [OnceLock<HashMap<String, String>>; 5] = [const { OnceLock::new() }; 5];
    TABLES[lang as usize].get_or_init(|| parse(lang.source()).unwrap_or_else(|e| panic!("locales/{lang:?}.toml: {e}")))
}

/// `[section] key = "text"` -> `"section.key" -> "text"`.
fn parse(source: &str) -> Result<HashMap<String, String>, toml::de::Error> {
    fn walk(prefix: &str, table: &toml::Table, out: &mut HashMap<String, String>) {
        for (k, v) in table {
            let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
            match v {
                toml::Value::Table(t) => walk(&key, t, out),
                toml::Value::String(s) => {
                    out.insert(key, s.clone());
                }
                other => {
                    out.insert(key, other.to_string());
                }
            }
        }
    }
    let table: toml::Table = toml::from_str(source)?;
    let mut out = HashMap::new();
    walk("", &table, &mut out);
    Ok(out)
}

/// The text of `key` in the current language.
pub fn text(key: &'static str) -> &'static str {
    text_in(current(), key)
}

pub fn text_in(lang: Lang, key: &'static str) -> &'static str {
    table(lang).get(key).or_else(|| table(Lang::En).get(key)).map_or(key, String::as_str)
}

/// The text of a key built at run time (the `--help` texts), if there is one.
pub fn lookup(key: &str) -> Option<&'static str> {
    table(current()).get(key).or_else(|| table(Lang::En).get(key)).map(String::as_str)
}

/// The text of `key` with its `{name}` arguments filled in.
pub fn format(key: &'static str, args: &[(&str, String)]) -> String {
    fill(text(key), args)
}

fn fill(text: &str, args: &[(&str, String)]) -> String {
    let mut out = text.to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// `tr!("section.key")` -> `&'static str`; `tr!("section.key", name = value, ...)` -> `String`.
#[macro_export]
macro_rules! tr {
    ($key:literal) => {
        $crate::i18n::text($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::format($key, &[$((stringify!($name), ::std::string::ToString::to_string(&$value))),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// `{name}` arguments of a text.
    fn args(text: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            let Some(len) = rest[start..].find('}') else { break };
            let name = &rest[start + 1..start + len];
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                out.insert(name.to_string());
            }
            rest = &rest[start + len..];
        }
        out
    }

    #[test]
    fn every_language_has_every_key_with_the_same_arguments() {
        let base = table(Lang::Ru);
        // All languages' problems at once, so each translation can be checked on its own.
        let mut problems = Vec::new();
        for lang in Lang::ALL {
            let t = table(lang);
            let mut missing: Vec<_> = base.keys().filter(|k| !t.contains_key(*k)).collect();
            let mut extra: Vec<_> = t.keys().filter(|k| !base.contains_key(*k)).collect();
            missing.sort();
            extra.sort();
            if !missing.is_empty() || !extra.is_empty() {
                problems.push(format!("{lang:?}: missing {} {missing:?}, extra {extra:?}", missing.len()));
            }
            for (k, v) in base {
                if t.get(k).is_some_and(|tv| args(v) != args(tv)) {
                    problems.push(format!("{lang:?} {k}: arguments differ"));
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    /// Every `tr!("...")` in the sources exists, so a typo cannot show a bare key.
    #[test]
    fn every_key_in_the_code_exists() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let mut unknown = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let code = std::fs::read_to_string(&path).unwrap();
            for (at, _) in code.match_indices("tr!(\"") {
                // Not `include_str!("...")` and the like.
                if code[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let part = &code[at + 5..];
                let key = &part[..part.find('"').unwrap()];
                if !key.contains("section.key") && key != "..." && !table(Lang::Ru).contains_key(key) {
                    unknown.push(format!("{}: {key}", path.display()));
                }
            }
        }
        assert!(unknown.is_empty(), "unknown keys: {unknown:#?}");
    }

    /// User-facing text lives in `locales/`, not in the code (tests may still assert Russian).
    #[test]
    fn no_russian_left_in_the_code() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let mut found = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let code = std::fs::read_to_string(&path).unwrap();
            let code = code.split("#[cfg(test)]").next().unwrap();
            for (i, line) in code.lines().enumerate() {
                let line = line.trim_start();
                // Comments are English; the language picker shows "Русский".
                if line.starts_with("//") || line.contains("\"Русский\"") {
                    continue;
                }
                if line.chars().any(|c| ('а'..='я').contains(&c.to_lowercase().next().unwrap_or(c))) {
                    found.push(format!("{}:{}: {line}", path.display(), i + 1));
                }
            }
        }
        assert!(found.is_empty(), "Russian text in code: {found:#?}");
    }

    #[test]
    fn arguments_are_filled_and_missing_keys_fall_back() {
        assert_eq!(
            fill("Обновлено {n}: {names}", &[("n", "2".into()), ("names", "A, B".into())]),
            "Обновлено 2: A, B"
        );
        assert_eq!(text_in(Lang::De, "no.such.key"), "no.such.key");
    }
}
