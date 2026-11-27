//! Macro check: reads every `macros-cache.txt` (account-wide and per character) and reports what is
//! broken: by checks that need no downloaded data, and by spell/item names against the cached name data
//! (`macro_names`) when it is there. Read-only over WTF.
//!
//! File format: `VER 3 <16 hex id> "<name>" "<icon>"`, the body lines, `END`. Lines of a body that
//! start with neither `/` nor `#` are addon data (HealBot `hbCrashProt_*`, `!dosmarkers`) and ignored.

use crate::chars::{Character, subdirs};
use crate::macro_fix::{self, MacroFix};
use crate::macro_names::{self, Names};
use crate::profiles::{Roster, last_played, realm_key};
use crate::wow::Flavor;
use serde::{Deserialize, Serialize};
use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, OnceLock};

const REALMS: &str = include_str!("../data/realms.tsv");
const DEPRECATED: &str = include_str!("../data/deprecated-api.tsv");
const ALIASES: &str = include_str!("../data/slash-aliases.tsv");

/// The game's limit of a macro's text.
pub const MAX_LEN: usize = 255;

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

/// (function, replacement, removed in patch, source, `drop-in` or empty).
fn deprecated() -> &'static [[&'static str; 5]] {
    static T: OnceLock<Vec<[&'static str; 5]>> = OnceLock::new();
    T.get_or_init(|| {
        rows(DEPRECATED)
            .filter(|r| r.len() >= 4)
            .map(|r| [r[0], r[1], r[2], r[3], r.get(4).copied().unwrap_or_default()])
            .collect()
    })
}

/// What a removed function is called now, when the new one takes the same arguments and returns the same values.
pub fn drop_in(func: &str) -> Option<&'static str> {
    deprecated().iter().find(|r| r[0] == func && r[4] == "drop-in").map(|r| r[1])
}

/// A removed function that returns a table now: (replacement, the table field that holds the old first value, the
/// argument counts the old call had). The other old values are not mapped, so only a call using one value is rewritten.
pub fn first_value(func: &str) -> Option<(&'static str, &'static str, std::ops::RangeInclusive<usize>)> {
    let r = deprecated().iter().find(|r| r[0] == func)?;
    let mut it = r[4].strip_prefix("first:")?.split(':');
    let field = it.next()?;
    let (lo, hi) = it.next()?.split_once('-')?;
    Some((r[1], field, lo.parse().ok()?..=hi.parse().ok()?))
}

/// A removed function whose replacement is the same call only with one argument (the old one also took `(slot, bookType)`).
pub fn one_arg(func: &str) -> Option<&'static str> {
    deprecated().iter().find(|r| r[0] == func && r[4] == "one-arg").map(|r| r[1])
}

/// The client languages that know the slash alias `cmd` (empty: not an alias).
pub fn alias_locales(cmd: &str) -> Vec<&'static str> {
    aliases().iter().filter(|a| a[2] == cmd).map(|a| a[1]).collect()
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
    /// The text language of the player's own client (`Config.wtf`), as it is, not widened by `client`.
    pub client_main: Option<&'static str>,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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

/// What a check found.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Kind {
    Deprecated {
        func: String,
        replacement: String,
        patch: String,
        url: String,
    },
    TooLong {
        chars: usize,
    },
    Brackets,
    /// `bad`: Russian realms (`firm`: a character's own macro, else it may not work there); `soft`: other realms,
    /// where it works with a client in that language; neither: realms of unknown language.
    Alias {
        alias: String,
        command: String,
        locales: String,
        bad: Vec<String>,
        soft: Vec<String>,
        firm: bool,
    },
    Secret {
        func: String,
    },
    /// A name of the language `lang`; `fix` is what the realm's language calls it.
    WrongLanguage {
        name: String,
        lang: String,
        fix: String,
        how: How,
    },
    UnknownName {
        name: String,
    },
}

/// How bad a finding is: it does not work, it may not work, or it is only for information.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Level {
    Broken,
    Warn,
    Info,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Broken => tr!("macros.level_broken"),
            Level::Warn => tr!("macros.level_warn"),
            Level::Info => tr!("macros.level_info"),
        }
    }
}

/// A client language as its name in the interface language (`deDE` -> German); an unknown code stays.
pub fn lang_name(code: &str) -> String {
    match code {
        "enUS" => tr!("game_lang.enUS"),
        "deDE" => tr!("game_lang.deDE"),
        "esES" => tr!("game_lang.esES"),
        "esMX" => tr!("game_lang.esMX"),
        "frFR" => tr!("game_lang.frFR"),
        "itIT" => tr!("game_lang.itIT"),
        "koKR" => tr!("game_lang.koKR"),
        "ptBR" => tr!("game_lang.ptBR"),
        "ruRU" => tr!("game_lang.ruRU"),
        "zhCN" => tr!("game_lang.zhCN"),
        "zhTW" => tr!("game_lang.zhTW"),
        _ => code,
    }
    .to_string()
}

/// The same for a sentence "... in German".
fn lang_in(code: &str) -> String {
    match code {
        "enUS" => tr!("game_lang.in_enUS"),
        "deDE" => tr!("game_lang.in_deDE"),
        "esES" => tr!("game_lang.in_esES"),
        "esMX" => tr!("game_lang.in_esMX"),
        "frFR" => tr!("game_lang.in_frFR"),
        "itIT" => tr!("game_lang.in_itIT"),
        "koKR" => tr!("game_lang.in_koKR"),
        "ptBR" => tr!("game_lang.in_ptBR"),
        "ruRU" => tr!("game_lang.in_ruRU"),
        "zhCN" => tr!("game_lang.in_zhCN"),
        "zhTW" => tr!("game_lang.in_zhTW"),
        _ => code,
    }
    .to_string()
}

/// `deDE/ruRU` as the names joined by " / ".
pub fn lang_names(codes: &str, name: fn(&str) -> String) -> String {
    codes.split('/').map(name).collect::<Vec<_>>().join(" / ")
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
        // `..` is concatenation, not a field access
        let field = before.is_some_and(|c| c == ':' || (c == '.' && !line[..at - 1].ends_with('.')));
        !before.is_some_and(ident) && !field && !after.is_some_and(ident)
    })
}

/// Every `[` of the line is closed and no `]` comes first.
pub fn balanced(line: &str) -> bool {
    let mut depth = 0i32;
    line.chars().all(|c| {
        depth += i32::from(c == '[') - i32::from(c == ']');
        depth >= 0
    }) && depth == 0
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
                for [func, replacement, patch, url, _] in deprecated().iter().copied() {
                    if find_word(code, func) {
                        out.push((
                            i,
                            Kind::Deprecated {
                                func: func.into(),
                                replacement: replacement.into(),
                                patch: patch.into(),
                                url: url.into(),
                            },
                        ));
                    }
                }
                if let Some(func) = SECRET.iter().find(|f| find_word(code, f)) {
                    out.push((i, Kind::Secret { func: func.to_string() }));
                }
            }
            continue;
        }
        if !balanced(line) {
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
            let (mut bad, mut soft, mut unknown) = (Vec::new(), Vec::new(), 0);
            for t in targets.iter().filter(|t| !t.valid().iter().any(|c| locales.contains(c))) {
                // A realm of unknown language is never called bad: it may well be the alias's language.
                match (t.ru(), t.loc.is_some()) {
                    (true, _) => bad.push(label(t)),
                    (_, true) => soft.push(label(t)),
                    _ => unknown += 1,
                }
            }
            let firm = firm && !bad.is_empty();
            if !bad.is_empty() || !soft.is_empty() || unknown > 0 {
                out.push((
                    i,
                    Kind::Alias {
                        alias: cmd,
                        command: first[0].into(),
                        locales: locales.join("/"),
                        bad,
                        soft,
                        firm,
                    },
                ));
            }
        }
    }
    out
}

/// Whose macro it is, as plain names for a shell that does not show `Finding::scope` (which is text in the
/// interface language). Not stored with the report: a report read back from disk has it empty.
#[derive(Debug, Clone, Default)]
pub struct Owner {
    /// The flavor's folder name (`_retail_`); `Finding::flavor` is its title.
    pub flavor: String,
    pub account: String,
    /// Empty for an account-wide macro.
    pub realm: String,
    pub character: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub flavor: String,
    /// "Account X" or "Name-Realm".
    pub scope: String,
    /// From the account's own macros (listed before its characters').
    pub account: bool,
    pub name: String,
    pub line: usize,
    pub kind: Kind,
    #[serde(skip)]
    pub owner: Owner,
    /// The whole macro corrected, the same for all findings of one macro (shown with the first, see `first_fixes`).
    /// Not stored with the finding: `macro_store` keeps the texts once and links them back.
    #[serde(skip)]
    pub fixed: Option<Arc<MacroFix>>,
}

/// Per finding, the fixed text of its macro when it is the first finding of that macro (so a text is shown once).
pub fn first_fixes(findings: &[Finding]) -> Vec<Option<&Arc<MacroFix>>> {
    let mut seen: Vec<&Arc<MacroFix>> = Vec::new();
    findings
        .iter()
        .map(|f| {
            let fix = f.fixed.as_ref()?;
            if seen.iter().any(|s| Arc::ptr_eq(s, fix)) {
                return None;
            }
            seen.push(fix);
            Some(fix)
        })
        .collect()
}

impl Finding {
    pub fn problem(&self) -> String {
        self.problem_marked().0
    }

    /// The problem text and the byte ranges of its offending parts (the name, function or alias), to be highlighted.
    pub fn problem_marked(&self) -> (String, Vec<Range<usize>>) {
        let m = |s: &str| format!("\u{1}{s}\u{2}");
        let text = match &self.kind {
            Kind::Deprecated { func, patch, .. } => tr!("macros.deprecated", func = m(func), patch = patch),
            Kind::TooLong { chars } => tr!("macros.too_long", n = chars, max = MAX_LEN),
            Kind::Brackets => tr!("macros.brackets").to_string(),
            Kind::Alias {
                alias,
                command,
                locales,
                bad,
                soft,
                firm,
                ..
            } => {
                let (alias, locale) = (m(alias), lang_names(locales, lang_name));
                match (bad.is_empty(), soft.is_empty()) {
                    (false, _) if *firm => tr!("macros.alias_firm", alias = alias, command = command, locale = locale),
                    (false, _) => tr!("macros.alias_maybe", alias = alias, command = command, locale = locale),
                    (true, false) => tr!("macros.alias_soft", alias = alias, command = command, locale = locale),
                    (true, true) => tr!("macros.alias_unknown", alias = alias, command = command, locale = locale),
                }
            }
            Kind::Secret { func } => tr!("macros.secret", func = m(func)),
            Kind::WrongLanguage { name, lang, how, .. } => {
                let (name, lang) = (m(name), lang_in(lang));
                match how {
                    How::Firm => tr!("macros.wrong_firm", name = name, lang = lang),
                    How::Maybe => tr!("macros.wrong_maybe", name = name, lang = lang),
                    How::Client => tr!("macros.wrong_soft", name = name, lang = lang),
                    How::Realm => tr!("macros.wrong_realm", name = name, lang = lang),
                }
            }
            Kind::UnknownName { name } => tr!("macros.unknown_name", name = m(name)),
        };
        let (mut out, mut marks, mut from) = (String::new(), Vec::new(), 0);
        for c in text.chars() {
            match c {
                '\u{1}' => from = out.len(),
                '\u{2}' => marks.push(from..out.len()),
                c => out.push(c),
            }
        }
        (out, marks)
    }

    /// Broken: fails for sure (removed API, too long, unbalanced brackets, a name nobody knows, or a name/command the
    /// Russian realm of the character's own macro cannot read). Warn: may fail (an account-wide macro on a Russian realm,
    /// works only with a client in another language, values hidden in combat). Info: the realm's language is unknown.
    pub fn level(&self) -> Level {
        match &self.kind {
            Kind::Deprecated { .. } | Kind::TooLong { .. } | Kind::Brackets | Kind::UnknownName { .. } => Level::Broken,
            Kind::Secret { .. } => Level::Warn,
            Kind::WrongLanguage { how, .. } => match how {
                How::Firm => Level::Broken,
                How::Maybe | How::Client => Level::Warn,
                How::Realm => Level::Info,
            },
            Kind::Alias { bad, soft, firm, .. } => match (bad.is_empty(), soft.is_empty()) {
                (false, _) if *firm => Level::Broken,
                (true, true) => Level::Info,
                _ => Level::Warn,
            },
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

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Scan {
    pub findings: Vec<Finding>,
    /// Macros read.
    pub checked: usize,
    /// Per product, the realm languages of the characters found (what `--update-names` downloads).
    #[serde(skip)]
    pub locales: BTreeMap<String, BTreeSet<&'static str>>,
    /// Flavors with macros whose realm languages have no name data yet.
    pub missing: Vec<String>,
}

impl Scan {
    /// How many findings there are of each level that occurs, worst first.
    pub fn levels(&self) -> Vec<(Level, usize)> {
        [Level::Broken, Level::Warn, Level::Info]
            .into_iter()
            .map(|l| (l, self.findings.iter().filter(|f| f.level() == l).count()))
            .filter(|&(_, n)| n > 0)
            .collect()
    }

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
    walk(flavors, Some(names_dir))
}

/// Per product, the realm languages the macros of `flavors` are judged by, without reading macros or name tables.
pub fn needs(flavors: &[Flavor]) -> BTreeMap<String, BTreeSet<&'static str>> {
    walk(flavors, None).locales
}

/// `names_dir` None: only collect `locales` (no macro is read, no name table loaded).
fn walk(flavors: &[Flavor], names_dir: Option<&Path>) -> Scan {
    let mut scan = Scan::default();
    for fl in flavors {
        let wtf = fl.wtf_dir();
        let region = portal(&wtf);
        let retail = is_retail(&fl.name);
        let product = fl.product();
        let names = match (product.as_deref(), names_dir) {
            (Some(p), Some(dir)) => Names::load(dir, p),
            _ => Default::default(),
        };
        let client = client_locale(&wtf);
        let (checked_before, mut seen) = (scan.checked, BTreeSet::new());
        let mut check_file = |path: &Path, scope: &str, owner: Owner, targets: &[Target], firm: bool| {
            if names_dir.is_none() {
                return;
            }
            let Ok(bytes) = fs::read(path) else { return };
            for m in parse(&String::from_utf8_lossy(&bytes)) {
                scan.checked += 1;
                let mut found = check(&m, retail, targets, firm);
                found.extend(macro_names::check(&m, targets, firm, &names));
                found.sort_by_key(|(line, _)| *line);
                let fixed = if found.is_empty() {
                    None
                } else {
                    macro_fix::build(&m, retail, targets, firm, &names).map(Arc::new)
                };
                for (line, kind) in found {
                    scan.findings.push(Finding {
                        flavor: fl.title(),
                        scope: scope.to_string(),
                        account: !firm,
                        name: m.name.clone(),
                        line,
                        kind,
                        owner: owner.clone(),
                        fixed: fixed.clone(),
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
                    let owner = Owner {
                        flavor: fl.name.clone(),
                        account: account.clone(),
                        realm: realm.clone(),
                        character: name.clone(),
                    };
                    chars.push((
                        Target {
                            label: format!("{name}-{realm}"),
                            loc,
                            client_main: client,
                            client: client.into_iter().collect(),
                        },
                        dir,
                        owner,
                    ));
                }
            }
            // Russian realms on the account: its other characters may well run a Russian client.
            if chars.iter().any(|(t, _, _)| t.ru()) {
                chars.iter_mut().for_each(|(t, _, _)| t.client.push("ruRU"));
            }
            seen.extend(chars.iter().flat_map(|(t, _, _)| t.valid()));
            let all: Vec<Target> = chars.iter().map(|(t, _, _)| t.clone()).collect();
            let account_owner = Owner {
                flavor: fl.name.clone(),
                account: account.clone(),
                ..Owner::default()
            };
            check_file(
                &acc_dir.join("macros-cache.txt"),
                &tr!("macros.account", account = account),
                account_owner,
                &all,
                false,
            );
            for (target, dir, owner) in &chars {
                check_file(&dir.join("macros-cache.txt"), &target.label, owner.clone(), std::slice::from_ref(target), true);
            }
        }
        if let Some(product) = product {
            scan.locales.entry(product).or_default().extend(&seen);
            if scan.checked > checked_before && seen.iter().any(|l| !names.has(l)) {
                scan.missing.push(fl.title());
            }
        }
    }
    // Worst first, then by macro, so the panel report, the clipboard and the console list the same way.
    sort_findings(&mut scan.findings);
    scan
}

pub fn sort_findings(findings: &mut [Finding]) {
    let key = |f: &Finding| (f.level(), f.flavor.clone(), !f.account, f.scope.clone(), f.name.clone(), f.line);
    findings.sort_by_key(key);
}

pub fn report(scan: &Scan) -> String {
    let mut out = String::new();
    let mut head: Option<(Level, &str, &str)> = None;
    for (f, fix) in scan.findings.iter().zip(first_fixes(&scan.findings)) {
        if head != Some((f.level(), &f.flavor, &f.scope)) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("[{}] {} / {}\n", f.level().label(), f.flavor, f.scope));
            head = Some((f.level(), &f.flavor, &f.scope));
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
        if let Some(fix) = fix {
            out.push_str(&macro_fix::render(fix));
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
            client_main: None,
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
        assert_eq!((soft.as_slice(), command.as_str()), (&["B-En (enUS)".to_string()][..], "/cast"));
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
        assert!(matches!(&f[0].1, Kind::Alias { bad, soft, .. } if bad.is_empty() && soft.is_empty()));
        let unknown = check(&mac, true, &[target("C-Nowhere", None)], true);
        assert!(matches!(&unknown[0].1, Kind::Alias { bad, soft, .. } if bad.is_empty() && soft.is_empty()));
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
        assert_eq!(needs(std::slice::from_ref(&fl)), s.locales);
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
        for (entry, text) in [("spell", spell), ("item-sparse", "")] {
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
        // The fixed text: the character's own macro in its realm's language; the account's only for the Russian realm.
        let own = s.findings.iter().find(|f| f.scope == "Bbb-Гордунни").and_then(|f| f.fixed.as_ref()).unwrap();
        assert_eq!((own.variants[0].lang.as_str(), own.variants[0].text.as_str()), ("ruRU", "/cast Огненный шар"));
        let account = s.findings.iter().find(|f| f.account).and_then(|f| f.fixed.as_ref()).unwrap();
        assert_eq!(account.variants.len(), 1);
        assert!(report(&s).contains("        /cast Огненный шар\n"));
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
        // the card names no character; the data still knows them
        let Kind::Alias { soft, .. } = &alias.kind else { panic!() };
        assert!(soft.contains(&"Aaa-Tarren Mill (enUS)".to_string()) && !alias.problem().contains("Aaa-Tarren Mill"));
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
        let Kind::Alias { soft, bad, .. } = &alias.kind else { panic!() };
        assert!(soft.iter().chain(bad).any(|c| c.contains("Aaa-Tarren Mill")) && !soft.iter().chain(bad).any(|c| c.contains("Gone")));
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
    fn guessed_language_is_marked_in_the_stored_labels() {
        let guessed = Target {
            label: "Aaa-Нигде".into(),
            loc: Some(Loc { code: "deDE", guessed: true }),
            client_main: None,
            client: Vec::new(),
        };
        let found = check(&m(&["/закл Heal"]), true, &[guessed, target("Bbb-Nowhere", None)], false);
        let Kind::Alias { soft: bad, firm: false, .. } = &found[0].1 else {
            panic!("{found:?}")
        };
        assert!(bad[0].starts_with("Aaa-Нигде (deDE, ") && bad[0] != "Aaa-Нигде (deDE)");
    }

    fn finding(kind: Kind) -> Finding {
        Finding {
            flavor: "Retail".into(),
            scope: "s".into(),
            account: false,
            name: "n".into(),
            line: 1,
            kind,
            owner: Owner::default(),
            fixed: None,
        }
    }

    fn alias(bad: &[&str], soft: &[&str], firm: bool) -> Kind {
        let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect();
        Kind::Alias {
            alias: "/a".into(),
            command: "/cast".into(),
            locales: "deDE".into(),
            bad: v(bad),
            soft: v(soft),
            firm,
        }
    }

    fn wrong(how: How) -> Kind {
        Kind::WrongLanguage {
            name: "a".into(),
            lang: "deDE".into(),
            fix: "b".into(),
            how,
        }
    }

    #[test]
    fn level_of_every_kind() {
        let deprecated = Kind::Deprecated {
            func: "f".into(),
            replacement: "g".into(),
            patch: "1".into(),
            url: "u".into(),
        };
        let level = |k| finding(k).level();
        assert_eq!(level(deprecated), Level::Broken);
        assert_eq!(level(Kind::TooLong { chars: 300 }), Level::Broken);
        assert_eq!(level(Kind::Brackets), Level::Broken);
        assert_eq!(level(Kind::UnknownName { name: "x".into() }), Level::Broken);
        assert_eq!(level(Kind::Secret { func: "f".into() }), Level::Warn);
        assert_eq!(level(wrong(How::Firm)), Level::Broken);
        assert_eq!(level(wrong(How::Maybe)), Level::Warn);
        assert_eq!(level(wrong(How::Client)), Level::Warn);
        assert_eq!(level(wrong(How::Realm)), Level::Info);
        assert_eq!(level(alias(&["a"], &[], true)), Level::Broken);
        assert_eq!(level(alias(&["a"], &["b"], true)), Level::Broken);
        assert_eq!(level(alias(&["a"], &[], false)), Level::Warn);
        assert_eq!(level(alias(&[], &["b"], false)), Level::Warn);
        assert_eq!(level(alias(&[], &[], false)), Level::Info);
    }

    #[test]
    fn findings_sort_by_level_then_macro_and_the_report_marks_levels() {
        let at = |level_kind: Kind, name: &str, line: usize| Finding {
            name: name.into(),
            line,
            ..finding(level_kind)
        };
        let mut scan = Scan {
            findings: vec![
                at(wrong(How::Realm), "B", 1),
                at(Kind::Brackets, "B", 5),
                at(Kind::Secret { func: "f".into() }, "A", 1),
                at(Kind::Brackets, "B", 2),
                at(Kind::Brackets, "A", 9),
            ],
            ..Default::default()
        };
        sort_findings(&mut scan.findings);
        let order: Vec<_> = scan.findings.iter().map(|f| (f.level(), f.name.as_str(), f.line)).collect();
        assert_eq!(
            order,
            [
                (Level::Broken, "A", 9),
                (Level::Broken, "B", 2),
                (Level::Broken, "B", 5),
                (Level::Warn, "A", 1),
                (Level::Info, "B", 1)
            ]
        );
        assert_eq!(scan.levels(), [(Level::Broken, 3), (Level::Warn, 1), (Level::Info, 1)]);
        scan.findings.retain(|f| f.level() != Level::Warn);
        assert_eq!(scan.levels(), [(Level::Broken, 3), (Level::Info, 1)]);
        let report = report(&scan);
        assert!(report.find(Level::Broken.label()).unwrap() < report.find(Level::Info.label()).unwrap());
        assert!(!report.contains(Level::Warn.label()));
    }

    #[test]
    fn report_groups_by_level_then_flavor_and_scope() {
        let at = |kind: Kind, flavor: &str, scope: &str| Finding {
            flavor: flavor.into(),
            scope: scope.into(),
            ..finding(kind)
        };
        let mut scan = Scan {
            findings: vec![
                at(Kind::Secret { func: "f".into() }, "Era", "s"),
                at(Kind::Brackets, "Retail", "b"),
                at(Kind::Brackets, "Era", "s"),
                at(Kind::Brackets, "Retail", "a"),
                Finding {
                    account: true,
                    ..at(Kind::Brackets, "Retail", "z")
                },
            ],
            ..Default::default()
        };
        sort_findings(&mut scan.findings);
        let heads: Vec<_> = report(&scan).lines().filter(|l| l.starts_with('[')).map(String::from).collect();
        let (broken, warn) = (Level::Broken.label(), Level::Warn.label());
        assert_eq!(
            heads,
            [
                format!("[{broken}] Era / s"),
                format!("[{broken}] Retail / z"),
                format!("[{broken}] Retail / a"),
                format!("[{broken}] Retail / b"),
                format!("[{warn}] Era / s")
            ]
        );
    }

    #[test]
    fn the_removed_functions_table_has_five_columns_and_a_verified_drop_in_set() {
        assert!(
            deprecated()
                .iter()
                .all(|r| r[3].starts_with("https://") && matches!(r[4], "" | "drop-in" | "one-arg") || r[4].starts_with("first:"))
        );
        assert_eq!(drop_in("GetItemCount"), Some("C_Item.GetItemCount"));
        assert_eq!(drop_in("GetItemIcon"), Some("C_Item.GetItemIconByID"));
        // tables, other arguments or unverified: never replaced
        for f in [
            "GetSpellInfo",
            "GetContainerItemInfo",
            "UnitAura",
            "UseContainerItem",
            "GetMouseFocus",
            "GetAddOnInfo",
            "NoSuchFunction",
        ] {
            assert_eq!(drop_in(f), None, "{f}");
        }
        // the old book-slot form (slot, bookType) of these two is read as a spell id by C_Spell
        assert_eq!((drop_in("GetSpellTexture"), drop_in("IsUsableSpell")), (None, None));
        assert_eq!(deprecated().iter().filter(|r| r[4] == "drop-in").count(), 14);
        // concatenation is no field access
        assert!(find_word("print(\"n: \"..GetItemCount(5))", "GetItemCount"));
        assert!(!find_word("print(C_Item.GetItemCount(5))", "GetItemCount"));
    }

    #[test]
    fn the_problem_marks_the_offending_part_and_names_no_character() {
        let cases = [
            (Kind::UnknownName { name: "Heroik Strike".into() }, "Heroik Strike"),
            (
                Kind::WrongLanguage {
                    name: "Огненный шар".into(),
                    lang: "ruRU".into(),
                    fix: "Fireball".into(),
                    how: How::Firm,
                },
                "Огненный шар",
            ),
            (
                Kind::Deprecated {
                    func: "GetSpellInfo".into(),
                    replacement: "C_Spell.GetSpellInfo".into(),
                    patch: "12.0.0".into(),
                    url: "u".into(),
                },
                "GetSpellInfo",
            ),
            (Kind::Secret { func: "UnitHealth".into() }, "UnitHealth"),
            (alias(&["Secret-Realm (ruRU)"], &["Other-Realm (enUS)"], true), "/a"),
            (alias(&[], &["Other-Realm (enUS)"], false), "/a"),
            (alias(&[], &[], false), "/a"),
        ];
        for (kind, token) in cases {
            let (text, marks) = finding(kind).problem_marked();
            assert_eq!(marks.iter().map(|m| &text[m.clone()]).collect::<Vec<_>>(), [token], "{text}");
            assert!(!text.contains("Realm") && !text.contains("ruRU)"), "{text}");
        }
        // a problem without an offending part has no marks, and `problem` is the same text
        let f = finding(Kind::Brackets);
        assert_eq!(f.problem_marked(), (f.problem(), Vec::new()));
        // every client language has a name; an unknown code stays as it is
        for code in CLIENT_LOCALES {
            assert_ne!(lang_name(code), code);
        }
        assert_eq!(
            (lang_name("xxXX"), lang_names("deDE/ruRU", lang_name)),
            ("xxXX".to_string(), format!("{} / {}", lang_name("deDE"), lang_name("ruRU")))
        );
    }

    #[test]
    fn the_removed_function_table_maps_the_first_value_and_the_one_argument_functions() {
        assert_eq!(first_value("GetSpellInfo"), Some(("C_Spell.GetSpellInfo", "name", 1..=1)));
        assert_eq!(first_value("UnitBuff"), Some(("C_UnitAuras.GetBuffDataByIndex", "name", 2..=3)));
        assert_eq!(
            first_value("GetContainerItemInfo"),
            Some(("C_Container.GetContainerItemInfo", "iconFileID", 2..=2))
        );
        assert_eq!(first_value("GetItemCount"), None);
        assert_eq!(one_arg("IsUsableSpell"), Some("C_Spell.IsSpellUsable"));
        assert_eq!(one_arg("GetSpellTexture"), Some("C_Spell.GetSpellTexture"));
        assert_eq!(one_arg("GetSpellInfo"), None);
        // the three kinds of compat never overlap
        for r in deprecated() {
            let kinds = [r[4] == "drop-in", r[4] == "one-arg", first_value(r[0]).is_some()];
            assert!(kinds.iter().filter(|k| **k).count() <= 1, "{}", r[0]);
        }
    }

    #[test]
    fn a_macro_shows_its_fixed_text_once() {
        let fix = Arc::new(MacroFix { variants: Vec::new() });
        let mut a = finding(Kind::Brackets);
        a.fixed = Some(fix.clone());
        let mut b = finding(Kind::Brackets);
        b.fixed = Some(fix);
        let mut other = finding(Kind::Brackets);
        other.fixed = Some(Arc::new(MacroFix { variants: Vec::new() }));
        let all = [a, finding(Kind::Brackets), b, other];
        let shown = first_fixes(&all);
        assert_eq!(shown.iter().map(Option::is_some).collect::<Vec<_>>(), [true, false, false, true]);
    }

    #[test]
    fn counts_group_findings_by_kind_most_first() {
        let f = |kind| Finding {
            flavor: "Retail".into(),
            scope: "s".into(),
            account: false,
            name: "n".into(),
            line: 1,
            kind,
            owner: Owner::default(),
            fixed: None,
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
