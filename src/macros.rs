//! Macro check: reads every `macros-cache.txt` (account-wide and per character) and reports what is
//! broken by checks that need no downloaded data. Read-only over WTF.
//!
//! File format: `VER 3 <16 hex id> "<name>" "<icon>"`, the body lines, `END`. Lines of a body that
//! start with neither `/` nor `#` are addon data (HealBot `hbCrashProt_*`, `!dosmarkers`) and ignored.

use crate::chars::{Character, subdirs};
use crate::profiles::{Roster, last_played, realm_key};
use crate::wow::Flavor;
use std::cell::OnceCell;
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
}

#[derive(Debug)]
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
    /// `firm`: a character's own macro; else an account-wide one that may not work on `bad`.
    Alias {
        alias: String,
        command: &'static str,
        locales: String,
        bad: Vec<String>,
        unknown: Vec<String>,
        firm: bool,
    },
    Secret {
        func: &'static str,
    },
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
            let bad: Vec<String> = targets
                .iter()
                .filter(|t| t.loc.is_some_and(|l| !locales.contains(&l.code)))
                .map(label)
                .collect();
            let unknown: Vec<String> = targets.iter().filter(|t| t.loc.is_none()).map(label).collect();
            if !bad.is_empty() || !unknown.is_empty() {
                out.push((
                    i,
                    Kind::Alias {
                        alias: cmd,
                        command: first[0],
                        locales: locales.join("/"),
                        bad,
                        unknown,
                        firm,
                    },
                ));
            }
        }
    }
    out
}

#[derive(Debug)]
pub struct Finding {
    pub flavor: String,
    /// "Account X" or "Name-Realm".
    pub scope: String,
    pub name: String,
    pub line: usize,
    pub kind: Kind,
}

impl Finding {
    fn problem(&self) -> String {
        match &self.kind {
            Kind::Deprecated { func, patch, .. } => tr!("macros.deprecated", func = func, patch = patch),
            Kind::TooLong { chars } => tr!("macros.too_long", n = chars, max = MAX_LEN),
            Kind::Brackets => tr!("macros.brackets").to_string(),
            Kind::Alias {
                alias,
                command,
                locales,
                bad,
                unknown,
                firm,
            } => {
                let mut parts = Vec::new();
                let targets = crate::maintenance::names_text(bad, 6);
                if !bad.is_empty() {
                    parts.push(if *firm {
                        tr!("macros.alias_firm", alias = alias, command = command, locale = locales, targets = targets)
                    } else {
                        tr!("macros.alias_soft", alias = alias, command = command, locale = locales, targets = targets)
                    });
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
        }
    }

    fn fix(&self) -> Option<String> {
        match &self.kind {
            Kind::Deprecated { replacement, url, .. } => Some(tr!("macros.fix_deprecated", replacement = replacement, url = url)),
            Kind::TooLong { chars } => Some(tr!("macros.fix_too_long", over = chars - MAX_LEN)),
            Kind::Brackets => Some(tr!("macros.fix_brackets").to_string()),
            Kind::Alias { command, .. } => Some(tr!("macros.fix_alias", command = command)),
            Kind::Secret { .. } => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct Scan {
    pub findings: Vec<Finding>,
    /// Macros read.
    pub checked: usize,
}

/// Checks the macros of every account and character of `flavors`.
pub fn scan(flavors: &[Flavor]) -> Scan {
    let mut scan = Scan::default();
    for fl in flavors {
        let wtf = fl.wtf_dir();
        let region = portal(&wtf);
        let retail = is_retail(&fl.name);
        let mut check_file = |path: &Path, scope: &str, targets: &[Target], firm: bool| {
            let Ok(bytes) = fs::read(path) else { return };
            for m in parse(&String::from_utf8_lossy(&bytes)) {
                scan.checked += 1;
                for (line, kind) in check(&m, retail, targets, firm) {
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
                        },
                        dir,
                    ));
                }
            }
            let all: Vec<Target> = chars.iter().map(|(t, _)| t.clone()).collect();
            check_file(&acc_dir.join("macros-cache.txt"), &tr!("macros.account", account = account), &all, false);
            for (target, dir) in &chars {
                check_file(&dir.join("macros-cache.txt"), &target.label, std::slice::from_ref(target), true);
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
        let Kind::Alias { bad, firm: true, command, .. } = &firm[0].1 else {
            panic!("{firm:?}")
        };
        assert_eq!((bad.as_slice(), *command), (&["B-En (enUS)".to_string()][..], "/cast"));
        let soft = check(&mac, true, &[ru, en], false);
        assert!(matches!(&soft[0].1, Kind::Alias { firm: false, bad, .. } if bad.len() == 1));
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
    fn scan_groups_account_and_character_findings() {
        let (_d, fl) = tree(&[("Tarren Mill", "Aaa"), ("Гордунни", "Bbb")]);
        let mac = "VER 3 0000000000000001 \"Heal\" \"1\"\n/закл Heal\nEND\nVER 3 0000000000000002 \"Old\" \"2\"\n/run GetSpellInfo(1)\nEND\n";
        put(&fl, "macros-cache.txt", mac);
        put(&fl, "Tarren Mill/Aaa/macros-cache.txt", mac);
        put(&fl, "Гордунни/Bbb/macros-cache.txt", mac);
        let s = scan(std::slice::from_ref(&fl));
        assert_eq!(s.checked, 6);
        // Account: alias (may not work for Aaa) + deprecated; Aaa: both; Bbb: deprecated only.
        let scopes: Vec<&str> = s.findings.iter().map(|f| f.scope.as_str()).collect();
        assert_eq!(scopes.iter().filter(|x| **x == "Aaa-Tarren Mill").count(), 2);
        assert_eq!(scopes.iter().filter(|x| **x == "Bbb-Гордунни").count(), 1);
        let alias = s.findings.iter().find(|f| matches!(&f.kind, Kind::Alias { firm: false, .. })).unwrap();
        assert!(alias.problem().contains("Aaa-Tarren Mill (enUS)"));
        assert!(s.findings.iter().any(|f| matches!(&f.kind, Kind::Alias { firm: true, .. })));
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
        let s = scan(std::slice::from_ref(&fl));
        // The account's and Ccc's file; Gone's folder is left alone.
        assert_eq!(s.checked, 2);
        assert!(s.findings.is_empty());
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
        };
        let found = check(&m(&["/закл Heal"]), true, &[guessed, target("Bbb-Nowhere", None)], false);
        let Kind::Alias { bad, unknown, firm: false, .. } = &found[0].1 else {
            panic!("{found:?}")
        };
        assert_eq!(unknown, &["Bbb-Nowhere"]);
        assert!(bad[0].starts_with("Aaa-Нигде (deDE, ") && bad[0] != "Aaa-Нигде (deDE)");
    }
}
