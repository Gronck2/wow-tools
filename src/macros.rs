//! Macro check: reads every `macros-cache.txt` (account-wide and per character) and reports what is
//! broken: by checks that need no downloaded data, and by spell/item names against the cached name data
//! (`macro_names`) when it is there. Read-only over WTF.
//!
//! File format: `VER 3 <16 hex id> "<name>" "<icon>"`, the body lines, `END`. Lines of a body that
//! start with neither `/` nor `#` are addon data (HealBot `hbCrashProt_*`, `!dosmarkers`) and ignored.

use crate::chars::{Character, subdirs};
use crate::macro_names::{self, Names};
use crate::profiles::{Roster, last_played, realm_key};
use crate::wow::Flavor;
use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

const REALMS: &str = include_str!("../data/realms.tsv");
const DEPRECATED: &str = include_str!("../data/deprecated-api.tsv");
const ALIASES: &str = include_str!("../data/slash-aliases.tsv");

/// The game's limit of a macro's text.
const MAX_LEN: usize = 255;

/// Midnight (12.0) hides some combat values from addon code; a macro reading them may stop working.
const SECRET: &[&str] = &[
    "UnitHealth",
    "UnitHealthMax",
    "UnitPower",
    "UnitPowerMax",
    "UnitCastingInfo",
    "UnitChannelInfo",
    "C_UnitAuras.GetAuraDataByIndex",
    "C_Spell.GetSpellCooldown",
    "C_Spell.GetSpellCharges",
];

#[derive(Debug, PartialEq)]
pub struct Macro {
    pub name: String,
    pub icon: String,
    pub body: Vec<String>,
}

/// The macros of a `macros-cache.txt`; an empty file has none.
pub fn parse(text: &str) -> Vec<Macro> {
    let mut out = Vec::new();
    let mut cur: Option<Macro> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("VER ") {
            out.extend(cur.take());
            // `3 <id> "<name>" "<icon>"`: the icon is a number or a file name, never quoted inside.
            let quoted = rest.splitn(3, ' ').nth(2).unwrap_or_default();
            let (name, icon) = quoted
                .strip_prefix('"')
                .and_then(|q| q.strip_suffix('"'))
                .and_then(|q| q.rsplit_once("\" \""))
                .unwrap_or_default();
            cur = Some(Macro {
                name: name.to_string(),
                icon: icon.to_string(),
                body: Vec::new(),
            });
        } else if line == "END" {
            out.extend(cur.take());
        } else if let Some(m) = cur.as_mut() {
            m.body.push(line.to_string());
        }
    }
    out.extend(cur);
    out
}

struct Realm {
    name: &'static str,
    english: &'static str,
    locale: &'static str,
    region: &'static str,
    id: &'static str,
}

fn rows(tsv: &'static str) -> impl Iterator<Item = Vec<&'static str>> {
    tsv.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| l.split('\t').collect())
}

fn realms() -> &'static [Realm] {
    static T: OnceLock<Vec<Realm>> = OnceLock::new();
    T.get_or_init(|| {
        rows(REALMS)
            .filter_map(|r| match r[..] {
                [name, english, locale, region, id] => Some(Realm {
                    name,
                    english,
                    locale,
                    region,
                    id,
                }),
                _ => None,
            })
            .collect()
    })
}

/// (function, replacement, removed in patch, source).
fn deprecated() -> &'static [[&'static str; 4]] {
    static T: OnceLock<Vec<[&'static str; 4]>> = OnceLock::new();
    T.get_or_init(|| rows(DEPRECATED).filter_map(|r| r[..].try_into().ok()).collect())
}

/// (command, locale, alias).
fn aliases() -> &'static [[&'static str; 3]] {
    static T: OnceLock<Vec<[&'static str; 3]>> = OnceLock::new();
    T.get_or_init(|| rows(ALIASES).filter_map(|r| r[..].try_into().ok()).collect())
}

/// The command an alias stands for (`/закл` -> `/cast`); any other word comes back as it is.
pub fn canonical_command(cmd: &str) -> String {
    aliases().iter().find(|a| a[2] == cmd).map_or(cmd, |a| a[0]).to_string()
}

/// The language of a realm and whether it was only taken from the realm's name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loc {
    pub code: &'static str,
    pub guessed: bool,
}

/// `SET portal "EU"` of `WTF\Config.wtf`.
fn portal(wtf: &Path) -> Option<String> {
    let text = fs::read_to_string(wtf.join("Config.wtf")).ok()?;
    let value = text.lines().find_map(|l| l.trim().strip_prefix("SET portal "))?;
    Some(value.trim().trim_matches('"').to_string())
}

/// Locales of the game client's text (`enGB` has the names of `enUS`).
const CLIENT_LOCALES: [&str; 11] = ["enUS", "deDE", "esES", "esMX", "frFR", "itIT", "koKR", "ptBR", "ruRU", "zhCN", "zhTW"];

/// The text language of the client of a flavor: `SET textLocale` of `WTF\Config.wtf`, else `SET locale`.
fn client_locale(wtf: &Path) -> Option<&'static str> {
    let text = fs::read_to_string(wtf.join("Config.wtf")).ok()?;
    let get = |key: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(key))
            .map(|v| v.trim().trim_matches('"').to_string())
    };
    let value = get("SET textLocale ").or_else(|| get("SET locale "))?;
    let value = if value == "enGB" { "enUS" } else { value.as_str() };
    CLIENT_LOCALES.iter().copied().find(|c| *c == value)
}

/// The language of the realm whose folder is `folder`: by name (also the English one), else by the
/// realm id the account's character list gives, else Cyrillic = `ruRU` (the only Cyrillic realms
/// are the Russian EU ones). A realm known by none of these has no language: never guessed.
fn realm_locale(acc_dir: &Path, folder: &str, region: Option<&str>, roster: &OnceCell<Roster>) -> Option<Loc> {
    let key = realm_key(folder);
    let known = |code| Some(Loc { code, guessed: false });
    // Same name on several regions (Draenor): the region of the install picks one; else all must agree.
    let pick = |hits: Vec<&Realm>| {
        let same_region: Vec<&Realm> = hits
            .iter()
            .copied()
            .filter(|r| region.is_some_and(|g| r.region.eq_ignore_ascii_case(g)))
            .collect();
        let pool = if same_region.is_empty() { hits } else { same_region };
        let first = pool.first()?;
        pool.iter().all(|r| r.locale == first.locale).then_some(first.locale)
    };
    if let Some(code) = pick(realms().iter().filter(|r| realm_key(r.name) == key || realm_key(r.english) == key).collect()) {
        return known(code);
    }
    if let Some(id) = roster.get_or_init(|| Roster::read(acc_dir)).realm_id(&key)
        && let Some(code) = pick(realms().iter().filter(|r| r.id == id).collect())
    {
        return known(code);
    }
    folder
        .chars()
        .any(|c| ('\u{400}'..='\u{4FF}').contains(&c))
        .then_some(Loc { code: "ruRU", guessed: true })
}

/// A character a macro runs for: the realm decides which slash aliases the client knows.
#[derive(Debug, Clone)]
pub struct Target {
    pub label: String,
    pub loc: Option<Loc>,
    /// Client languages the player really uses (see `scan`); a Russian realm ignores them.
    pub client: Vec<&'static str>,
}

impl Target {
    pub fn ru(&self) -> bool {
        self.loc.is_some_and(|l| l.code == "ruRU")
    }

    /// The languages whose names and aliases work for this character, the realm's first: a Russian realm
    /// accepts only ruRU, any other the realm's language or a client language of the player.
    pub fn valid(&self) -> Vec<&'static str> {
        if self.ru() {
            return vec!["ruRU"];
        }
        self.loc.map(|l| l.code).into_iter().chain(self.client.iter().copied()).collect()
    }
}

/// How sure a wrong-language name is for its targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum How {
    /// A Russian realm, the character's own macro.
    Firm,
    /// A Russian realm, an account-wide macro.
    Maybe,
    /// Another realm: works with a client in the name's language.
    Client,
    /// The realm's language is unknown.
    Realm,
}

#[derive(Debug, Clone)]
pub enum Kind {
    Deprecated {
        func: &'static str,
        replacement: &'static str,
        patch: &'static str,
        url: &'static str,
    },
    TooLong {
        chars: usize,
    },
    Brackets,
    /// `bad`: Russian realms (`firm`: a character's own macro, else it may not work there); `soft`: other realms,
    /// where it works with a client in that language; `unknown`: realms of unknown language.
    Alias {
        alias: String,
        command: &'static str,
        locales: String,
        bad: Vec<String>,
        soft: Vec<String>,
        unknown: Vec<String>,
        firm: bool,
    },
    Secret {
        func: &'static str,
    },
    /// A name of the language `lang`; `fix` is what the realm's language calls it.
    WrongLanguage {
        name: String,
        lang: String,
        fix: String,
        bad: Vec<String>,
        how: How,
    },
    UnknownName {
        name: String,
    },
}

impl Kind {
    /// The short name of the kind, for the counts of the panel.
    pub fn title(&self) -> &'static str {
        match self {
            Kind::Deprecated { .. } => tr!("macros.kind_deprecated"),
            Kind::TooLong { .. } => tr!("macros.kind_too_long"),
            Kind::Brackets => tr!("macros.kind_brackets"),
            Kind::Alias { .. } => tr!("macros.kind_alias"),
            Kind::Secret { .. } => tr!("macros.kind_secret"),
            Kind::WrongLanguage { .. } => tr!("macros.kind_wrong"),
            Kind::UnknownName { .. } => tr!("macros.kind_unknown"),
        }
    }
}

/// Whole-word match: not part of a longer name and not a field (`C_Spell.GetSpellInfo`, `x:GetSpellInfo`).
fn find_word(line: &str, word: &str) -> bool {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    line.match_indices(word).any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let after = line[at + word.len()..].chars().next();
        !before.is_some_and(|c| ident(c) || c == '.' || c == ':') && !after.is_some_and(ident)
    })
}

fn is_retail(flavor: &str) -> bool {
    matches!(flavor, "_retail_" | "_ptr_" | "_xptr_" | "_beta_" | "_alpha_")
}

/// Every problem of one macro as (body line from 1, 0 = the whole text; kind).
/// shortcut: brackets are counted on every command line, so a chat line with a lone `[` is flagged; add a chat-command list if it nags.
pub fn check(m: &Macro, retail: bool, targets: &[Target], firm: bool) -> Vec<(usize, Kind)> {
    let mut out = Vec::new();
    let text = m.body.join("\n");
    // The game counts characters, not bytes (a saved 247-character macro is 265 bytes).
    let chars = text.chars().count();
    if chars > MAX_LEN {
        out.push((0, Kind::TooLong { chars }));
    }
    for (i, line) in m.body.iter().enumerate().map(|(i, l)| (i + 1, l.trim())) {
        if !line.starts_with(['/', '#']) {
            continue;
        }
        let end = line.find(|c: char| c.is_whitespace() || c == '[').unwrap_or(line.len());
        let cmd = line[..end].to_lowercase();
        if matches!(cmd.as_str(), "/run" | "/script") {
            if retail {
                let code = &line[end..];
                for [func, replacement, patch, url] in deprecated().iter().copied() {
                    if find_word(code, func) {
                        out.push((i, Kind::Deprecated { func, replacement, patch, url }));
                    }
                }
                if let Some(func) = SECRET.iter().find(|f| find_word(code, f)) {
                    out.push((i, Kind::Secret { func }));
                }
            }
            continue;
        }
        let mut depth = 0i32;
        let balanced = line.chars().all(|c| {
            depth += i32::from(c == '[') - i32::from(c == ']');
            depth >= 0
        }) && depth == 0;
        if !balanced {
            out.push((i, Kind::Brackets));
        }
        let found: Vec<&[&str; 3]> = aliases().iter().filter(|a| a[2] == cmd).collect();
        if let Some(first) = found.first() {
            let locales: Vec<&str> = found.iter().map(|a| a[1]).collect();
            let label = |t: &Target| match t.loc {
                Some(l) if l.guessed => format!("{} ({}, {})", t.label, l.code, tr!("macros.from_name")),
                Some(l) => format!("{} ({})", t.label, l.code),
                None => t.label.clone(),
            };
            let (mut bad, mut soft, mut unknown) = (Vec::new(), Vec::new(), Vec::new());
            for t in targets.iter().filter(|t| !t.valid().iter().any(|c| locales.contains(c))) {
                // A realm of unknown language is never called bad: it may well be the alias's language.
                match (t.ru(), t.loc.is_some()) {
                    (true, _) => bad.push(label(t)),
                    (_, true) => soft.push(label(t)),
                    _ => unknown.push(label(t)),
                }
            }
            let firm = firm && !bad.is_empty();
            if !bad.is_empty() || !soft.is_empty() || !unknown.is_empty() {
                out.push((
                    i,
                    Kind::Alias {
                        alias: cmd,
                        command: first[0],
                        locales: locales.join("/"),
                        bad,
                        soft,
                        unknown,
                        firm,
                    },
                ));
            }
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub flavor: String,
    /// "Account X" or "Name-Realm".
    pub scope: String,
    pub name: String,
    pub line: usize,
    pub kind: Kind,
}

impl Finding {
    pub fn problem(&self) -> String {
        match &self.kind {
            Kind::Deprecated { func, patch, .. } => tr!("macros.deprecated", func = func, patch = patch),
            Kind::TooLong { chars } => tr!("macros.too_long", n = chars, max = MAX_LEN),
            Kind::Brackets => tr!("macros.brackets").to_string(),
            Kind::Alias {
                alias,
                command,
                locales,
                bad,
                soft,
                unknown,
                firm,
            } => {
                let mut parts = Vec::new();
                if !bad.is_empty() {
                    let targets = crate::maintenance::names_text(bad, 6);
                    parts.push(if *firm {
                        tr!("macros.alias_firm", alias = alias, command = command, locale = locales, targets = targets)
                    } else {
                        tr!("macros.alias_maybe", alias = alias, command = command, locale = locales, targets = targets)
                    });
                }
                if !soft.is_empty() {
                    let targets = crate::maintenance::names_text(soft, 6);
                    parts.push(tr!("macros.alias_soft", alias = alias, command = command, locale = locales, targets = targets));
                }
                if !unknown.is_empty() {
                    parts.push(tr!(
                        "macros.alias_unknown",
                        alias = alias,
                        command = command,
                        locale = locales,
                        targets = crate::maintenance::names_text(unknown, 6)
                    ));
                }
                parts.join("; ")
            }
            Kind::Secret { func } => tr!("macros.secret", func = func),
            Kind::WrongLanguage { name, lang, bad, how, .. } => {
                let targets = crate::maintenance::names_text(bad, 6);
                match how {
                    How::Firm => tr!("macros.wrong_firm", name = name, lang = lang, targets = targets),
                    How::Maybe => tr!("macros.wrong_maybe", name = name, lang = lang, targets = targets),
                    How::Client => tr!("macros.wrong_soft", name = name, lang = lang, targets = targets),
                    How::Realm => tr!("macros.wrong_realm", name = name, lang = lang, targets = targets),
                }
            }
            Kind::UnknownName { name } => tr!("macros.unknown_name", name = name),
        }
    }

    pub fn fix(&self) -> Option<String> {
        match &self.kind {
            Kind::Deprecated { replacement, url, .. } => Some(tr!("macros.fix_deprecated", replacement = replacement, url = url)),
            Kind::TooLong { chars } => Some(tr!("macros.fix_too_long", over = chars - MAX_LEN)),
            Kind::Brackets => Some(tr!("macros.fix_brackets").to_string()),
            Kind::Alias { command, .. } => Some(tr!("macros.fix_alias", command = command)),
            Kind::Secret { .. } | Kind::UnknownName { .. } => None,
            Kind::WrongLanguage { fix, .. } => Some(tr!("macros.fix_wrong", fix = fix)),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Scan {
    pub findings: Vec<Finding>,
    /// Macros read.
    pub checked: usize,
    /// Per product, the realm languages of the characters found (what `--update-names` downloads).
    pub locales: BTreeMap<String, BTreeSet<&'static str>>,
    /// Flavors with macros whose realm languages have no name data yet.
    pub missing: Vec<String>,
}

impl Scan {
    /// How many findings there are of each kind, the most first.
    pub fn counts(&self) -> Vec<(&'static str, usize)> {
        let mut out: Vec<(&'static str, usize)> = Vec::new();
        for f in &self.findings {
            let title = f.kind.title();
            match out.iter_mut().find(|(t, _)| *t == title) {
                Some((_, n)) => *n += 1,
                None => out.push((title, 1)),
            }
        }
        out.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
        out
    }
}

/// Checks the macros of every account and character of `flavors`; names are checked against the cache in `names_dir`.
pub fn scan(flavors: &[Flavor], names_dir: &Path) -> Scan {
    let mut scan = Scan::default();
    for fl in flavors {
        let wtf = fl.wtf_dir();
        let region = portal(&wtf);
        let retail = is_retail(&fl.name);
        let product = fl.product();
        let names = product.as_deref().map(|p| Names::load(names_dir, p)).unwrap_or_default();
        let client = client_locale(&wtf);
        let (checked_before, mut seen) = (scan.checked, BTreeSet::new());
        let mut check_file = |path: &Path, scope: &str, targets: &[Target], firm: bool| {
            let Ok(bytes) = fs::read(path) else { return };
            for m in parse(&String::from_utf8_lossy(&bytes)) {
                scan.checked += 1;
                let mut found = check(&m, retail, targets, firm);
                found.extend(macro_names::check(&m, targets, firm, &names));
                found.sort_by_key(|(line, _)| *line);
                for (line, kind) in found {
                    scan.findings.push(Finding {
                        flavor: fl.title(),
                        scope: scope.to_string(),
                        name: m.name.clone(),
                        line,
                        kind,
                    });
                }
            }
        };
        for (account, acc_dir) in subdirs(&wtf.join("Account")) {
            let roster = OnceCell::new();
            let mut chars = Vec::new();
            for (realm, realm_dir) in subdirs(&acc_dir) {
                let loc = realm_locale(&acc_dir, &realm, region.as_deref(), &roster);
                for (name, dir) in subdirs(&realm_dir) {
                    // A deleted character's folder stays in WTF: not a target.
                    let c = Character {
                        account: account.clone(),
                        realm: realm.clone(),
                        name: name.clone(),
                        dir: dir.clone(),
                    };
                    if !roster.get_or_init(|| Roster::read(&acc_dir)).has(&c, last_played(&c)) {
                        continue;
                    }
                    chars.push((
                        Target {
                            label: format!("{name}-{realm}"),
                            loc,
                            client: client.into_iter().collect(),
                        },
                        dir,
                    ));
                }
            }
            // Russian realms on the account: its other characters may well run a Russian client.
            if chars.iter().any(|(t, _)| t.ru()) {
                chars.iter_mut().for_each(|(t, _)| t.client.push("ruRU"));
            }
            seen.extend(chars.iter().flat_map(|(t, _)| t.valid()));
            let all: Vec<Target> = chars.iter().map(|(t, _)| t.clone()).collect();
            check_file(&acc_dir.join("macros-cache.txt"), &tr!("macros.account", account = account), &all, false);
            for (target, dir) in &chars {
                check_file(&dir.join("macros-cache.txt"), &target.label, std::slice::from_ref(target), true);
            }
        }
        if let Some(product) = product {
            scan.locales.entry(product).or_default().extend(&seen);
            if scan.checked > checked_before && seen.iter().any(|l| !names.has(l)) {
                scan.missing.push(fl.title());
            }
        }
    }
    scan
}

pub fn report(scan: &Scan) -> String {
    let mut out = String::new();
    let mut head: Option<(&str, &str)> = None;
    for f in &scan.findings {
        if head != Some((&f.flavor, &f.scope)) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("{} / {}\n", f.flavor, f.scope));
            head = Some((&f.flavor, &f.scope));
        }
        out.push_str(&if f.line == 0 {
            tr!("macros.item", name = f.name, problem = f.problem())
        } else {
            tr!("macros.item_line", name = f.name, line = f.line, problem = f.problem())
        });
        out.push('\n');
        if let Some(fix) = f.fix() {
            out.push_str(&format!("      -> {fix}\n"));
        }
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&if scan.findings.is_empty() {
        tr!("macros.none", n = scan.checked)
    } else {
        tr!("macros.total", found = scan.findings.len(), n = scan.checked)
    });
    if !scan.missing.is_empty() {
        out.push('\n');
        out.push_str(&tr!("macros.no_names", flavors = scan.missing.join(", ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(body: &[&str]) -> Macro {
        Macro {
            name: "Test".into(),
            icon: "1".into(),
            body: body.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn target(label: &str, code: Option<&'static str>) -> Target {
        Target {
            label: label.into(),
            loc: code.map(|code| Loc { code, guessed: false }),
            client: Vec::new(),
        }
    }

    fn kinds(m: &Macro, retail: bool, targets: &[Target], firm: bool) -> Vec<(usize, String)> {
        check(m, retail, targets, firm).into_iter().map(|(l, k)| (l, format!("{k:?}"))).collect()
    }

    #[test]
    fn parses_macros_icons_crlf_and_empty_files() {
        assert!(parse("").is_empty());
        let text = "VER 3 0000000000000001 \"A b\" \"135851\"\r\n#showtooltip\r\n/cast Heal\r\nEND\r\nVER 3 0000000000000002 \"Pet\" \"INV_MISC_QUESTIONMARK\"\n!dosmarkers\nEND\n";
        let ms = parse(text);
        assert_eq!(ms.len(), 2);
        assert_eq!((ms[0].name.as_str(), ms[0].icon.as_str()), ("A b", "135851"));
        assert_eq!(ms[0].body, ["#showtooltip", "/cast Heal"]);
        assert_eq!(ms[1].icon, "INV_MISC_QUESTIONMARK");
        // Addon data is kept in the body but never judged.
        assert!(check(&ms[1], true, &[], true).is_empty());
    }

    #[test]
    fn deprecated_api_is_found_in_whole_words_on_retail_only() {
        let bad = m(&["/run local n = GetSpellInfo(100) print(UnitBuff(\"player\", 1))"]);
        let found = kinds(&bad, true, &[], true);
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|f| f.1.contains("GetSpellInfo")) && found.iter().any(|f| f.1.contains("UnitBuff")));
        assert!(kinds(&bad, false, &[], true).is_empty());
        let fine = m(&["/script print(C_Spell.GetSpellInfo(100), obj:GetItemInfo(1), MyGetSpellInfo(2), GetSpellInfoX)"]);
        assert!(kinds(&fine, true, &[], true).is_empty());
    }

    #[test]
    fn secret_values_are_a_note_on_retail() {
        let found = kinds(&m(&["/run print(UnitHealth(\"target\"))"]), true, &[], true);
        assert_eq!(found.len(), 1);
        assert!(found[0].1.starts_with("Secret"));
        assert!(
            kinds(&m(&["/run print(UnitHealthMax(\"target\"))"]), true, &[], true)[0]
                .1
                .contains("UnitHealthMax")
        );
    }

    #[test]
    fn length_counts_characters_not_bytes() {
        let long = m(&[&format!("/say {}", "a".repeat(260))]);
        assert!(kinds(&long, true, &[], true)[0].1.starts_with("TooLong"));
        // 200 Cyrillic letters: 200 characters, 400 bytes: fine.
        let wide = m(&[&format!("/say {}", "ж".repeat(200))]);
        assert!(kinds(&wide, true, &[], true).is_empty());
        // 250 Cyrillic letters + "/say ": 255 characters, 505 bytes: still fine.
        let wider = m(&[&format!("/say {}", "ж".repeat(250))]);
        assert!(kinds(&wider, true, &[], true).is_empty());
        assert!(kinds(&m(&["/say hi"]), true, &[], true).is_empty());
    }

    #[test]
    fn unbalanced_brackets_are_found() {
        assert_eq!(kinds(&m(&["#showtooltip", "/cast [@focus,exists Heal"]), true, &[], true)[0].0, 2);
        assert_eq!(kinds(&m(&["/cast ]Heal["]), true, &[], true).len(), 1);
        assert!(kinds(&m(&["/cast [mod:shift][@focus,help][] Heal", "/run t = {}; t[1] = 2"]), true, &[], true).is_empty());
    }

    #[test]
    fn alias_is_judged_by_the_realm_language() {
        let mac = m(&["/закл Heal"]);
        let ru = target("A-Ru", Some("ruRU"));
        let en = target("B-En", Some("enUS"));
        assert!(check(&mac, true, std::slice::from_ref(&ru), true).is_empty());
        let firm = check(&mac, true, std::slice::from_ref(&en), true);
        let Kind::Alias { soft, bad, command, .. } = &firm[0].1 else {
            panic!("{firm:?}")
        };
        assert_eq!((soft.as_slice(), *command), (&["B-En (enUS)".to_string()][..], "/cast"));
        assert!(bad.is_empty());
        let soft = check(&mac, true, &[ru.clone(), en.clone()], false);
        assert!(matches!(&soft[0].1, Kind::Alias { firm: false, bad, soft, .. } if bad.is_empty() && soft.len() == 1));
        // An English realm with a Russian client language (or an unknown realm with one) knows the alias.
        let mut ru_client = en;
        ru_client.client = vec!["ruRU"];
        let mut nowhere = target("C-Nowhere", None);
        nowhere.client = vec!["ruRU"];
        assert!(check(&mac, true, &[ru_client, nowhere], false).is_empty());
        // An unknown realm whose client language is not the alias's is never "bad": its language is unknown.
        let mut en_client = target("D-Nowhere", None);
        en_client.client = vec!["enUS"];
        let f = check(&mac, true, &[en_client], true);
        assert!(matches!(&f[0].1, Kind::Alias { bad, soft, unknown, .. } if bad.is_empty() && soft.is_empty() && unknown.len() == 1));
        let unknown = check(&mac, true, &[target("C-Nowhere", None)], true);
        assert!(matches!(&unknown[0].1, Kind::Alias { bad, unknown, .. } if bad.is_empty() && unknown.len() == 1));
        // The English command is fine everywhere.
        assert!(check(&m(&["/cast Heal"]), true, &[target("B-En", Some("enUS"))], true).is_empty());
    }

    /// A fake WTF tree: realm folders, characters and macro files.
    fn tree(realms: &[(&str, &str)]) -> (tempfile::TempDir, Flavor) {
        let dir = tempfile::tempdir().unwrap();
        let wtf = dir.path().join("WTF");
        for (realm, name) in realms {
            let ch = wtf.join("Account").join("ACC").join(realm).join(name);
            fs::create_dir_all(&ch).unwrap();
        }
        fs::write(wtf.join("Config.wtf"), "SET portal \"EU\"\r\nSET locale \"enGB\"\r\n").unwrap();
        let fl = Flavor::new("_retail_", dir.path());
        (dir, fl)
    }

    fn put(fl: &Flavor, rel: &str, text: &str) {
        fs::write(fl.wtf_dir().join("Account").join("ACC").join(rel), text).unwrap();
    }

    #[test]
    fn realm_matches_by_name_english_name_id_and_cyrillic() {
        let (_d, fl) = tree(&[
            ("Tarren Mill", "Aaa"),
            ("Gordunni", "Bbb"),
            ("Oldname", "Ccc"),
            ("Нигде", "Ddd"),
            ("Nowhere", "Eee"),
        ]);
        let acc = fl.wtf_dir().join("Account").join("ACC");
        fs::create_dir_all(acc.join("SavedVariables")).unwrap();
        fs::write(acc.join("character-list-order.txt"), "1602-0000AAAA").unwrap();
        fs::write(
            acc.join("SavedVariables").join("Some.lua"),
            "X = { [\"Player-1602-0000AAAA\"] = { [\"name\"] = \"Ccc\", [\"realm\"] = \"Oldname\" } }",
        )
        .unwrap();
        let roster = OnceCell::new();
        let loc = |folder: &str| realm_locale(&acc, folder, Some("EU"), &roster);
        assert_eq!(loc("TarrenMill"), Some(Loc { code: "enUS", guessed: false }));
        assert_eq!(loc("Gordunni").map(|l| l.code), Some("ruRU"));
        assert_eq!(loc("Oldname"), Some(Loc { code: "ruRU", guessed: false }));
        assert_eq!(loc("Нигде"), Some(Loc { code: "ruRU", guessed: true }));
        assert_eq!(loc("Nowhere"), None);
        assert_eq!(loc("Гордунни"), Some(Loc { code: "ruRU", guessed: false }));
        assert_eq!(portal(&fl.wtf_dir()).as_deref(), Some("EU"));
    }

    #[test]
    fn client_locale_reads_text_locale_then_locale_and_maps_engb() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = |text: &str| {
            fs::write(dir.path().join("Config.wtf"), text).unwrap();
            client_locale(dir.path())
        };
        assert_eq!(client_locale(dir.path()), None);
        assert_eq!(cfg("SET locale \"enUS\"\r\nSET textLocale \"deDE\"\r\n"), Some("deDE"));
        assert_eq!(cfg("SET locale \"frFR\"\r\n"), Some("frFR"));
        assert_eq!(cfg("SET textLocale \"enGB\"\r\n"), Some("enUS"));
        assert_eq!(cfg("SET textLocale \"xxXX\"\r\n"), None);
    }

    #[test]
    fn scan_client_languages_count_per_account_and_feed_update_names() {
        let (_d, fl) = tree(&[("Tarren Mill", "Aaa")]);
        let wtf = fl.wtf_dir();
        fs::write(wtf.join("Config.wtf"), "SET portal \"EU\"\r\nSET textLocale \"deDE\"\r\n").unwrap();
        // A second account with a Russian realm: its characters must not give ACC a Russian client.
        fs::create_dir_all(wtf.join("Account").join("RU").join("Гордунни").join("Bbb")).unwrap();
        put(
            &fl,
            "Tarren Mill/Aaa/macros-cache.txt",
            "VER 3 0000000000000001 \"F\" \"1\"\n/cast Огненный шар\nEND\n",
        );
        let names = tempfile::tempdir().unwrap();
        put_names(names.path(), "enUS", "1\tFireball\n");
        put_names(names.path(), "deDE", "1\tFeuerball\n");
        put_names(names.path(), "ruRU", "1\tОгненный шар\n");
        let s = scan(std::slice::from_ref(&fl), names.path());
        // Download set: realm + client languages of ACC, plus the Russian realm's own.
        assert_eq!(s.locales["wow"], BTreeSet::from(["enUS", "deDE", "ruRU"]));
        assert!(
            s.findings
                .iter()
                .any(|f| f.scope == "Aaa-Tarren Mill" && matches!(&f.kind, Kind::WrongLanguage { .. }))
        );
        // A Russian-realm character on the same account: ACC's characters get a Russian client.
        fs::create_dir_all(wtf.join("Account").join("ACC").join("Гордунни").join("Ccc")).unwrap();
        let s = scan(std::slice::from_ref(&fl), names.path());
        assert!(!s.findings.iter().any(|f| f.scope == "Aaa-Tarren Mill"));
    }

    /// A name cache as `macro_names` writes it: `wow-<locale>.zip` with tab-separated `spell` and `item` entries.
    fn put_names(dir: &Path, locale: &str, spell: &str) {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(fs::File::create(dir.join(format!("wow-{locale}.zip"))).unwrap());
        for (entry, text) in [("spell", spell), ("item", "")] {
            zip.start_file(entry, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn scan_checks_names_firm_for_character_soft_for_account_and_hints_without_cache() {
        let (_d, fl) = tree(&[("Tarren Mill", "Aaa"), ("Гордунни", "Bbb")]);
        let mac = "VER 3 0000000000000001 \"F\" \"1\"\n/cast Fireball\nEND\n";
        put(&fl, "macros-cache.txt", mac);
        put(&fl, "Tarren Mill/Aaa/macros-cache.txt", mac);
        put(&fl, "Гордунни/Bbb/macros-cache.txt", mac);
        let names = tempfile::tempdir().unwrap();
        put_names(names.path(), "enUS", "1\tFireball\n");
        // ruRU not cached: its realm is skipped and the hint names the flavor.
        let s = scan(std::slice::from_ref(&fl), names.path());
        assert!(s.findings.is_empty());
        assert_eq!(s.missing.len(), 1);
        assert!(report(&s).contains("--update-names"));
        assert_eq!(s.locales["wow"], BTreeSet::from(["enUS", "ruRU"]));
        // With ruRU: Bbb's own file is firm, the account's list is soft, Aaa is fine.
        put_names(names.path(), "ruRU", "1\tОгненный шар\n");
        let s = scan(std::slice::from_ref(&fl), names.path());
        assert!(s.missing.is_empty() && !report(&s).contains("--update-names"));
        let wrong: Vec<(&str, How)> = s
            .findings
            .iter()
            .filter_map(|f| match &f.kind {
                Kind::WrongLanguage { how, fix, .. } if fix == "Огненный шар" => Some((f.scope.as_str(), *how)),
                _ => None,
            })
            .collect();
        assert_eq!(wrong.len(), 2);
        assert!(wrong.contains(&("Bbb-Гордунни", How::Firm)));
        // The Russian realm in the account list "may not work", it never gets the client clause.
        assert!(wrong.iter().any(|(scope, how)| *scope != "Bbb-Гордунни" && *how == How::Maybe));
        assert!(!s.findings.iter().any(|f| f.scope == "Aaa-Tarren Mill"));
    }

    #[test]
    fn scan_groups_account_and_character_findings() {
        let (_d, fl) = tree(&[("Tarren Mill", "Aaa"), ("Tarren Mill", "Bbb")]);
        let mac = "VER 3 0000000000000001 \"Heal\" \"1\"\n/закл Heal\nEND\nVER 3 0000000000000002 \"Old\" \"2\"\n/run GetSpellInfo(1)\nEND\n";
        put(&fl, "macros-cache.txt", mac);
        put(&fl, "Tarren Mill/Aaa/macros-cache.txt", mac);
        put(&fl, "Tarren Mill/Bbb/macros-cache.txt", mac);
        let s = scan(std::slice::from_ref(&fl), std::path::Path::new("no-names"));
        assert_eq!(s.checked, 6);
        // Account: alias (works only with a Russian client on both) + deprecated; Aaa and Bbb: both, soft.
        let scopes: Vec<&str> = s.findings.iter().map(|f| f.scope.as_str()).collect();
        assert_eq!(scopes.iter().filter(|x| **x == "Aaa-Tarren Mill").count(), 2);
        assert_eq!(scopes.iter().filter(|x| **x == "Bbb-Tarren Mill").count(), 2);
        let alias = s.findings.iter().find(|f| matches!(&f.kind, Kind::Alias { firm: false, .. })).unwrap();
        assert!(alias.problem().contains("Aaa-Tarren Mill (enUS)"));
        assert!(!s.findings.iter().any(|f| matches!(&f.kind, Kind::Alias { firm: true, .. })));
        let text = report(&s);
        assert!(text.contains("Heal (строка 1)") && text.contains("C_Spell.GetSpellInfo"));
        assert!(report(&Scan::default()).contains('0'));
    }

    #[test]
    fn deleted_characters_are_not_targets_and_not_checked() {
        let (_d, fl) = tree(&[("Oldname", "Ccc"), ("Oldname", "Gone")]);
        let acc = fl.wtf_dir().join("Account").join("ACC");
        let mac = "VER 3 0000000000000001 \"Heal\" \"1\"\n/закл Heal\nEND\n";
        put(&fl, "macros-cache.txt", mac);
        put(&fl, "Oldname/Ccc/macros-cache.txt", mac);
        put(&fl, "Oldname/Gone/macros-cache.txt", mac);
        // The list is written last: the folders were not played after it.
        fs::create_dir_all(acc.join("SavedVariables")).unwrap();
        fs::write(acc.join("character-list-order.txt"), "1602-0000AAAA").unwrap();
        fs::write(
            acc.join("SavedVariables").join("Some.lua"),
            "X = { [\"Player-1602-0000AAAA\"] = { [\"name\"] = \"Ccc\", [\"realm\"] = \"Oldname\" } }",
        )
        .unwrap();
        let s = scan(std::slice::from_ref(&fl), std::path::Path::new("no-names"));
        // The account's and Ccc's file; Gone's folder is left alone.
        assert_eq!(s.checked, 2);
        assert!(s.findings.is_empty());
    }

    #[test]
    fn deleted_character_is_not_listed_in_account_wide_findings() {
        let (_d, fl) = tree(&[("Tarren Mill", "Aaa"), ("Tarren Mill", "Gone")]);
        let acc = fl.wtf_dir().join("Account").join("ACC");
        put(&fl, "macros-cache.txt", "VER 3 0000000000000001 \"Heal\" \"1\"\n/закл Heal\nEND\n");
        fs::create_dir_all(acc.join("SavedVariables")).unwrap();
        fs::write(acc.join("character-list-order.txt"), "1306-0000AAAA").unwrap();
        fs::write(
            acc.join("SavedVariables").join("Some.lua"),
            "X = { [\"Player-1306-0000AAAA\"] = { [\"name\"] = \"Aaa\", [\"realm\"] = \"Tarren Mill\" } }",
        )
        .unwrap();
        let s = scan(std::slice::from_ref(&fl), std::path::Path::new("no-names"));
        assert_eq!(s.checked, 1);
        let alias = s.findings.iter().find(|f| matches!(&f.kind, Kind::Alias { .. })).unwrap();
        assert!(alias.problem().contains("Aaa-Tarren Mill") && !alias.problem().contains("Gone"));
    }

    #[test]
    fn parser_survives_quotes_and_unterminated_macros() {
        let ms = parse("VER 3 0000000000000001 \"Say \"hi\"\" \"12\"\n/say x\nEND\nVER 3 0000000000000002 \"Cut\" \"3\"\n/cast Heal");
        assert_eq!(ms.len(), 2);
        assert_eq!((ms[0].name.as_str(), ms[0].icon.as_str()), ("Say \"hi\"", "12"));
        assert_eq!((ms[1].name.as_str(), ms[1].body.as_slice()), ("Cut", &["/cast Heal".to_string()][..]));
    }

    #[test]
    fn field_access_and_classic_are_not_flagged_and_limit_is_exact() {
        assert!(kinds(&m(&["/run print(x.GetSpellInfo(1))"]), true, &[], true).is_empty());
        assert!(kinds(&m(&["/run print(UnitHealth(\"target\"))"]), false, &[], true).is_empty());
        // 255 characters pass, 256 do not.
        assert!(kinds(&m(&[&format!("/say {}", "a".repeat(250))]), true, &[], true).is_empty());
        assert!(kinds(&m(&[&format!("/say {}", "a".repeat(251))]), true, &[], true)[0].1.starts_with("TooLong"));
    }

    #[test]
    fn guessed_language_is_marked_and_soft_wording_lists_the_characters() {
        let guessed = Target {
            label: "Aaa-Нигде".into(),
            loc: Some(Loc { code: "deDE", guessed: true }),
            client: Vec::new(),
        };
        let found = check(&m(&["/закл Heal"]), true, &[guessed, target("Bbb-Nowhere", None)], false);
        let Kind::Alias {
            soft: bad,
            unknown,
            firm: false,
            ..
        } = &found[0].1
        else {
            panic!("{found:?}")
        };
        assert_eq!(unknown, &["Bbb-Nowhere"]);
        assert!(bad[0].starts_with("Aaa-Нигде (deDE, ") && bad[0] != "Aaa-Нигде (deDE)");
    }

    #[test]
    fn counts_group_findings_by_kind_most_first() {
        let f = |kind| Finding {
            flavor: "Retail".into(),
            scope: "s".into(),
            name: "n".into(),
            line: 1,
            kind,
        };
        let name = |n: &str| Kind::UnknownName { name: n.into() };
        let scan = Scan {
            findings: vec![f(Kind::Brackets), f(name("a")), f(name("b"))],
            ..Default::default()
        };
        assert_eq!(scan.counts(), [(tr!("macros.kind_unknown"), 2), (tr!("macros.kind_brackets"), 1)]);
        assert!(Scan::default().counts().is_empty());
    }
}
