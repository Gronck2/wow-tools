//! Spell and item names per locale for the macro check: downloaded from wago.tools on request
//! (`wow-tools macros --update-names`, never by itself), cached in `data\names\<product>-<locale>.zip` and
//! used offline. A cache is current while `<product>-<locale>.meta` holds the build wago.tools calls the latest.
//!
//! shortcut: the key is the build of `/api/builds/<product>/latest`; the CSV can come from a slightly newer build,
//! which is harmless for names; upgrade only if names ever lag behind a patch.

use crate::macros::{How, Kind, Macro, Target, canonical_command};
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

/// Longest record kept; a stray quote would otherwise swallow the rest of the file into one record.
const RECORD_MAX: usize = 64 * 1024;

/// Splits one CSV record into fields (`""` is a quote inside a quoted field).
fn split_fields(rec: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted) = (Vec::new(), String::new(), false);
    let mut it = rec.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' if quoted && it.peek() == Some(&'"') => {
                it.next();
                cur.push('"');
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// A wago.tools export: the `ID` column and the first `*_lang` column are found by header name. A quoted field
/// may span lines; a record that grows past `RECORD_MAX` (a stray quote) is dropped and the next line starts fresh.
/// Rows without a number and a name are dropped.
fn parse_csv(text: &str) -> Result<Rows> {
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    let header = split_fields(lines.next().unwrap_or_default());
    let (Some(id_col), Some(name_col)) = (
        header.iter().position(|h| h.trim() == "ID"),
        header.iter().position(|h| h.trim().ends_with("_lang")),
    ) else {
        bail!(tr!("macros.names_columns"));
    };
    let mut out = Vec::new();
    let mut rec = String::new();
    let mut odd = false;
    for line in lines {
        if !rec.is_empty() {
            rec.push('\n');
        }
        rec.push_str(line);
        odd ^= line.matches('"').count() % 2 == 1;
        if odd && rec.len() <= RECORD_MAX {
            continue;
        }
        if !odd {
            let f = split_fields(&rec);
            if let (Some(id), Some(name)) = (f.get(id_col), f.get(name_col))
                && let Ok(id) = id.trim().parse::<u32>()
            {
                let name = name.replace(['\t', '\n', '\r'], " ");
                if !name.trim().is_empty() {
                    out.push((id, name));
                }
            }
        }
        rec.clear();
        odd = false;
    }
    Ok(out)
}

#[derive(Default)]
struct Table {
    by_name: HashMap<String, Vec<u32>>,
    by_id: HashMap<u32, String>,
}

impl Table {
    /// With `en` (the enUS table), a row that still has the English text of its id is an untranslated one: it is
    /// kept in `by_id` but is not a name of this language. Per row: another id with the same text stays.
    fn new(rows: Rows, en: Option<&Table>) -> Table {
        let mut t = Table::default();
        for (id, name) in rows {
            let untranslated = en.and_then(|e| e.by_id.get(&id)).is_some_and(|e| e.trim().eq_ignore_ascii_case(name.trim()));
            if !untranslated {
                t.by_name.entry(name.trim().to_lowercase()).or_default().push(id);
            }
            t.by_id.entry(id).or_insert(name);
        }
        t
    }

    /// The name of one of `ids` that this table has, one that differs from `lc` first (an untranslated
    /// row of another id must not hide a real translation, whatever the row order).
    fn name_of(&self, ids: Option<&Vec<u32>>, lc: &str) -> Option<&String> {
        let mut found = ids?.iter().filter_map(|id| self.by_id.get(id));
        let first = found.clone().next();
        found.find(|n| n.trim().to_lowercase() != lc).or(first)
    }
}

#[derive(Default)]
struct Locale {
    spell: Table,
    item: Table,
}

impl Locale {
    fn new([spell, item]: [Rows; 2], en: Option<&Locale>) -> Locale {
        Locale {
            spell: Table::new(spell, en.map(|e| &e.spell)),
            item: Table::new(item, en.map(|e| &e.item)),
        }
    }

    fn has(&self, lc: &str) -> bool {
        self.spell.by_name.contains_key(lc) || self.item.by_name.contains_key(lc)
    }

    /// What this locale calls what `other` calls `lc` (a spell first, then an item).
    fn translate(&self, other: &Locale, lc: &str) -> Option<String> {
        self.spell
            .name_of(other.spell.by_name.get(lc), lc)
            .or_else(|| self.item.name_of(other.item.by_name.get(lc), lc))
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
        let mut raw = BTreeMap::new();
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let file = e.file_name().to_string_lossy().into_owned();
            if let Some(locale) = file.strip_prefix(&prefix).and_then(|f| f.strip_suffix(".zip"))
                && let Ok(tables) = read_cache(dir, product, locale)
            {
                raw.insert(locale.to_string(), tables);
            }
        }
        // enUS first: the other locales are compared with it
        let en = raw.remove(ENUS).map(|t| Locale::new(t, None));
        for (locale, tables) in raw {
            names.locales.insert(locale, Locale::new(tables, en.as_ref()));
        }
        names.locales.extend(en.map(|l| (ENUS.to_string(), l)));
        names
    }

    pub fn has(&self, locale: &str) -> bool {
        self.locales.contains_key(locale)
    }

    fn knows(&self, locale: &str, lc: &str) -> bool {
        self.locales.get(locale).is_some_and(|l| l.has(lc))
    }

    /// `Spell(Subtext)`: when the whole text is no name anywhere, the part before the bracket may be.
    pub fn base<'a>(&self, name: &'a str) -> &'a str {
        if !self.locales.values().any(|l| l.has(&name.to_lowercase()))
            && let Some(i) = name.rfind('(')
            && name.ends_with(')')
            && self.locales.values().any(|l| l.has(&name[..i].trim_end().to_lowercase()))
        {
            return name[..i].trim_end();
        }
        name
    }

    /// What `dst` calls what `src` calls `lc` (lowercased): through the ids of a spell and of an item. Rows of `dst`
    /// that only repeat the text (untranslated, not a name of `dst`) are no answer; two different names left, or a
    /// spell and an item that disagree = `Ambiguous`.
    pub fn translate_exact(&self, src: &str, lc: &str, dst: &str) -> Exact {
        let (Some(s), Some(d)) = (self.locales.get(src), self.locales.get(dst)) else {
            return Exact::None;
        };
        let mut answers = Vec::new();
        for (from, to) in [(&s.spell, &d.spell), (&s.item, &d.item)] {
            let mut found: Vec<&str> = Vec::new();
            for name in from.by_name.get(lc).into_iter().flatten().filter_map(|id| to.by_id.get(id)) {
                let name = name.trim();
                let low = name.to_lowercase();
                if low != lc && to.by_name.contains_key(&low) && !found.iter().any(|f| f.to_lowercase() == low) {
                    found.push(name);
                }
            }
            match found[..] {
                [] => {}
                [one] => answers.push(one),
                _ => return Exact::Ambiguous,
            }
        }
        match answers[..] {
            [] => Exact::None,
            [one] => Exact::One(one.to_string()),
            [a, b] if a.to_lowercase() == b.to_lowercase() => Exact::One(a.to_string()),
            _ => Exact::Ambiguous,
        }
    }

    /// The name `lang` calls `name` (the bytes of `name` it replaces, that name), when it is wrong for a target of
    /// `valid` (the languages that work for each target) and exactly one name is known: every other language that
    /// knows the text must give the same one (homographs across languages = no guess). Targets whose
    /// languages are not all cached are not judged, as in `check`.
    pub fn retarget(&self, name: &str, lang: &str, valid: &[Vec<&str>]) -> Option<(usize, String)> {
        let name = self.base(name);
        let lc = name.to_lowercase();
        let wrong = valid
            .iter()
            .filter(|v| !v.is_empty() && v.iter().all(|c| self.locales.contains_key(*c)))
            .any(|v| !v.iter().any(|c| self.knows(c, &lc)));
        if !wrong {
            return None;
        }
        let mut one: Option<String> = None;
        for (code, other) in &self.locales {
            if code != lang && other.has(&lc) {
                match self.translate_exact(code, &lc, lang) {
                    Exact::One(new) if one.as_ref().is_none_or(|o| o.to_lowercase() == new.to_lowercase()) => one = Some(new),
                    Exact::One(_) | Exact::Ambiguous => return None,
                    Exact::None => {}
                }
            }
        }
        one.map(|new| (name.len(), new))
    }
}

#[cfg(test)]
type FakeRows<'a> = &'a [(u32, &'a str)];

#[cfg(test)]
impl Names {
    /// Fake name tables: (locale, spells, items) with enUS among them.
    pub fn fake(tables: &[(&str, FakeRows, FakeRows)]) -> Names {
        let rows = |r: &[(u32, &str)]| -> Rows { r.iter().map(|(i, n)| (*i, n.to_string())).collect() };
        let mut names = Names::default();
        let en = tables.iter().find(|t| t.0 == ENUS).map(|t| Locale::new([rows(t.1), rows(t.2)], None));
        for t in tables.iter().filter(|t| t.0 != ENUS) {
            names.locales.insert(t.0.to_string(), Locale::new([rows(t.1), rows(t.2)], en.as_ref()));
        }
        names.locales.extend(en.map(|l| (ENUS.to_string(), l)));
        names
    }
}

/// The answer of `Names::translate_exact`.
#[derive(Debug, PartialEq)]
pub enum Exact {
    One(String),
    Ambiguous,
    None,
}

fn cache_path(dir: &Path, product: &str, locale: &str, ext: &str) -> PathBuf {
    dir.join(format!("{product}-{locale}.{ext}"))
}

/// The name data of one product in the cache.
#[derive(Debug, Clone, PartialEq)]
pub struct Cached {
    pub product: String,
    /// The build of the enUS data (any locale's when there is none).
    pub build: String,
    pub locales: Vec<String>,
}

/// What `dir` holds, per product, from the small `.meta` files (no zip is opened).
pub fn cached(dir: &Path) -> Vec<Cached> {
    let mut out: BTreeMap<String, Cached> = BTreeMap::new();
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let file = e.file_name().to_string_lossy().into_owned();
        let Some((product, locale)) = file.strip_suffix(".meta").and_then(|f| f.split_once('-')) else {
            continue;
        };
        if !cache_path(dir, product, locale, "zip").exists() {
            continue;
        }
        let build = fs::read_to_string(e.path()).unwrap_or_default().trim().to_string();
        let entry = out.entry(product.to_string()).or_insert_with(|| Cached {
            product: product.to_string(),
            build: build.clone(),
            locales: Vec::new(),
        });
        if locale == ENUS {
            entry.build = build;
        }
        entry.locales.push(locale.to_string());
    }
    out.into_values().collect()
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
fn validate(locale: &str, spell: &Rows, item: &Rows, en: &Rows) -> Result<()> {
    if spell.is_empty() || item.is_empty() {
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
    // enUS is done first in `update`; its cache is what a translation is compared with.
    let en = if locale == ENUS {
        Rows::new()
    } else {
        read_cache(dir, product, ENUS)
            .with_context(|| tr!("macros.names_no_en", product = product, locale = locale))?
            .into_iter()
            .next()
            .unwrap_or_default()
    };
    progress(tr!("macros.names_download", product = product, locale = locale, build = build));
    let mut tables: [Rows; 2] = Default::default();
    for (slot, (table, _)) in tables.iter_mut().zip(TABLES) {
        let url = format!("https://wago.tools/db2/{table}/csv?product={product}&locale={locale}");
        *slot = parse_csv(&String::from_utf8_lossy(&get(&url, "text/csv")?))?;
    }
    validate(locale, &tables[0], &tables[1], &en)?;
    write_cache(dir, product, locale, build, &tables)?;
    Ok(true)
}

/// Brings the cache of every product in `needs` (product -> realm locales) up to date, enUS always included.
/// A failure of one locale does not stop the others; it is reported and counted.
pub fn update(dir: &Path, needs: &BTreeMap<String, BTreeSet<&str>>, progress: &mut dyn FnMut(String)) -> Result<()> {
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

/// Where the name data is cached.
pub fn dir() -> PathBuf {
    crate::config::data_dir().join("names")
}

/// Downloads what the characters of `flavors` need (`update` for the locales their macros are judged by).
pub fn update_for(flavors: &[crate::wow::Flavor], progress: &mut dyn FnMut(String)) -> Result<()> {
    let dir = dir();
    let mut needs = crate::macros::needs(flavors);
    needs.retain(|_, l| !l.is_empty());
    update(&dir, &needs, progress)
}

/// A spell or item name inside a macro line: its bytes `start..end` of the line and the text there.
#[derive(Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub name: String,
}

/// Where the spell and item names of a macro line are: `/cast [@focus] !Name(Rank 1); Other`, castsequence lists,
/// `reset=...`. Slots, bags (`/use 0 1`) and `item:123` are not names. `[...]`, `!` and `(Rank N)` stay outside.
pub fn name_spans(line: &str) -> Vec<Span> {
    let lead = line.len() - line.trim_start().len();
    let t = line.trim();
    let end = lead + t.find(|c: char| c.is_whitespace() || c == '[').unwrap_or(t.len());
    let cmd = canonical_command(&line[lead..end].to_lowercase());
    if !NAME_COMMANDS.contains(&cmd.as_str()) {
        return Vec::new();
    }
    let list = matches!(cmd.as_str(), "/castsequence" | "/castrandom" | "/userandom");
    // Pieces: the runs outside `[...]`, cut at `;` (and at `,` of a list).
    let (mut pieces, mut from, mut depth) = (Vec::new(), end, 0usize);
    for (i, c) in line[end..].char_indices().map(|(i, c)| (end + i, c)) {
        match c {
            '[' => {
                if depth == 0 {
                    pieces.push((from, i));
                }
                depth += 1;
            }
            ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    from = i + 1;
                }
            }
            ';' | ',' if depth == 0 && (c == ';' || list) => {
                pieces.push((from, i));
                from = i + 1;
            }
            _ => {}
        }
    }
    if depth == 0 {
        pieces.push((from, line.len()));
    }
    let off = |sub: &str| sub.as_ptr() as usize - line.as_ptr() as usize;
    let mut out = Vec::new();
    for (s, e) in pieces {
        let mut piece = line[s..e].trim_start();
        // `reset=5/target` leads the first piece of a castsequence
        while piece.starts_with("reset=") {
            piece = piece[piece.find(char::is_whitespace).unwrap_or(piece.len())..].trim_start();
        }
        let mut name = piece.trim_start_matches('!').trim();
        if let Some(i) = name.rfind('(')
            && name.ends_with(')')
            && name[i..].chars().any(|c| c.is_ascii_digit())
        {
            name = name[..i].trim_end();
        }
        let number = name.chars().all(|c| c.is_ascii_digit() || c.is_whitespace());
        if !name.is_empty() && !number && !name.to_lowercase().starts_with("item:") {
            out.push(Span {
                start: off(name),
                end: off(name) + name.len(),
                name: name.to_string(),
            });
        }
    }
    out
}

fn command_names(line: &str) -> Vec<String> {
    name_spans(line).into_iter().map(|s| s.name).collect()
}

/// Wrong-language and unknown names of one macro, as (body line from 1, kind). Targets whose realm language
/// is not cached are skipped (the caller says so).
pub fn check(m: &Macro, targets: &[Target], firm: bool, names: &Names) -> Vec<(usize, Kind)> {
    // A target is judged only when every language valid for it is cached.
    let ready: Vec<(&Target, Vec<&str>)> = targets
        .iter()
        .map(|t| (t, t.valid()))
        .filter(|(_, valid)| !valid.is_empty() && valid.iter().all(|c| names.locales.contains_key(*c)))
        .collect();
    let mut out = Vec::new();
    if ready.is_empty() {
        return out;
    }
    for (i, line) in m.body.iter().enumerate() {
        for name in command_names(line) {
            let name = names.base(&name).to_string();
            let lc = name.to_lowercase();
            let mut unknown = false;
            // (language of the name, name in the realm's language, how sure) -> targets
            let mut wrong: BTreeMap<(String, String, How), Vec<String>> = BTreeMap::new();
            for (t, valid) in &ready {
                if valid.iter().any(|c| names.locales[*c].has(&lc)) {
                    continue;
                }
                let fix = names.locales.iter().filter(|(_, l)| l.has(&lc)).find_map(|(code, other)| {
                    let fix = valid.iter().find_map(|c| names.locales[*c].translate(other, &lc))?;
                    let how = match (t.ru(), t.loc.is_some()) {
                        (true, _) if firm => How::Firm,
                        (true, _) => How::Maybe,
                        (_, true) => How::Client,
                        _ => How::Realm,
                    };
                    Some((code.clone(), fix, how))
                });
                match fix {
                    // the same text in both languages (an untranslated row of this id): nothing to fix
                    Some((_, fix, _)) if fix.trim().to_lowercase() == lc => {}
                    Some(key) => wrong.entry(key).or_default().push(t.label.clone()),
                    None => unknown = true,
                }
            }
            if unknown {
                out.push((i + 1, Kind::UnknownName { name: name.clone() }));
            }
            for ((lang, fix, how), bad) in wrong {
                out.push((
                    i + 1,
                    Kind::WrongLanguage {
                        name: name.clone(),
                        lang,
                        fix,
                        bad,
                        how,
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

    fn locale(spell: &[(u32, &str)], item: &[(u32, &str)], en: Option<&Locale>) -> Locale {
        Locale::new([rows(spell), rows(item)], en)
    }

    /// enUS, ruRU (spells 1 and 2, item 10) and deDE (spell 1 only).
    fn names() -> Names {
        let mut n = Names::default();
        let en = locale(
            &[(1, "Fireball"), (2, "Death and Decay"), (261403, "Fireball"), (3, "Charge"), (5, "Smite")],
            &[(10, "Healing Potion")],
            None,
        );
        // 261403 and 3 are untranslated rows; id 5 is really called "Flash Heal" in Russian
        let ru = locale(
            &[
                (1, "Огненный шар"),
                (2, "Смерть и разложение"),
                (261403, "Fireball"),
                (3, "Charge"),
                (5, "Flash Heal"),
            ],
            &[(10, "Лечебное зелье")],
            Some(&en),
        );
        n.locales.insert("deDE".into(), locale(&[(1, "Feuerball")], &[], Some(&en)));
        n.locales.insert("ruRU".into(), ru);
        n.locales.insert("enUS".into(), en);
        n
    }

    fn target(label: &str, code: &'static str) -> Target {
        Target {
            label: label.into(),
            loc: Some(Loc { code, guessed: false }),
            client_main: None,
            client: Vec::new(),
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
            parse_csv(text).unwrap(),
            rows(&[(1, "Fireball"), (2, "Hand of Gul'dan, the \"Big\""), (3, "two lines"), (6, "Last")])
        );
    }

    #[test]
    fn csv_finds_columns_by_header_and_survives_a_stray_quote() {
        assert_eq!(parse_csv("Flags,Name_lang,ID\n0,Fireball,1\n").unwrap(), rows(&[(1, "Fireball")]));
        assert!(parse_csv("ID,Other\n1,x\n").is_err() && parse_csv("Name_lang,Other\n1,x\n").is_err() && parse_csv("").is_err());
        let mut text = String::from("ID,Name_lang\n1,Before\n2,\"stray\n");
        for i in 3..20000 {
            text.push_str(&format!("{i},Row {i}\n"));
        }
        // the quote eats rows up to the record limit, then parsing goes on
        let got = parse_csv(&text).unwrap();
        assert_eq!(got[0], (1, "Before".to_string()));
        assert!(got.len() > 10000 && got.len() < 19000);
    }

    #[test]
    fn extracts_names_from_commands() {
        let n = command_names;
        assert_eq!(n("/cast [@focus,harm] !Fireball(Rank 3); [help] Heal"), ["Fireball", "Heal"]);
        assert_eq!(n("#showtooltip Fireball"), ["Fireball"]);
        assert_eq!(n("/userandom A b, C d"), ["A b", "C d"]);
        assert_eq!(n("/castsequence reset=5/target/shift A b, C d"), ["A b", "C d"]);
        assert_eq!(n("/cast Name With (OLD)"), ["Name With (OLD)"]);
        assert!(n("/use 13").is_empty() && n("/use 0 1").is_empty() && n("/use item:19019").is_empty());
        assert_eq!(n("/use [mod] Healing Potion"), ["Healing Potion"]);
        assert_eq!(n("/закл Огненный шар"), ["Огненный шар"]);
        assert_eq!(n("/cast Hand of Gul'dan, Big"), ["Hand of Gul'dan, Big"]);
        assert!(n("/stopmacro [combat]").is_empty() && n("/run x()").is_empty() && n("#showtooltip").is_empty());
    }

    #[test]
    fn name_spans_point_at_the_name_bytes_only() {
        let line = " /cast [@focus,harm] !Fireball(Rank 3);[help]  Heal ";
        let spans = name_spans(line);
        assert_eq!(spans.iter().map(|s| &line[s.start..s.end]).collect::<Vec<_>>(), ["Fireball", "Heal"]);
        assert!(spans.iter().all(|s| line[s.start..s.end] == s.name));
        let line = "/castsequence reset=5/target/shift A b, C d";
        assert_eq!(name_spans(line).iter().map(|s| &line[s.start..s.end]).collect::<Vec<_>>(), ["A b", "C d"]);
        let line = "/закл Огненный шар";
        assert_eq!(&line[name_spans(line)[0].start..], "Огненный шар");
        assert!(name_spans("/cast [@focus Fireball").is_empty());
    }

    #[test]
    fn exact_translation_needs_one_name() {
        let n = names();
        assert_eq!(n.translate_exact("enUS", "fireball", "deDE"), Exact::One("Feuerball".into()));
        assert_eq!(n.translate_exact("deDE", "feuerball", "ruRU"), Exact::One("Огненный шар".into()));
        // an item when no spell has the name; the untranslated row (same text) is no answer
        assert_eq!(n.translate_exact("enUS", "healing potion", "ruRU"), Exact::One("Лечебное зелье".into()));
        assert_eq!(n.translate_exact("enUS", "charge", "ruRU"), Exact::None);
        assert_eq!(n.translate_exact("enUS", "fireball", "frFR"), Exact::None);
        // two ids with one source name and two target names
        let en = locale(&[(20, "Smite"), (21, "Smite"), (22, "Smite")], &[], None);
        let ru = locale(&[(20, "Кара"), (21, "Священный огонь"), (22, "кара")], &[], Some(&en));
        let mut two = Names::default();
        two.locales.insert("ruRU".into(), ru);
        two.locales.insert("enUS".into(), en);
        assert_eq!(two.translate_exact("enUS", "smite", "ruRU"), Exact::Ambiguous);
        // `Spell(Subtext)` with a known spell before the bracket
        assert_eq!(n.base("Fireball(Racial)"), "Fireball");
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
        assert!(f[0].1.contains("WrongLanguage") && f[0].1.contains("enUS") && f[0].1.contains("Огненный шар") && f[0].1.contains("how: Firm"));
        // typo: unknown anywhere
        let f = found(&["/cast Огненый шар"], std::slice::from_ref(&ru), true);
        assert!(f.len() == 1 && f[0].1.contains("UnknownName"));
        // account scope: English works on neither realm
        let both = [ru.clone(), de.clone()];
        let f = found(&["/cast Fireball"], &both, false);
        assert_eq!(f.len(), 2);
        assert!(f.iter().all(|(_, k)| k.contains("WrongLanguage")));
        // the Russian realm only "may not work" (never the client clause), the German one has the client clause
        assert!(f.iter().any(|(_, k)| k.contains("how: Maybe") && k.contains("A-Ru")));
        assert!(f.iter().any(|(_, k)| k.contains("how: Client") && k.contains("B-De")));
        // German has no Death and Decay: unknown there, wrong language on the ruRU realm
        let f = found(&["/cast Death and Decay"], &both, false);
        assert!(f.iter().any(|(_, k)| k.contains("UnknownName")) && f.iter().any(|(_, k)| k.contains("WrongLanguage")));
        // `Spell(Subtext)`: the bracket is not part of the name
        let f = found(&["/cast Fireball(Racial)"], std::slice::from_ref(&ru), true);
        assert!(f.len() == 1 && f[0].1.contains("WrongLanguage") && f[0].1.contains("\"Fireball\""));
        assert!(found(&["/cast Огненный шар(Racial)"], std::slice::from_ref(&ru), true).is_empty());
        // a realm whose language is not cached is not judged
        assert!(found(&["/cast Fireball"], &[target("C-Fr", "frFR")], true).is_empty());
    }

    #[test]
    fn other_realms_accept_the_realm_language_or_a_client_language() {
        let names = names();
        let en = target("A-En", "enUS");
        let mut en_ru = target("B-En", "enUS");
        en_ru.client = vec!["ruRU"];
        let verdict = |body: &[&str], t: &[Target], firm| check(&mac(body), t, firm, &names).into_iter().map(|(_, k)| k).collect::<Vec<_>>();
        // A Russian client language on an English realm: Russian and English names both work.
        assert!(
            verdict(
                &["/cast Огненный шар", "/cast Fireball", "/закл Огненный шар"],
                std::slice::from_ref(&en_ru),
                true
            )
            .is_empty()
        );
        // No Russian client: soft even for the character's own macro, with the fix in the realm's language.
        let f = verdict(&["/cast Огненный шар"], std::slice::from_ref(&en), true);
        assert!(matches!(&f[..], [Kind::WrongLanguage { lang, fix, how: How::Client, .. }] if lang == "ruRU" && fix == "Fireball"));
        // Account list: only the character without the Russian client is named.
        let f = verdict(&["/cast Огненный шар"], &[en.clone(), en_ru], false);
        assert!(matches!(&f[..], [Kind::WrongLanguage { bad, how: How::Client, .. }] if bad == &["A-En"]));
        // An unknown realm is judged by the client languages alone.
        let mut nowhere = Target { loc: None, ..en };
        assert!(verdict(&["/cast Огненный шар"], std::slice::from_ref(&nowhere), true).is_empty());
        nowhere.client = vec!["enUS"];
        let f = verdict(&["/cast Огненный шар"], &[nowhere], true);
        assert!(matches!(&f[..], [Kind::WrongLanguage { how: How::Realm, .. }]));
    }

    #[test]
    fn account_wide_name_fails_differently_per_realm_and_unknown_realm_is_never_bad() {
        let mut nowhere = Target {
            loc: None,
            ..target("C-Nowhere", "enUS")
        };
        nowhere.client = vec!["enUS"];
        let ts = [target("A-Ru", "ruRU"), target("B-En", "enUS"), nowhere];
        let f = check(&mac(&["/cast Feuerball"]), &ts, false, &names());
        let by = |h: How| {
            f.iter()
                .find_map(|(_, k)| matches!(k, Kind::WrongLanguage { how, .. } if *how == h).then(|| format!("{k:?}")))
        };
        // Russian realm: "may not work"; English realm: "works only with a deDE client"; unknown realm: neither list.
        assert!(by(How::Maybe).is_some_and(|k| k.contains("A-Ru") && !k.contains("B-En") && k.contains("Огненный шар")));
        assert!(by(How::Client).is_some_and(|k| k.contains("B-En") && !k.contains("A-Ru") && k.contains("Fireball")));
        assert!(by(How::Realm).is_some_and(|k| k.contains("C-Nowhere")));
        assert!(by(How::Firm).is_none() && f.len() == 3);
    }

    #[test]
    fn untranslated_rows_are_not_names_of_the_language() {
        let names = names();
        let ru = target("A-Ru", "ruRU");
        let f = check(&mac(&["/cast Fireball"]), std::slice::from_ref(&ru), true, &names);
        assert!(matches!(&f[..], [(_, Kind::WrongLanguage { fix, .. })] if fix == "Огненный шар"));
        // the row stays in by_id; id 5 (its own English name differs) keeps the text valid
        assert!(names.locales["ruRU"].spell.by_id.contains_key(&261403));
        assert!(check(&mac(&["/cast Flash Heal"]), std::slice::from_ref(&ru), true, &names).is_empty());
        // identical in both languages for the same id: not flagged
        assert!(check(&mac(&["/cast Charge"]), std::slice::from_ref(&ru), true, &names).is_empty());
    }

    #[test]
    fn untranslated_row_listed_first_does_not_hide_the_translation() {
        let en = locale(&[(261403, "Fireball"), (1, "Fireball")], &[], None);
        let ru = locale(&[(261403, "Fireball"), (1, "Огненный шар")], &[], Some(&en));
        let mut names = Names::default();
        names.locales.insert("ruRU".into(), ru);
        names.locales.insert("enUS".into(), en);
        let f = check(&mac(&["/cast Fireball"]), &[target("A-Ru", "ruRU")], true, &names);
        assert!(matches!(&f[..], [(_, Kind::WrongLanguage { fix, .. })] if fix == "Огненный шар"));
    }

    #[test]
    fn untranslated_item_rows_are_not_names_of_the_language() {
        let en = locale(&[], &[(10, "Healing Potion"), (11, "Bandage"), (12, "Torch")], None);
        let ru = locale(&[], &[(10, "Healing Potion"), (11, "Бинт"), (12, "Healing Potion")], Some(&en));
        let mut names = Names::default();
        names.locales.insert("ruRU".into(), ru);
        names.locales.insert("enUS".into(), en);
        let ru = target("A-Ru", "ruRU");
        let f = check(&mac(&["/use Bandage"]), std::slice::from_ref(&ru), true, &names);
        assert!(matches!(&f[..], [(_, Kind::WrongLanguage { fix, .. })] if fix == "Бинт"));
        // id 12 really is "Healing Potion" in Russian, so the text stays valid there; id 10 is left untranslated
        assert!(check(&mac(&["/use Healing Potion"]), std::slice::from_ref(&ru), true, &names).is_empty());
    }

    #[test]
    fn russian_realm_stays_firm_whatever_the_client_language() {
        let mut ru = target("A-Ru", "ruRU");
        ru.client = vec!["enUS"];
        let f = check(&mac(&["/cast Fireball"]), std::slice::from_ref(&ru), true, &names());
        assert!(matches!(&f[..], [(_, Kind::WrongLanguage { how: How::Firm, fix, .. })] if fix == "Огненный шар"));
    }

    #[test]
    fn rejects_english_for_another_locale_but_keeps_real_data() {
        let en = rows(&[(1, "A"), (2, "B"), (3, "C"), (4, "D")]);
        assert!(validate("ruRU", &en, &en, &en).is_err());
        assert!(validate("ruRU", &rows(&[]), &en, &en).is_err());
        assert!(validate("enUS", &en, &rows(&[]), &en).is_err());
        assert!(validate("enUS", &en, &en, &en).is_ok());
        // Era leaves rows untranslated; 70% English is still real data
        let en: Rows = (1..=100).map(|i| (i, format!("N{i}"))).collect();
        let mut ru = en.clone();
        for r in ru.iter_mut().take(30) {
            r.1 = format!("Ru{}", r.0);
        }
        assert!(validate("ruRU", &ru, &en, &en).is_ok());
    }

    #[test]
    fn english_limit_is_98_percent() {
        let en: Rows = (1..=100).map(|i| (i, format!("N{i}"))).collect();
        let translated = |n: u32| -> Rows { en.iter().map(|(i, s)| (*i, if *i <= n { format!("Ru{i}") } else { s.clone() })).collect() };
        assert!(validate("ruRU", &translated(2), &en, &en).is_ok()); // 98 identical
        assert!(validate("ruRU", &translated(1), &en, &en).is_err()); // 99 identical
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
        let needs = BTreeMap::from([("wow_classic_era".to_string(), BTreeSet::from(["ruRU"]))]);
        update(dir.path(), &needs, &mut |_| {}).unwrap();
        let n = Names::load(dir.path(), "wow_classic_era");
        assert!(n.has("enUS") && n.has("ruRU") && n.locales["enUS"].has("fireball"));
        // second run: current, nothing downloaded
        let mut again = Vec::new();
        update(dir.path(), &needs, &mut |l| again.push(l)).unwrap();
        assert_eq!(again.len(), 2);
    }

    #[test]
    fn cached_lists_products_with_build_and_locales_from_the_meta_files() {
        let dir = tempfile::tempdir().unwrap();
        let put = |name: &str, text: &str| fs::write(dir.path().join(name), text).unwrap();
        put("wow-ruRU.zip", "");
        put("wow-ruRU.meta", "1.2.4");
        put("wow-enUS.zip", "");
        put("wow-enUS.meta", "1.2.5\n");
        // A meta without its zip (a cut write) is not data.
        put("wow-deDE.meta", "1.2.5");
        put("wow_classic_era-enUS.zip", "");
        put("wow_classic_era-enUS.meta", "1.15.1");
        assert!(cached(&dir.path().join("none")).is_empty());
        let got = cached(dir.path());
        assert_eq!(got.len(), 2);
        let wow = got.iter().find(|c| c.product == "wow").unwrap();
        assert_eq!((wow.build.as_str(), wow.locales.len()), ("1.2.5", 2));
        assert_eq!(got.iter().find(|c| c.product == "wow_classic_era").unwrap().build, "1.15.1");
    }
}
