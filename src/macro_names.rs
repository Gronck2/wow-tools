//! Spell and item names per locale for the macro check: downloaded from wago.tools on request
//! (`wow-tools macros --update-names`, never by itself), cached in `data\names\<product>-<locale>.zip` and
//! used offline. A cache is current while `<product>-<locale>.meta` holds the build wago.tools calls the latest.
//!
//! shortcut: the key is the build of `/api/builds/<product>/latest`; the CSV can come from a slightly newer build,
//! which is harmless for names; upgrade only if names ever lag behind a patch.

use crate::macros::{Kind, Macro, Target, canonical_command};
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const ENUS: &str = "enUS";
/// A real translation is never this identical to English; a wrong locale silently returns English. Classic Era
/// keeps English for rows it never translated, so the limit is high, not "most rows".
const SAME_MAX_PERCENT: usize = 98;
/// (wago.tools table, entry name in the cache).
const TABLES: [(&str, &str); 2] = [("SpellName", "spell"), ("ItemSearchName", "item")];
const NAME_COMMANDS: &[&str] = &["/cast", "/use", "/castsequence", "/castrandom", "/userandom", "#showtooltip", "#show"];

type Rows = Vec<(u32, String)>;

/// `ID,Name_lang,...` of a wago.tools export; only the first two columns are read. Fields may be quoted
/// (`""` is a quote) and a quoted field may span lines. Rows without a number and a name are dropped.
fn parse_csv(text: &str) -> Rows {
    let mut out = Vec::new();
    let mut rec = String::new();
    for line in text.lines() {
        if !rec.is_empty() {
            rec.push('\n');
        }
        rec.push_str(line);
        if rec.matches('"').count() % 2 == 1 {
            continue;
        }
        if let Some((id, rest)) = rec.split_once(',')
            && let Ok(id) = id.parse::<u32>()
        {
            let name = match rest.strip_prefix('"') {
                Some(q) => {
                    let mut s = String::new();
                    let mut it = q.chars().peekable();
                    while let Some(c) = it.next() {
                        if c == '"' {
                            if it.peek() == Some(&'"') {
                                it.next();
                            } else {
                                break;
                            }
                        }
                        s.push(c);
                    }
                    s
                }
                None => rest.split(',').next().unwrap_or_default().to_string(),
            };
            let name = name.replace(['\t', '\n', '\r'], " ");
            if !name.trim().is_empty() {
                out.push((id, name));
            }
        }
        rec.clear();
    }
    out
}

#[derive(Default)]
struct Table {
    by_name: HashMap<String, Vec<u32>>,
    by_id: HashMap<u32, String>,
}

impl Table {
    fn new(rows: Rows) -> Table {
        let mut t = Table::default();
        for (id, name) in rows {
            t.by_name.entry(name.trim().to_lowercase()).or_default().push(id);
            t.by_id.entry(id).or_insert(name);
        }
        t
    }

    /// The name of the first of `ids` that this table has.
    fn name_of(&self, ids: Option<&Vec<u32>>) -> Option<&String> {
        ids?.iter().find_map(|id| self.by_id.get(id))
    }
}

#[derive(Default)]
struct Locale {
    spell: Table,
    item: Table,
}

impl Locale {
    fn has(&self, lc: &str) -> bool {
        self.spell.by_name.contains_key(lc) || self.item.by_name.contains_key(lc)
    }

    /// What this locale calls what `other` calls `lc` (a spell first, then an item).
    fn translate(&self, other: &Locale, lc: &str) -> Option<String> {
        self.spell
            .name_of(other.spell.by_name.get(lc))
            .or_else(|| self.item.name_of(other.item.by_name.get(lc)))
            .cloned()
    }
}

/// The cached names of one product, by locale.
#[derive(Default)]
pub struct Names {
    locales: BTreeMap<String, Locale>,
}

impl Names {
    /// Every locale cached for `product`; none when the folder or a file is missing or broken.
    pub fn load(dir: &Path, product: &str) -> Names {
        let mut names = Names::default();
        let prefix = format!("{product}-");
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let file = e.file_name().to_string_lossy().into_owned();
            if let Some(locale) = file.strip_prefix(&prefix).and_then(|f| f.strip_suffix(".zip"))
                && let Ok([spell, item]) = read_cache(dir, product, locale)
            {
                names.locales.insert(
                    locale.to_string(),
                    Locale {
                        spell: Table::new(spell),
                        item: Table::new(item),
                    },
                );
            }
        }
        names
    }

    pub fn has(&self, locale: &str) -> bool {
        self.locales.contains_key(locale)
    }
}

fn cache_path(dir: &Path, product: &str, locale: &str, ext: &str) -> PathBuf {
    dir.join(format!("{product}-{locale}.{ext}"))
}

fn read_cache(dir: &Path, product: &str, locale: &str) -> Result<[Rows; 2]> {
    let mut zip = ZipArchive::new(fs::File::open(cache_path(dir, product, locale, "zip"))?)?;
    let mut read = |entry: &str| -> Result<Rows> {
        let mut text = String::new();
        zip.by_name(entry)?.read_to_string(&mut text)?;
        Ok(text
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .filter_map(|(id, name)| Some((id.parse().ok()?, name.to_string())))
            .collect())
    };
    Ok([read(TABLES[0].1)?, read(TABLES[1].1)?])
}

/// Writes the zip through a temp file; the meta goes last, so a cut write never looks current.
fn write_cache(dir: &Path, product: &str, locale: &str, build: &str, tables: &[Rows; 2]) -> Result<()> {
    fs::create_dir_all(dir)?;
    let tmp = cache_path(dir, product, locale, "tmp");
    let mut zip = ZipWriter::new(fs::File::create(&tmp)?);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for ((_, entry), rows) in TABLES.iter().zip(tables) {
        zip.start_file(*entry, opts)?;
        let mut text = String::new();
        for (id, name) in rows {
            text.push_str(&format!("{id}\t{name}\n"));
        }
        zip.write_all(text.as_bytes())?;
    }
    zip.finish()?;
    fs::rename(&tmp, cache_path(dir, product, locale, "zip"))?;
    fs::write(&tmp, build)?;
    fs::rename(&tmp, cache_path(dir, product, locale, "meta"))?;
    Ok(())
}

/// Rejects a table that is empty, or English although another language was asked for.
fn validate(locale: &str, spell: &Rows, en: &Rows) -> Result<()> {
    if spell.is_empty() {
        bail!(tr!("macros.names_empty", locale = locale));
    }
    if locale != ENUS {
        let en: HashMap<u32, &String> = en.iter().map(|(id, n)| (*id, n)).collect();
        let same = spell.iter().filter(|(id, n)| en.get(id).is_some_and(|e| e.eq_ignore_ascii_case(n))).count();
        if same * 100 > spell.len() * SAME_MAX_PERCENT {
            bail!(tr!("macros.names_english", locale = locale));
        }
    }
    Ok(())
}

fn get(url: &str, accept: &str) -> Result<Vec<u8>> {
    let (status, body) = crate::selfupdate::http::get(url, Some(accept))?;
    if status != 200 {
        bail!(tr!("macros.names_http", url = url, status = status));
    }
    Ok(body)
}

fn latest_build(product: &str) -> Result<String> {
    let body = get(&format!("https://wago.tools/api/builds/{product}/latest"), "application/json")?;
    let v: serde_json::Value = serde_json::from_slice(&body)?;
    v["version"]
        .as_str()
        .map(str::to_string)
        .with_context(|| tr!("macros.names_no_build", product = product))
}

/// Downloads the names of `locale` unless the cache holds `build`. Returns whether it downloaded.
fn update_locale(dir: &Path, product: &str, locale: &str, build: &str, progress: &mut dyn FnMut(String)) -> Result<bool> {
    let meta = fs::read_to_string(cache_path(dir, product, locale, "meta")).unwrap_or_default();
    if meta.trim() == build && cache_path(dir, product, locale, "zip").exists() {
        return Ok(false);
    }
    progress(tr!("macros.names_download", product = product, locale = locale, build = build));
    let mut tables: [Rows; 2] = Default::default();
    for (slot, (table, _)) in tables.iter_mut().zip(TABLES) {
        let url = format!("https://wago.tools/db2/{table}/csv?product={product}&locale={locale}");
        *slot = parse_csv(&String::from_utf8_lossy(&get(&url, "text/csv")?));
    }
    // enUS is done first in `update`, so its cache is there to compare with.
    let en = if locale == ENUS {
        Rows::new()
    } else {
        read_cache(dir, product, ENUS)?.into_iter().next().unwrap_or_default()
    };
    validate(locale, &tables[0], &en)?;
    write_cache(dir, product, locale, build, &tables)?;
    Ok(true)
}

/// Brings the cache of every product in `needs` (product -> realm locales) up to date, enUS always included.
/// A failure of one locale does not stop the others; it is reported and counted.
pub fn update(dir: &Path, needs: &BTreeMap<&str, BTreeSet<&str>>, progress: &mut dyn FnMut(String)) -> Result<()> {
    let mut failed = 0;
    for (product, locales) in needs {
        let build = match latest_build(product) {
            Ok(b) => b,
            Err(e) => {
                progress(format!("{product}: {e:#}"));
                failed += 1;
                continue;
            }
        };
        for locale in std::iter::once(ENUS).chain(locales.iter().copied().filter(|l| *l != ENUS)) {
            match update_locale(dir, product, locale, &build, progress) {
                Ok(true) => progress(tr!("macros.names_done", product = product, locale = locale)),
                Ok(false) => progress(tr!("macros.names_current", product = product, locale = locale, build = build)),
                Err(e) => {
                    progress(format!("{product} {locale}: {e:#}"));
                    failed += 1;
                }
            }
        }
    }
    if failed > 0 {
        bail!(tr!("macros.names_failed", n = failed));
    }
    Ok(())
}

/// The spell and item names a macro line asks for: `/cast [@focus] !Name(Rank 1); Other`, castsequence lists,
/// `reset=...`. Slots, bags (`/use 0 1`) and `item:123` are not names.
fn command_names(line: &str) -> Vec<String> {
    let line = line.trim();
    let end = line.find(|c: char| c.is_whitespace() || c == '[').unwrap_or(line.len());
    let cmd = canonical_command(&line[..end].to_lowercase());
    if !NAME_COMMANDS.contains(&cmd.as_str()) {
        return Vec::new();
    }
    let mut text = String::new();
    let mut depth = 0usize;
    for c in line[end..].chars() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => text.push(c),
            _ => {}
        }
    }
    let list = matches!(cmd.as_str(), "/castsequence" | "/castrandom");
    let mut out = Vec::new();
    for clause in text.split(';') {
        let clause = clause.split_whitespace().filter(|w| !w.starts_with("reset=")).collect::<Vec<_>>().join(" ");
        let pieces: Vec<&str> = if list { clause.split(',').collect() } else { vec![&clause] };
        for piece in pieces {
            let mut name = piece.trim().trim_start_matches('!').trim();
            if let Some(i) = name.rfind('(')
                && name.ends_with(')')
                && name[i..].chars().any(|c| c.is_ascii_digit())
            {
                name = name[..i].trim_end();
            }
            let number = name.chars().all(|c| c.is_ascii_digit() || c.is_whitespace());
            if !name.is_empty() && !number && !name.to_lowercase().starts_with("item:") {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// Wrong-language and unknown names of one macro, as (body line from 1, kind). Targets whose realm language
/// is not cached are skipped (the caller says so).
pub fn check(m: &Macro, targets: &[Target], firm: bool, names: &Names) -> Vec<(usize, Kind)> {
    let ready: Vec<(&Target, &Locale)> = targets.iter().filter_map(|t| Some((t, names.locales.get(t.loc?.code)?))).collect();
    let mut out = Vec::new();
    if ready.is_empty() {
        return out;
    }
    for (i, line) in m.body.iter().enumerate() {
        for name in command_names(line) {
            let lc = name.to_lowercase();
            let mut unknown = false;
            // (language of the name, name that works on the realm) -> targets
            let mut wrong: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
            for (t, realm) in &ready {
                if realm.has(&lc) {
                    continue;
                }
                let fix = names
                    .locales
                    .iter()
                    .filter(|(_, l)| l.has(&lc))
                    .find_map(|(code, other)| Some((code.clone(), realm.translate(other, &lc)?)));
                match fix {
                    Some(key) => wrong.entry(key).or_default().push(t.label.clone()),
                    None => unknown = true,
                }
            }
            if unknown {
                out.push((i + 1, Kind::UnknownName { name: name.clone() }));
            }
            for ((lang, fix), bad) in wrong {
                out.push((
                    i + 1,
                    Kind::WrongLanguage {
                        name: name.clone(),
                        lang,
                        fix,
                        bad,
                        firm,
                    },
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macros::Loc;

    fn rows(r: &[(u32, &str)]) -> Rows {
        r.iter().map(|(i, n)| (*i, n.to_string())).collect()
    }

    fn locale(spell: &[(u32, &str)], item: &[(u32, &str)]) -> Locale {
        Locale {
            spell: Table::new(rows(spell)),
            item: Table::new(rows(item)),
        }
    }

    /// enUS, ruRU (spells 1 and 2, item 10) and deDE (spell 1 only).
    fn names() -> Names {
        let mut n = Names::default();
        n.locales
            .insert("enUS".into(), locale(&[(1, "Fireball"), (2, "Death and Decay")], &[(10, "Healing Potion")]));
        n.locales.insert(
            "ruRU".into(),
            locale(&[(1, "Огненный шар"), (2, "Смерть и разложение")], &[(10, "Лечебное зелье")]),
        );
        n.locales.insert("deDE".into(), locale(&[(1, "Feuerball")], &[]));
        n
    }

    fn target(label: &str, code: &'static str) -> Target {
        Target {
            label: label.into(),
            loc: Some(Loc { code, guessed: false }),
        }
    }

    fn mac(body: &[&str]) -> Macro {
        Macro {
            name: "T".into(),
            icon: "1".into(),
            body: body.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn csv_handles_quotes_commas_and_embedded_lines() {
        let text = "ID,Name_lang\n1,Fireball\n2,\"Hand of Gul'dan, the \"\"Big\"\"\"\n3,\"two\nlines\"\n4,\n5,\"\"\nx,Bad\n6,Last,extra,cols\n";
        assert_eq!(
            parse_csv(text),
            rows(&[(1, "Fireball"), (2, "Hand of Gul'dan, the \"Big\""), (3, "two lines"), (6, "Last")])
        );
    }

    #[test]
    fn extracts_names_from_commands() {
        let n = command_names;
        assert_eq!(n("/cast [@focus,harm] !Fireball(Rank 3); [help] Heal"), ["Fireball", "Heal"]);
        assert_eq!(n("#showtooltip Fireball"), ["Fireball"]);
        assert_eq!(n("/castsequence reset=5/target/shift A b, C d"), ["A b", "C d"]);
        assert_eq!(n("/cast Name With (OLD)"), ["Name With (OLD)"]);
        assert!(n("/use 13").is_empty() && n("/use 0 1").is_empty() && n("/use item:19019").is_empty());
        assert_eq!(n("/use [mod] Healing Potion"), ["Healing Potion"]);
        assert_eq!(n("/закл Огненный шар"), ["Огненный шар"]);
        assert_eq!(n("/cast Hand of Gul'dan, Big"), ["Hand of Gul'dan, Big"]);
        assert!(n("/stopmacro [combat]").is_empty() && n("/run x()").is_empty() && n("#showtooltip").is_empty());
    }

    #[test]
    fn verdicts_for_character_and_account_scope() {
        let names = names();
        let ru = target("A-Ru", "ruRU");
        let de = target("B-De", "deDE");
        let found = |body: &[&str], t: &[Target], firm| {
            check(&mac(body), t, firm, &names)
                .into_iter()
                .map(|(l, k)| (l, format!("{k:?}")))
                .collect::<Vec<_>>()
        };
        // ok
        assert!(found(&["/cast Огненный шар", "/use Лечебное зелье"], std::slice::from_ref(&ru), true).is_empty());
        // wrong language: an enUS name on a ruRU realm, with the fix
        let f = found(&["/cast fireball"], std::slice::from_ref(&ru), true);
        assert_eq!(f.len(), 1);
        assert!(f[0].1.contains("WrongLanguage") && f[0].1.contains("enUS") && f[0].1.contains("Огненный шар") && f[0].1.contains("firm: true"));
        // typo: unknown anywhere
        let f = found(&["/cast Огненый шар"], std::slice::from_ref(&ru), true);
        assert!(f.len() == 1 && f[0].1.contains("UnknownName"));
        // account scope: English works on neither realm
        let both = [ru.clone(), de.clone()];
        let f = found(&["/cast Fireball"], &both, false);
        assert_eq!(f.len(), 2);
        assert!(f.iter().all(|(_, k)| k.contains("WrongLanguage") && k.contains("firm: false")));
        // German has no Death and Decay: unknown there, wrong language on the ruRU realm
        let f = found(&["/cast Death and Decay"], &both, false);
        assert!(f.iter().any(|(_, k)| k.contains("UnknownName")) && f.iter().any(|(_, k)| k.contains("WrongLanguage")));
        // a realm whose language is not cached is not judged
        assert!(found(&["/cast Fireball"], &[target("C-Fr", "frFR")], true).is_empty());
    }

    #[test]
    fn rejects_english_for_another_locale_but_keeps_real_data() {
        let en = rows(&[(1, "A"), (2, "B"), (3, "C"), (4, "D")]);
        assert!(validate("ruRU", &en, &en).is_err());
        assert!(validate("ruRU", &rows(&[]), &en).is_err());
        assert!(validate("enUS", &en, &en).is_ok());
        // Era leaves rows untranslated; 70% English is still real data
        let en: Rows = (1..=100).map(|i| (i, format!("N{i}"))).collect();
        let mut ru = en.clone();
        for r in ru.iter_mut().take(30) {
            r.1 = format!("Ru{}", r.0);
        }
        assert!(validate("ruRU", &ru, &en).is_ok());
    }

    #[test]
    fn cache_roundtrip_and_current_build() {
        let dir = tempfile::tempdir().unwrap();
        let tables = [rows(&[(1, "Огненный шар")]), rows(&[(10, "Зелье, лечебное")])];
        write_cache(dir.path(), "wow", "ruRU", "12.1.5.1", &tables).unwrap();
        let n = Names::load(dir.path(), "wow");
        assert!(n.has("ruRU") && !n.has("enUS"));
        assert!(n.locales["ruRU"].has("огненный шар") && n.locales["ruRU"].has("зелье, лечебное"));
        // another product with a longer name is not mistaken for it
        assert!(!Names::load(dir.path(), "wow_classic").has("ruRU"));
        // same build: nothing is downloaded (a download would need the network)
        assert!(!update_locale(dir.path(), "wow", "ruRU", "12.1.5.1", &mut |_| {}).unwrap());
        assert!(!Names::load(&dir.path().join("nope"), "wow").has("ruRU"));
    }

    #[test]
    #[ignore = "network: wago.tools"]
    fn live_wago() {
        let dir = tempfile::tempdir().unwrap();
        let needs = BTreeMap::from([("wow_classic_era", BTreeSet::from(["ruRU"]))]);
        update(dir.path(), &needs, &mut |_| {}).unwrap();
        let n = Names::load(dir.path(), "wow_classic_era");
        assert!(n.has("enUS") && n.has("ruRU") && n.locales["enUS"].has("fireball"));
        // second run: current, nothing downloaded
        let mut again = Vec::new();
        update(dir.path(), &needs, &mut |l| again.push(l)).unwrap();
        assert_eq!(again.len(), 2);
    }
}
