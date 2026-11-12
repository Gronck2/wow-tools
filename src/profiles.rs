//! UI profiles: what makes a character's interface, saved under a name and put on another
//! character later.
//!
//! A profile is a zip in `<backup_dir>/profiles/<flavor>/`: the files of the character's folder
//! (chat, Edit Mode layouts, enabled addons, its own SavedVariables; not its key bindings and
//! macros, which belong to the character), the account's Edit Mode layouts and, for addons that
//! keep their settings account-wide as AceDB profiles (ElvUI, BigWigs, Clique...), which profile
//! the character uses and that profile's contents. Applying it copies the files, adds the Edit
//! Mode layouts the source uses to the target's own (`editmode::merge`) and switches the target
//! character to the same addon profile: shared with the source on the same account; on another
//! account the profile is added (under a new name when one of that name there differs). WTF is
//! snapshotted (`pre-profile`) first; the flavor must be closed.

use crate::app::{self, Ctx};
use crate::chars::Character;
use crate::editmode;
use crate::lua;
use crate::wow::Flavor;
use anyhow::{Context, Result, bail};
use chrono::{Local, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const META: &str = "profile.json";
const ADDONS: &str = "addons.json";
const FILES: &str = "character/";
/// The source account's `edit-mode-cache-account.txt`: the layouts its slots may point at.
const ACCOUNT_LAYOUTS: &str = "account/edit-mode-cache-account.txt";
/// Battle.net's checksums of the server-synced files, and what belongs to the character: its
/// key bindings, click bindings and macros (profiles saved before 0.5.2 have them; not applied).
const SKIP: [&str; 4] = ["cache.md5", "bindings-cache.wtf", "click-bindings-cache.txt", "macros-cache.txt"];

/// What a profile is, without its contents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub flavor: String,
    pub account: String,
    pub realm: String,
    pub name: String,
    pub created: NaiveDateTime,
    /// Files of the character folder.
    pub files: usize,
    /// Addons whose account-wide profile goes with it (their SavedVariables file names, no `.lua`).
    pub addons: Vec<String>,
    /// Of everything inside (`content_hash`): an unchanged character is not saved twice.
    #[serde(default)]
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub path: PathBuf,
    pub meta: Meta,
}

impl Profile {
    /// "Gronckk (Tarren Mill)".
    pub fn title(&self) -> String {
        format!("{} ({})", self.meta.name, self.meta.realm)
    }
}

/// The AceDB profile of one database (`ElvDB`, `ElvPrivateDB`...) a character used.
#[derive(Debug, Serialize, Deserialize)]
struct AceProfile {
    /// File in the account's `SavedVariables`.
    file: String,
    var: String,
    profile: String,
    /// The profile table as written in the file.
    data: Option<String>,
    /// The same profile of the addon's modules: (namespace, table).
    namespaces: Vec<(String, String)>,
}

pub fn root(ctx: &Ctx) -> PathBuf {
    ctx.cfg.backup_dir.join("profiles")
}

/// Every saved profile, newest first.
pub fn list(root: &Path) -> Vec<Profile> {
    let mut out = Vec::new();
    for flavor in fs::read_dir(root).into_iter().flatten().flatten() {
        for file in fs::read_dir(flavor.path()).into_iter().flatten().flatten() {
            let path = file.path();
            if path.extension().is_some_and(|x| x == "zip")
                && let Ok(meta) = read_meta(&path)
            {
                out.push(Profile { path, meta });
            }
        }
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.meta.created));
    out
}

fn read_meta(path: &Path) -> Result<Meta> {
    let mut zip = ZipArchive::new(File::open(path)?)?;
    let mut text = String::new();
    zip.by_name(META)?.read_to_string(&mut text)?;
    Ok(serde_json::from_str(&text)?)
}

/// The key AceDB keeps a character under.
fn ace_key(c: &Character) -> String {
    format!("{} - {}", c.name, c.realm)
}

/// `WTF\Account\<ACC>` of a character.
fn account_dir(c: &Character) -> &Path {
    c.dir.parent().and_then(Path::parent).unwrap_or(&c.dir)
}

/// `WTF\Account\<ACC>\SavedVariables` of a character.
fn account_sv(c: &Character) -> PathBuf {
    account_dir(c).join("SavedVariables")
}

/// When a character was last played: the newest of the files the game writes when it logs out.
/// Folder times are not that (deleting a `.bak` touches them), nor is `AddOns.txt` (the character
/// screen rewrites it for every character at once).
pub fn last_played(c: &Character) -> Option<NaiveDateTime> {
    let files = fs::read_dir(&c.dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.file_name().eq_ignore_ascii_case("AddOns.txt"));
    let saved = fs::read_dir(c.dir.join("SavedVariables"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("lua")));
    files
        .chain(saved)
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            meta.is_file().then(|| meta.modified().ok()).flatten()
        })
        .max()
        .map(|t| chrono::DateTime::<Local>::from(t).naive_local())
}

/// Files of a character's folder that go into a profile: (path inside it, file).
fn char_files(c: &Character) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for e in fs::read_dir(&c.dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if e.path().is_file() && !SKIP.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
            out.push((name, e.path()));
        }
    }
    for e in fs::read_dir(c.dir.join("SavedVariables")).into_iter().flatten().flatten() {
        let path = e.path();
        if path.is_file() && path.extension().is_some_and(|x| x.eq_ignore_ascii_case("lua")) {
            out.push((format!("SavedVariables/{}", e.file_name().to_string_lossy()), path));
        }
    }
    out.sort();
    out
}

/// The AceDB databases of `text` (`Name = { ["profileKeys"] = ...`): (name, index of its `{`).
fn ace_dbs(text: &str) -> Vec<(String, usize)> {
    lua::globals(text)
        .into_iter()
        .map(|(name, v)| (name, v.start))
        .filter(|&(_, open)| text.as_bytes()[open] == b'{' && lua::subtable(text, open, "profileKeys").is_some())
        .collect()
}

fn read_ace(c: &Character) -> Vec<AceProfile> {
    let key = ace_key(c);
    let mut files: Vec<PathBuf> = fs::read_dir(account_sv(c))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lua")))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for path in files {
        // Some addons keep raw bytes in strings: read what is text. A table with such bytes is
        // not carried (`data` stays empty), only which profile the character uses.
        let Ok(bytes) = fs::read(&path) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        if !text.contains("[\"profileKeys\"]") {
            continue;
        }
        for (var, db) in ace_dbs(&text) {
            let Some(profile) = lua::subtable(&text, db, "profileKeys").and_then(|keys| lua::string(&text, keys, &key)) else {
                continue;
            };
            let data = lua::subtable(&text, db, "profiles")
                .and_then(|p| lua::value_text(&text, p, &profile))
                .filter(|t| !t.contains(char::REPLACEMENT_CHARACTER));
            let mut namespaces = Vec::new();
            if let Some(nss) = lua::subtable(&text, db, "namespaces") {
                for ns in lua::entries(&text, nss) {
                    let Some(name) = ns.key else { continue };
                    let Some(p) = lua::subtable(&text, ns.value.start, "profiles") else {
                        continue;
                    };
                    if let Some(t) = lua::value_text(&text, p, &profile).filter(|t| !t.contains(char::REPLACEMENT_CHARACTER)) {
                        namespaces.push((name, t.to_string()));
                    }
                }
            }
            out.push(AceProfile {
                file: path.file_name().unwrap().to_string_lossy().into_owned(),
                var,
                profile,
                data: data.map(str::to_string),
                namespaces,
            });
        }
    }
    out
}

/// Saves `c`'s interface as a new profile; returns it and a summary.
/// What `save` did: a new profile, or none because an earlier one holds the same (`new` false).
pub struct Saved {
    pub summary: String,
    pub new: bool,
}

pub fn save(ctx: &Ctx, fl: &Flavor, c: &Character) -> Result<Saved> {
    let files = char_files(c);
    if files.is_empty() {
        bail!(tr!("profiles.empty_char", name = c));
    }
    let ace = read_ace(c);
    let mut addons: Vec<String> = ace.iter().map(|a| a.file.trim_end_matches(".lua").to_string()).collect();
    addons.dedup();
    let account_layouts = fs::read(account_dir(c).join(ACCOUNT_EDIT_MODE)).ok();
    let hash = content_hash(&files, &ace, account_layouts.as_deref())?;
    // The same as a profile saved before: a second copy says nothing new.
    if let Some(same) = list(&root(ctx)).into_iter().find(|p| {
        let m = &p.meta;
        m.hash == hash && m.flavor == fl.name && m.account == c.account && m.realm == c.realm && m.name == c.name
    }) {
        let summary = tr!("profiles.unchanged", time = short_time(same.meta.created));
        log::info!("{}: {}", same.title(), summary);
        return Ok(Saved { summary, new: false });
    }
    let meta = Meta {
        flavor: fl.name.clone(),
        account: c.account.clone(),
        realm: c.realm.clone(),
        name: c.name.clone(),
        created: Local::now().naive_local(),
        files: files.len(),
        addons,
        hash,
    };
    let dir = root(ctx).join(&fl.name);
    fs::create_dir_all(&dir).with_context(|| tr!("snapshot.create_failed", path = dir.display()))?;
    let stem: String = format!("{} {} - {}", meta.created.format("%Y%m%d-%H%M%S"), c.name, c.realm)
        .chars()
        .map(|ch| if "<>:\"/\\|?*".contains(ch) { '_' } else { ch })
        .collect();
    // Two saves in one second get different names.
    let mut path = dir.join(format!("{stem}.zip"));
    for n in 2.. {
        if !path.exists() {
            break;
        }
        path = dir.join(format!("{stem}-{n}.zip"));
    }
    let tmp = path.with_extension("zip.partial");
    let written = (|| -> Result<()> {
        let mut zip = ZipWriter::new(BufWriter::new(File::create(&tmp)?));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file(META, opts)?;
        zip.write_all(serde_json::to_string_pretty(&meta)?.as_bytes())?;
        zip.start_file(ADDONS, opts)?;
        zip.write_all(serde_json::to_string(&ace)?.as_bytes())?;
        if let Some(d) = &account_layouts {
            zip.start_file(ACCOUNT_LAYOUTS, opts)?;
            zip.write_all(d)?;
        }
        for (rel, src) in &files {
            zip.start_file(format!("{FILES}{rel}"), opts)?;
            io::copy(&mut File::open(src)?, &mut zip).with_context(|| tr!("error.read", path = src.display()))?;
        }
        zip.finish()?.flush()?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    fs::rename(&tmp, &path)?;
    let profile = Profile { path, meta };
    let summary = tr!(
        "profiles.saved",
        name = profile.title(),
        files = profile.meta.files,
        addons = profile.meta.addons.len()
    );
    log::info!("{}", summary);
    Ok(Saved { summary, new: true })
}

/// "23:38" today, else the day ("13.10.2026"): short enough for a panel row.
pub fn short_time(t: NaiveDateTime) -> String {
    if t.date() == Local::now().date_naive() {
        t.format("%H:%M").to_string()
    } else {
        t.format(&tr!("time.date").replace(" %H:%M", "")).to_string()
    }
}

/// FNV-1a over the files (path, size, bytes), the addon profiles and the account's layouts.
fn content_hash(files: &[(String, PathBuf)], ace: &[AceProfile], account_layouts: Option<&[u8]>) -> Result<String> {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for (rel, path) in files {
        let bytes = fs::read(path).with_context(|| tr!("error.read", path = path.display()))?;
        feed(rel.as_bytes());
        feed(&(bytes.len() as u64).to_le_bytes());
        feed(&bytes);
    }
    feed(serde_json::to_string(ace)?.as_bytes());
    if let Some(d) = account_layouts {
        feed(d);
    }
    Ok(format!("{h:016x}"))
}

/// Puts `profile` on `target` (a character of `fl`, the profile's flavor); returns a summary.
pub fn apply(ctx: &Ctx, fl: &Flavor, profile: &Profile, target: &Character) -> Result<String> {
    let meta = &profile.meta;
    if meta.account == target.account && meta.realm.to_lowercase() == target.realm.to_lowercase() && meta.name.to_lowercase() == target.name.to_lowercase() {
        bail!(tr!("profiles.same", name = profile.title()));
    }
    app::ensure_game_closed(fl)?;
    let mut zip = ZipArchive::new(File::open(&profile.path).with_context(|| tr!("error.read", path = profile.path.display()))?)?;
    let mut text = String::new();
    zip.by_name(ADDONS)?.read_to_string(&mut text)?;
    let ace: Vec<AceProfile> = serde_json::from_str(&text)?;

    if let Some(id) = app::safety_snapshot(ctx, fl, "pre-profile")? {
        log::info!("{}", tr!("profiles.snapshot", flavor = fl.title(), id = id));
    }
    let mut files = 0;
    let (mut edit_mode, mut source_account) = (None, None);
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else { continue };
        if rel == Path::new(ACCOUNT_LAYOUTS) {
            let mut d = Vec::new();
            entry.read_to_end(&mut d)?;
            source_account = Some(d);
            continue;
        }
        let Ok(rel) = rel.strip_prefix(FILES.trim_end_matches('/')) else { continue };
        if entry.is_dir() || rel.as_os_str().is_empty() {
            continue;
        }
        let top = rel.components().count() == 1;
        if top && SKIP.iter().any(|s| rel.as_os_str().eq_ignore_ascii_case(s)) {
            continue;
        }
        if top && rel.as_os_str().eq_ignore_ascii_case(EDIT_MODE) {
            let mut d = Vec::new();
            entry.read_to_end(&mut d)?;
            edit_mode = Some(d);
            continue;
        }
        let dst = target.dir.join(rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&dst).with_context(|| tr!("profiles.write_failed", path = dst.display()))?;
        io::copy(&mut entry, &mut out)?;
        files += 1;
    }
    let same_account = meta.account == target.account;
    let layouts = match edit_mode {
        Some(source) => put_layouts(meta, target, &source, source_account.as_deref())?,
        None => None,
    };
    files += usize::from(layouts.is_some());

    let key = ace_key(target);
    let sv = account_sv(target);
    let (mut switched, mut missing, mut unreadable) = (Vec::new(), Vec::new(), Vec::new());
    let mut by_file: Vec<(&str, Vec<&AceProfile>)> = Vec::new();
    for a in &ace {
        match by_file.iter_mut().find(|(f, _)| *f == a.file) {
            Some((_, list)) => list.push(a),
            None => by_file.push((&a.file, vec![a])),
        }
    }
    for (file, dbs) in by_file {
        let addon = file.trim_end_matches(".lua").to_string();
        let path = sv.join(file);
        let mut s = match fs::read(&path) {
            Err(_) => {
                missing.push(addon);
                continue;
            }
            // Rewriting it as text would mangle what is not.
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(s) => s,
                Err(_) => {
                    unreadable.push(addon);
                    continue;
                }
            },
        };
        let before = s.clone();
        let mut any = false;
        for a in dbs {
            any |= switch(&mut s, a, &key, same_account, &ace_key_of(meta));
        }
        if !any {
            missing.push(addon);
            continue;
        }
        if s != before {
            let tmp = path.with_extension("lua.wow-tools");
            fs::write(&tmp, &s).with_context(|| tr!("profiles.write_failed", path = tmp.display()))?;
            fs::rename(&tmp, &path)?;
        }
        switched.push(addon);
    }
    let mut summary = tr!(
        "profiles.applied",
        name = profile.title(),
        to = format!("{} ({})", target.name, target.realm),
        files = files,
        addons = switched.len()
    );
    if let Some(m) = &layouts {
        if !m.added.is_empty() {
            summary.push('\n');
            summary.push_str(&tr!("profiles.layouts_added", layouts = m.added.join(", ")));
        }
        if !m.full.is_empty() {
            summary.push('\n');
            summary.push_str(&tr!("profiles.layouts_full", layouts = m.full.join(", "), max = editmode::MAX_LAYOUTS));
        }
    }
    if !missing.is_empty() {
        summary.push('\n');
        summary.push_str(&tr!("profiles.missing", addons = missing.join(", ")));
    }
    if !unreadable.is_empty() {
        summary.push('\n');
        summary.push_str(&tr!("profiles.unreadable", addons = unreadable.join(", ")));
    }
    app::safety_done(ctx, fl);
    log::info!("{}", summary.replace('\n', " | "));
    Ok(summary)
}

const EDIT_MODE: &str = "edit-mode-cache-character.txt";
const ACCOUNT_EDIT_MODE: &str = "edit-mode-cache-account.txt";

/// Adds the Edit Mode layouts of the profile (`source`: its `edit-mode-cache-character.txt`,
/// `source_account`: the account's layouts when it was saved) to the target's and points the
/// target's specializations at them. The target's own layouts stay. A file not in the known
/// format is left as it is (copied only when the target has none). `None` when nothing was written.
fn put_layouts(meta: &Meta, target: &Character, source: &[u8], source_account: Option<&[u8]>) -> Result<Option<editmode::Merged>> {
    let dst = target.dir.join(EDIT_MODE);
    let current = fs::read(&dst).ok();
    let target_account = match fs::read(account_dir(target).join(ACCOUNT_EDIT_MODE)) {
        Ok(d) => editmode::account_layouts(&d),
        Err(_) => Some(Vec::new()),
    };
    let source_account = match source_account {
        Some(d) => editmode::account_layouts(d),
        // Saved before profiles kept them: on the same account they are the target's.
        None if meta.account == target.account => target_account.clone(),
        None => None,
    };
    let merged = target_account.and_then(|t| editmode::merge(source, source_account.as_deref(), current.as_deref(), &t, &meta.name));
    let merged = match (merged, &current) {
        (Some(m), _) => m,
        (None, None) => editmode::Merged {
            data: source.to_vec(),
            added: Vec::new(),
            full: Vec::new(),
        },
        (None, Some(_)) => {
            log::warn!("{}: Edit Mode layouts not in the known format, left as they are", dst.display());
            return Ok(None);
        }
    };
    if current.as_deref() != Some(&merged.data[..]) {
        fs::write(&dst, &merged.data).with_context(|| tr!("profiles.write_failed", path = dst.display()))?;
    }
    Ok(Some(merged))
}

fn ace_key_of(meta: &Meta) -> String {
    format!("{} - {}", meta.name, meta.realm)
}

/// Points `key` of database `a.var` in the file text `s` at the profile, adding the profile when
/// this account has none like it. False when the file has no such database.
fn switch(s: &mut String, a: &AceProfile, key: &str, same_account: bool, source_key: &str) -> bool {
    let db = |s: &str| ace_dbs(s).into_iter().find(|(v, _)| *v == a.var).map(|(_, open)| open);
    let Some(open) = db(s) else { return false };
    let mut name = a.profile.clone();
    if let Some(data) = &a.data {
        let existing = lua::subtable(s, open, "profiles")
            .and_then(|p| lua::value_text(s, p, &name))
            .map(str::to_string);
        // On the source's account the profile is the one it uses now; elsewhere a profile of the
        // same name ("Default") is someone else's.
        let ours = same_account || existing.as_deref().is_none_or(|t| t == data);
        if !ours {
            name = format!("{} ({source_key})", a.profile.trim());
        }
        if existing.is_none() || !ours {
            if let Some(p) = lua::subtable(s, open, "profiles") {
                lua::set(s, p, &name, data);
            }
            for (ns, text) in &a.namespaces {
                let open = db(s).unwrap();
                if let Some(p) = lua::subtable(s, open, "namespaces")
                    .and_then(|nss| lua::subtable(s, nss, ns))
                    .and_then(|n| lua::subtable(s, n, "profiles"))
                {
                    lua::set(s, p, &name, text);
                }
            }
        }
    }
    let open = db(s).unwrap();
    let keys = lua::subtable(s, open, "profileKeys").unwrap();
    lua::set(s, keys, key, &lua::quote(&name));
    true
}

/// A character to pick in the panel.
#[derive(Debug, Clone)]
pub struct CharEntry {
    pub flavor: String,
    pub character: Character,
    /// When it was last played (`last_played`).
    pub played: Option<NaiveDateTime>,
    pub look: Look,
}

/// A character's class and faction, as far as its addons know them (the game keeps neither in WTF).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Look {
    pub class: Option<Class>,
    pub faction: Option<Faction>,
}

impl Look {
    fn or(self, other: Look) -> Look {
        Look {
            class: self.class.or(other.class),
            faction: self.faction.or(other.faction),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Faction {
    Alliance,
    Horde,
}

impl Faction {
    fn parse(s: &str) -> Option<Faction> {
        match s {
            "Alliance" => Some(Faction::Alliance),
            "Horde" => Some(Faction::Horde),
            _ => None,
        }
    }

    /// From the race's file name (`BloodElf`, `Scourge`); `None` for races that pick a side
    /// (Pandaren, Dracthyr, Earthen) and for unknown ones.
    fn of_race(race: &str) -> Option<Faction> {
        const ALLIANCE: [&str; 11] = [
            "Human",
            "Dwarf",
            "NightElf",
            "Gnome",
            "Draenei",
            "Worgen",
            "VoidElf",
            "LightforgedDraenei",
            "DarkIronDwarf",
            "KulTiran",
            "Mechagnome",
        ];
        const HORDE: [&str; 11] = [
            "Orc",
            "Scourge",
            "Tauren",
            "Troll",
            "BloodElf",
            "Goblin",
            "Nightborne",
            "HighmountainTauren",
            "MagharOrc",
            "ZandalariTroll",
            "Vulpera",
        ];
        if ALLIANCE.contains(&race) {
            Some(Faction::Alliance)
        } else if HORDE.contains(&race) {
            Some(Faction::Horde)
        } else {
            None
        }
    }
}

/// A class by the game's token (`DEATHKNIGHT`), the same in every language.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Class {
    Warrior,
    Paladin,
    Hunter,
    Rogue,
    Priest,
    DeathKnight,
    Shaman,
    Mage,
    Warlock,
    Monk,
    Druid,
    DemonHunter,
    Evoker,
}

impl Class {
    fn parse(token: &str) -> Option<Class> {
        Some(match token {
            "WARRIOR" => Class::Warrior,
            "PALADIN" => Class::Paladin,
            "HUNTER" => Class::Hunter,
            "ROGUE" => Class::Rogue,
            "PRIEST" => Class::Priest,
            "DEATHKNIGHT" => Class::DeathKnight,
            "SHAMAN" => Class::Shaman,
            "MAGE" => Class::Mage,
            "WARLOCK" => Class::Warlock,
            "MONK" => Class::Monk,
            "DRUID" => Class::Druid,
            "DEMONHUNTER" => Class::DemonHunter,
            "EVOKER" => Class::Evoker,
            _ => return None,
        })
    }
}

/// The characters of `flavors` that still exist (`Roster`), last played first.
pub fn characters(flavors: &[Flavor]) -> Vec<CharEntry> {
    let mut rosters: HashMap<PathBuf, Roster> = HashMap::new();
    let mut out: Vec<CharEntry> = flavors
        .iter()
        .flat_map(|fl| {
            crate::chars::list(&fl.wtf_dir()).into_iter().map(move |c| CharEntry {
                flavor: fl.name.clone(),
                played: last_played(&c),
                character: c,
                look: Look::default(),
            })
        })
        .filter_map(|mut e| {
            let c = &e.character;
            let account = c.dir.parent().and_then(Path::parent).unwrap_or(&c.dir).to_path_buf();
            let roster = rosters.entry(account).or_insert_with_key(|dir| Roster::read(dir));
            if !roster.has(c, e.played) {
                return None;
            }
            e.look = roster.look(c);
            Some(e)
        })
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.played));
    out
}

/// Which characters an account has now. A folder in WTF stays after its character was deleted,
/// renamed or moved to another realm. The game keeps the account's character list in
/// `character-list-order.txt`, but by GUID only (`1084-0B52E2A1`: realm id and number); names
/// for them come from addons that keep their data by GUID (`["Player-1084-0B52E2A1"] = {
/// ["name"] = "Groncky", ["realm"] = "Tarren Mill"` in BigWigs, WorldQuestTracker, AllTheThings,
/// RCLootCouncil, ...). A character not played since its addons were installed has no name, so
/// it is judged realm by realm: the realm id of a named one gives the realm's name.
/// The same records give the class and the race or faction (WorldQuestTracker, AllTheThings,
/// AltVault); ElvUI keeps class and faction by realm and name (`elvui_looks`).
#[derive(Debug, Default)]
struct Roster {
    /// The list was read; when the game wrote it.
    listed: bool,
    listed_at: Option<NaiveDateTime>,
    /// Realm id -> (GUIDs on it, of them named).
    realms: HashMap<String, (usize, usize)>,
    /// (realm id, lowercase name, realm as `realm_key` when known, class and faction).
    known: Vec<(String, String, Option<String>, Look)>,
    /// (`realm_key`, lowercase name) -> class and faction from ElvUI.
    elvui: HashMap<(String, String), Look>,
}

impl Roster {
    fn read(account: &Path) -> Roster {
        let elvui = elvui_looks(account);
        let Ok(list) = fs::read_to_string(account.join("character-list-order.txt")) else {
            return Roster { elvui, ..Roster::default() };
        };
        let is_guid = |t: &str| {
            t.split_once('-').is_some_and(|(realm, n)| {
                !realm.is_empty() && realm.bytes().all(|b| b.is_ascii_digit()) && !n.is_empty() && n.bytes().all(|b| b.is_ascii_hexdigit())
            })
        };
        let mut wanted: Vec<String> = list.split_whitespace().filter(|t| is_guid(t)).map(|t| format!("Player-{t}")).collect();
        if wanted.is_empty() {
            return Roster { elvui, ..Roster::default() };
        }
        let realm_id = |guid: &str| guid.split('-').nth(1).unwrap_or_default().to_string();
        let listed_at = fs::metadata(account.join("character-list-order.txt"))
            .and_then(|m| m.modified())
            .ok()
            .map(|t| chrono::DateTime::<Local>::from(t).naive_local());
        let mut roster = Roster {
            listed: true,
            listed_at,
            elvui,
            ..Roster::default()
        };
        // A GUID is wanted until its name, class and faction are known.
        let mut found: Vec<(String, Found)> = Vec::new();
        for g in &wanted {
            roster.realms.entry(realm_id(g)).or_default().0 += 1;
        }
        // Small files first: often every GUID is found before the big ones.
        let mut files: Vec<(u64, PathBuf)> = fs::read_dir(account.join("SavedVariables"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("lua")))
            .map(|e| (e.metadata().map(|m| m.len()).unwrap_or(0), e.path()))
            .collect();
        files.sort();
        for (_, path) in files {
            if wanted.is_empty() {
                break;
            }
            let Ok(bytes) = fs::read(&path) else { continue };
            let text = String::from_utf8_lossy(&bytes);
            for (at, _) in text.match_indices("[\"Player-") {
                let key = at + 2;
                let Some(len) = text[key..].find('"') else { continue };
                let Some(i) = wanted.iter().position(|w| *w == text[key..key + len]) else {
                    continue;
                };
                // `["Player-..."] = {`: the record's name, realm, class, race, faction.
                let after = key + len + 1;
                let Some(open) = text[after..].find('{').map(|o| after + o) else { continue };
                if text[after..open].trim() != "] =" {
                    continue;
                }
                let entries = lua::entries(&text, open);
                let get = |keys: &[&str]| {
                    keys.iter().find_map(|k| {
                        let e = entries.iter().find(|e| e.key.as_deref() == Some(*k))?;
                        (text.as_bytes()[e.value.start] == b'"').then(|| lua::decode(&text.as_bytes()[e.value.clone()]))
                    })
                };
                let at = found.iter().position(|(g, _)| *g == wanted[i]).unwrap_or_else(|| {
                    found.push((wanted[i].clone(), Found::default()));
                    found.len() - 1
                });
                let f = &mut found[at].1;
                if f.name.is_none()
                    && let Some(name) = get(&["name"])
                {
                    // "Groncky-TarrenMill", or "Groncky" with the realm on its own.
                    f.name = Some(match name.split_once('-') {
                        Some((n, r)) => (n.to_string(), Some(r.to_string())),
                        None => (name, get(&["realm"])),
                    });
                }
                f.look.class = f.look.class.or_else(|| get(&["class", "classKey"]).and_then(|c| Class::parse(&c)));
                f.look.faction = f
                    .look
                    .faction
                    .or_else(|| get(&["faction"]).and_then(|c| Faction::parse(&c)))
                    .or_else(|| get(&["race", "raceKey"]).and_then(|r| Faction::of_race(&r)));
                if f.name.is_some() && f.look.class.is_some() && f.look.faction.is_some() {
                    wanted.remove(i);
                }
            }
        }
        for (guid, f) in found {
            let Some((name, realm)) = f.name else { continue };
            let id = realm_id(&guid);
            roster.realms.entry(id.clone()).or_default().1 += 1;
            roster.known.push((id, name.to_lowercase(), realm.map(|r| realm_key(&r)), f.look));
        }
        roster.fill_realms(account);
        roster
    }

    /// What the addons know of `c`: its GUID records, then ElvUI.
    fn look(&self, c: &Character) -> Look {
        let name = c.name.to_lowercase();
        let realm = realm_key(&c.realm);
        let records = self
            .known
            .iter()
            .find(|(_, n, r, _)| *n == name && r.as_ref().is_none_or(|r| *r == realm))
            .map(|k| k.3)
            .unwrap_or_default();
        records.or(self.elvui.get(&(realm, name)).copied().unwrap_or_default())
    }

    /// Some addons keep the name without the realm. The realm id tells it when another named
    /// character of that realm id has one; else the folders: the realms whose folder has that name,
    /// minus the realms other ids are, when one is left.
    fn fill_realms(&mut self, account: &Path) {
        let mut by_id: HashMap<String, String> = HashMap::new();
        for (id, _, realm, _) in &self.known {
            if let Some(r) = realm {
                by_id.entry(id.clone()).or_insert_with(|| r.clone());
            }
        }
        let folders: Vec<(String, String)> = fs::read_dir(account)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir() && e.file_name() != "SavedVariables")
            .flat_map(|realm| {
                let key = realm_key(&realm.file_name().to_string_lossy());
                fs::read_dir(realm.path())
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .map(move |c| (key.clone(), c.file_name().to_string_lossy().to_lowercase()))
            })
            .collect();
        for k in 0..self.known.len() {
            if self.known[k].2.is_some() {
                continue;
            }
            let id = self.known[k].0.clone();
            let realm = by_id.get(&id).cloned().or_else(|| {
                let taken: Vec<&String> = by_id.values().collect();
                let mut candidates: Vec<&String> = folders
                    .iter()
                    .filter(|(r, n)| *n == self.known[k].1 && !taken.contains(&r))
                    .map(|(r, _)| r)
                    .collect();
                candidates.dedup();
                (candidates.len() == 1).then(|| candidates[0].clone())
            });
            if let Some(r) = realm {
                by_id.entry(id).or_insert_with(|| r.clone());
                self.known[k].2 = Some(r);
            }
        }
    }

    /// False only when `c` is surely gone: its realm is on the list with every character named
    /// and `c` is not one of them, or every realm of the list is known and `c`'s is not one. A
    /// character played (`played`) after the list was written is newer than it: kept.
    fn has(&self, c: &Character, played: Option<NaiveDateTime>) -> bool {
        if !self.listed || played.is_some_and(|p| self.listed_at.is_none_or(|l| p > l)) {
            return true;
        }
        let name = c.name.to_lowercase();
        let realm = realm_key(&c.realm);
        if self.known.iter().any(|(_, n, r, _)| *n == name && r.as_ref().is_none_or(|r| *r == realm)) {
            return true;
        }
        // The realm ids whose named characters say they are `c`'s realm.
        let ids: Vec<&String> = self
            .realms
            .keys()
            .filter(|id| self.known.iter().any(|(i, _, r, _)| i == *id && r.as_ref() == Some(&realm)))
            .collect();
        if !ids.is_empty() {
            return ids.iter().any(|id| self.realms[*id].0 > self.realms[*id].1);
        }
        // Not on a known realm: gone if every realm of the list is known.
        let named = |id: &String| self.known.iter().any(|(i, _, r, _)| i == id && r.is_some());
        !self.realms.keys().all(named)
    }
}

/// What the GUID records of one character say so far.
#[derive(Default)]
struct Found {
    /// Name, and realm when the record has it.
    name: Option<(String, Option<String>)>,
    look: Look,
}

/// ElvUI's `ElvDB.class[realm][name] = "MAGE"` and `ElvDB.faction[realm][name] = "Horde"`, by
/// (`realm_key`, lowercase name).
fn elvui_looks(account: &Path) -> HashMap<(String, String), Look> {
    let mut out: HashMap<(String, String), Look> = HashMap::new();
    let Ok(bytes) = fs::read(account.join("SavedVariables").join("ElvUI.lua")) else {
        return out;
    };
    let text = String::from_utf8_lossy(&bytes);
    let Some(db) = lua::globals(&text)
        .into_iter()
        .find(|(n, v)| n == "ElvDB" && text.as_bytes()[v.start] == b'{')
        .map(|(_, v)| v.start)
    else {
        return out;
    };
    for table in ["class", "faction"] {
        let Some(open) = lua::subtable(&text, db, table) else { continue };
        for realm in lua::entries(&text, open) {
            let Some(r) = realm.key.as_deref().filter(|_| text.as_bytes()[realm.value.start] == b'{') else {
                continue;
            };
            for c in lua::entries(&text, realm.value.start) {
                let Some(n) = c.key.as_deref().filter(|_| text.as_bytes()[c.value.start] == b'"') else {
                    continue;
                };
                let value = lua::decode(&text.as_bytes()[c.value.clone()]);
                let look = out.entry((realm_key(r), n.to_lowercase())).or_default();
                match table {
                    "class" => look.class = Class::parse(&value),
                    _ => look.faction = Faction::parse(&value),
                }
            }
        }
    }
    out
}

/// "Tarren Mill" and "TarrenMill", "The Sha'tar" and "TheShatar" are the same realm.
fn realm_key(realm: &str) -> String {
    realm
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\'' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn delete(profile: &Profile) -> Result<()> {
    fs::remove_file(&profile.path).with_context(|| tr!("snapshot.remove_failed", path = profile.path.display()))?;
    log::info!("{}", tr!("profiles.deleted", name = profile.title()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chars;
    use crate::config::Config;

    const ELV: &str = "\r\nElvDB = {\r\n[\"profileKeys\"] = {\r\n[\"Main - Realm\"] = \"Main - Realm\",\r\n[\"Alt - Realm\"] = \"Default\",\r\n},\r\n[\"profiles\"] = {\r\n[\"Main - Realm\"] = {\r\n[\"scale\"] = 0.7,\r\n},\r\n[\"Default\"] = {\r\n},\r\n},\r\n[\"namespaces\"] = {\r\n[\"Bags\"] = {\r\n[\"profiles\"] = {\r\n[\"Main - Realm\"] = {\r\n[\"size\"] = 30,\r\n},\r\n},\r\n},\r\n},\r\n}\r\nElvPrivateDB = {\r\n[\"profileKeys\"] = {\r\n[\"Main - Realm\"] = \"Main - Realm\",\r\n},\r\n[\"profiles\"] = {\r\n[\"Main - Realm\"] = {\r\n[\"bags\"] = true,\r\n},\r\n},\r\n}\r\n";
    const OTHER: &str = "\r\nElvDB = {\r\n[\"profileKeys\"] = {\r\n},\r\n[\"profiles\"] = {\r\n},\r\n[\"namespaces\"] = {\r\n[\"Bags\"] = {\r\n[\"profiles\"] = {\r\n},\r\n},\r\n},\r\n}\r\n";

    fn ctx(backups: &Path) -> Ctx {
        Ctx {
            cfg: Config {
                backup_dir: backups.to_path_buf(),
                ..Config::default()
            },
            config_path: backups.join("config.toml"),
            explicit_config: None,
        }
    }

    #[test]
    fn save_and_apply() {
        let tmp = tempfile::tempdir().unwrap();
        let fl = Flavor::new("_retail_", tmp.path().join("_retail_"));
        let acc = fl.wtf_dir().join("Account/ACC");
        for c in ["Realm/Main", "Realm/Alt"] {
            fs::create_dir_all(acc.join(c).join("SavedVariables")).unwrap();
        }
        fs::create_dir_all(acc.join("SavedVariables")).unwrap();
        fs::create_dir_all(fl.wtf_dir().join("Account/ACC2/Realm/Third")).unwrap();
        fs::create_dir_all(fl.wtf_dir().join("Account/ACC2/SavedVariables")).unwrap();
        fs::write(acc.join("Realm/Main/bindings-cache.wtf"), "BIND").unwrap();
        fs::write(acc.join("Realm/Main/macros-cache.txt"), "MACROS").unwrap();
        fs::write(acc.join("Realm/Alt/macros-cache.txt"), "ALT MACROS").unwrap();
        fs::write(acc.join("Realm/Main/edit-mode-cache-character.txt"), "LAYOUT").unwrap();
        fs::write(acc.join("Realm/Main/cache.md5"), "md5").unwrap();
        fs::write(acc.join("Realm/Main/SavedVariables/Details.lua"), "x").unwrap();
        fs::write(acc.join("Realm/Main/SavedVariables/Details.lua.bak"), "old").unwrap();
        fs::write(acc.join("SavedVariables/ElvUI.lua"), ELV).unwrap();
        fs::write(acc.join("SavedVariables/Plain.lua"), "\r\nPlainDB = {\r\n}\r\n").unwrap();
        fs::write(fl.wtf_dir().join("Account/ACC2/SavedVariables/ElvUI.lua"), OTHER).unwrap();

        let backups = tmp.path().join("backups");
        let ctx = ctx(&backups);
        let all = chars::list(&fl.wtf_dir());
        let main = chars::find(&all, "ACC/Realm/Main").unwrap();
        save(&ctx, &fl, main).unwrap();
        let profile = list(&root(&ctx)).remove(0);
        assert_eq!(profile.meta.files, 2, "cache.md5, bindings and .bak stay out");
        assert_eq!(profile.meta.addons, ["ElvUI"]);
        assert_eq!(list(&root(&ctx)), std::slice::from_ref(&profile));
        assert!(apply(&ctx, &fl, &profile, main).is_err(), "not onto itself");
        let again = save(&ctx, &fl, main).unwrap();
        assert!(!again.new, "{}", again.summary);
        assert_eq!(list(&root(&ctx)).len(), 1, "nothing new to save");

        // Same account: the files, and the shared profile.
        let alt = chars::find(&all, "Realm/Alt").unwrap();
        let summary = apply(&ctx, &fl, &profile, alt).unwrap();
        assert!(summary.contains('2'), "{summary}");
        assert_eq!(fs::read_to_string(alt.dir.join("edit-mode-cache-character.txt")).unwrap(), "LAYOUT");
        assert!(alt.dir.join("SavedVariables/Details.lua").exists());
        assert!(!alt.dir.join("cache.md5").exists());
        assert_eq!(fs::read_to_string(alt.dir.join("macros-cache.txt")).unwrap(), "ALT MACROS", "the target's own");
        assert!(!alt.dir.join("bindings-cache.wtf").exists());
        let elv = fs::read_to_string(acc.join("SavedVariables/ElvUI.lua")).unwrap();
        assert_eq!(
            elv,
            ELV.replace("[\"Alt - Realm\"] = \"Default\",", "[\"Alt - Realm\"] = \"Main - Realm\",").replacen(
                "[\"Main - Realm\"] = \"Main - Realm\",\r\n},\r\n[\"profiles\"] = {\r\n[\"Main - Realm\"] = {\r\n[\"bags\"]",
                "[\"Main - Realm\"] = \"Main - Realm\",\r\n[\"Alt - Realm\"] = \"Main - Realm\",\r\n},\r\n[\"profiles\"] = {\r\n[\"Main - Realm\"] = {\r\n[\"bags\"]",
                1
            )
        );
        assert!(
            fs::read_dir(backups.join("_retail_/wtf"))
                .unwrap()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().contains("pre-profile"))
        );

        // Another account: the profile is added there; ElvPrivateDB is not in that file.
        let third = chars::find(&all, "Realm/Third").unwrap();
        apply(&ctx, &fl, &profile, third).unwrap();
        let safety = fs::read_dir(backups.join("_retail_/wtf"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("pre-profile"))
            .count();
        assert_eq!(safety, 1, "the snapshot of the first apply covers the second");
        // The game wrote WTF in between: that is not in the snapshot, so a new one is taken.
        fs::write(acc.join("Realm/Alt/config-cache.wtf"), "SET played \"1\"").unwrap();
        apply(&ctx, &fl, &profile, alt).unwrap();
        let safety = fs::read_dir(backups.join("_retail_/wtf"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("pre-profile"))
            .count();
        assert_eq!(safety, 2, "WTF changed after the last action");
        let other = fs::read_to_string(fl.wtf_dir().join("Account/ACC2/SavedVariables/ElvUI.lua")).unwrap();
        assert!(other.contains("[\"Third - Realm\"] = \"Main - Realm\","), "{other}");
        assert!(
            other.contains("[\"profiles\"] = {\r\n[\"Main - Realm\"] = {\r\n[\"scale\"] = 0.7,\r\n},\r\n},"),
            "{other}"
        );
        assert!(
            other.contains("[\"Bags\"] = {\r\n[\"profiles\"] = {\r\n[\"Main - Realm\"] = {\r\n[\"size\"] = 30,"),
            "{other}"
        );

        delete(&profile).unwrap();
        assert!(list(&root(&ctx)).is_empty());

        // A changed character is saved anew.
        assert!(save(&ctx, &fl, main).unwrap().new);
        fs::write(acc.join("Realm/Main/chat-cache.txt"), "CHAT2").unwrap();
        assert!(save(&ctx, &fl, main).unwrap().new);
        assert_eq!(list(&root(&ctx)).len(), 2);
    }

    #[test]
    fn roster_drops_gone_characters() {
        let tmp = tempfile::tempdir().unwrap();
        let acc = tmp.path().join("Account/ACC");
        fs::create_dir_all(acc.join("SavedVariables")).unwrap();
        let ch = |realm: &str, name: &str| Character {
            account: "ACC".into(),
            realm: realm.into(),
            name: name.into(),
            dir: acc.join(realm).join(name),
        };
        // No list: nothing is hidden.
        assert!(Roster::read(&acc).has(&ch("Tarren Mill", "Deleted"), None));

        fs::write(acc.join("character-list-order.txt"), "Version: 2\r\n0 1084-0B52E2A1 1\r\n0 1615-0AE5AEC9 2\r\n").unwrap();
        let big_wigs = "\r\nBigWigsDB = {\r\n[\"Player-1084-0B52E2A1\"] = {\r\n[\"name\"] = \"Groncky\",\r\n[\"realm\"] = \"Tarren Mill\",\r\n},\r\n}\r\n";
        fs::write(acc.join("SavedVariables/BigWigs.lua"), big_wigs).unwrap();
        // Tarren Mill's only character is named; the other realm of the list is not known yet.
        let partial = Roster::read(&acc);
        assert!(partial.has(&ch("Tarren Mill", "groncky"), None));
        assert!(!partial.has(&ch("Tarren Mill", "Deleted"), None));
        assert!(partial.has(&ch("Silvermoon", "Groncky"), None), "might be the unnamed one");

        let rc = "\r\nRC = {\r\n[\"Player-1615-0AE5AEC9\"] = {\r\n[\"name\"] = \"Гронки-Азурегос\",\r\n},\r\n}\r\n";
        fs::write(acc.join("SavedVariables/RCLootCouncil.lua"), rc).unwrap();
        let full = Roster::read(&acc);
        assert!(full.has(&ch("Азурегос", "Гронки"), None));
        assert!(!full.has(&ch("Silvermoon", "Groncky"), None), "moved to Tarren Mill");
        assert!(!full.has(&ch("Азурегос", "Гронкмаг"), None));

        // A name without its realm: the one realm whose folder has it, of those not taken.
        fs::write(
            acc.join("character-list-order.txt"),
            "Version: 2
0 1084-0B52E2A1 1
0 3391-0AAAAAAA 2
",
        )
        .unwrap();
        let wqt = "
WQT = {
[\"Player-3391-0AAAAAAA\"] = {
[\"name\"] = \"Gronckowski\",
},
}
";
        fs::write(acc.join("SavedVariables/WorldQuestTracker.lua"), wqt).unwrap();
        for c in ["Silvermoon/Gronckowski", "Tarren Mill/Gronckowski", "Tarren Mill/Groncky"] {
            fs::create_dir_all(acc.join(c)).unwrap();
        }
        let moved = Roster::read(&acc);
        assert!(moved.has(&ch("Silvermoon", "Gronckowski"), None));
        assert!(!moved.has(&ch("Tarren Mill", "Gronckowski"), None), "moved to Silvermoon");
        assert!(!moved.has(&ch("Kazzak", "Groncky"), None), "a realm the list does not have");
        // Played after the list was written (a new character the list has not caught up with).
        let later = Some(Local::now().naive_local() + chrono::Duration::hours(1));
        assert!(moved.has(&ch("Kazzak", "Groncky"), later));
    }

    #[test]
    fn class_and_faction_from_addons() {
        let tmp = tempfile::tempdir().unwrap();
        let acc = tmp.path().join("Account/ACC");
        fs::create_dir_all(acc.join("SavedVariables")).unwrap();
        let ch = |realm: &str, name: &str| Character {
            account: "ACC".into(),
            realm: realm.into(),
            name: name.into(),
            dir: acc.join(realm).join(name),
        };
        let look = |class, faction| Look {
            class: Some(class),
            faction: Some(faction),
        };
        fs::write(
            acc.join("character-list-order.txt"),
            "Version: 2\r\n0 1084-0B52E2A1 1\r\n0 1084-0B52E2A2 2\r\n0 1922-09199798 3\r\n",
        )
        .unwrap();
        // A name and class in one addon, the race in another; nested tables have names of their own.
        let wqt = "\r\nWQT = {\r\n[\"Player-1084-0B52E2A1\"] = {\r\n[\"class\"] = \"PALADIN\",\r\n[\"name\"] = \"Gronckk\",\r\n[\"realm\"] = \"Tarren Mill\",\r\n},\r\n}\r\n";
        let att = "\r\nATT = {\r\n[\"Player-1084-0B52E2A1\"] = {\r\n[\"Lockouts\"] = {\r\n{\r\n[\"name\"] = \"Boss\",\r\n},\r\n},\r\n[\"race\"] = \"BloodElf\",\r\n},\r\n[\"Player-1084-0B52E2A2\"] = {\r\n[\"name\"] = \"Pandy\",\r\n[\"realm\"] = \"Tarren Mill\",\r\n[\"class\"] = \"MONK\",\r\n[\"race\"] = \"Pandaren\",\r\n},\r\n}\r\n";
        // AltVault's own keys; ElvUI by realm and name.
        let av = "\r\nAV = {\r\n[\"Player-1922-09199798\"] = {\r\n[\"name\"] = \"Теньпитона\",\r\n[\"realm\"] = \"Азурегос\",\r\n[\"classKey\"] = \"DRUID\",\r\n[\"faction\"] = \"Alliance\",\r\n},\r\n}\r\n";
        let elv = "\r\nElvDB = {\r\n[\"class\"] = {\r\n[\"Tarren Mill\"] = {\r\n[\"Pandy\"] = \"MONK\",\r\n[\"Old\"] = \"MAGE\",\r\n},\r\n},\r\n[\"faction\"] = {\r\n[\"Tarren Mill\"] = {\r\n[\"Pandy\"] = \"Horde\",\r\n[\"Gronckk\"] = \"Alliance\",\r\n},\r\n},\r\n}\r\n";
        for (file, text) in [("WorldQuestTracker", wqt), ("AllTheThings", att), ("AltVault", av), ("ElvUI", elv)] {
            fs::write(acc.join(format!("SavedVariables/{file}.lua")), text).unwrap();
        }
        let r = Roster::read(&acc);
        assert_eq!(
            r.look(&ch("Tarren Mill", "gronckk")),
            look(Class::Paladin, Faction::Horde),
            "a record beats ElvUI"
        );
        assert_eq!(
            r.look(&ch("TarrenMill", "Pandy")),
            look(Class::Monk, Faction::Horde),
            "a pandaren's side from ElvUI"
        );
        assert_eq!(r.look(&ch("Азурегос", "Теньпитона")), look(Class::Druid, Faction::Alliance));
        let old = r.look(&ch("Tarren Mill", "Old"));
        assert_eq!((old.class, old.faction), (Some(Class::Mage), None));
        assert_eq!(r.look(&ch("Kazzak", "Gronckk")), Look::default(), "another realm");
    }

    /// The class and faction of the real characters (read-only): how many are known.
    #[test]
    #[ignore]
    fn live_looks() {
        let start = std::time::Instant::now();
        let all = characters(&crate::wow::all_flavors(crate::wow::detect_root().as_deref()));
        let known = |f: fn(&Look) -> bool| all.iter().filter(|e| f(&e.look)).count();
        println!(
            "{} characters in {:?}: class {}, faction {}",
            all.len(),
            start.elapsed(),
            known(|l| l.class.is_some()),
            known(|l| l.faction.is_some())
        );
        for e in all.iter().filter(|e| e.look.class.is_none() || e.look.faction.is_none()) {
            println!("  {} {} / {}: {:?}", e.flavor, e.character.name, e.character.realm, e.look);
        }
    }

    #[test]
    fn edit_mode_layouts_are_added() {
        let tmp = tempfile::tempdir().unwrap();
        let fl = Flavor::new("_retail_", tmp.path().join("_retail_"));
        let frame = "0 0 1 7 7 UIParent 0.0 45.0 -1 ##$$%/";
        for (acc, layout) in [("ACC", "1"), ("ACC2", "2")] {
            let dir = fl.wtf_dir().join("Account").join(acc);
            fs::create_dir_all(dir.join("SavedVariables")).unwrap();
            fs::write(dir.join(ACCOUNT_EDIT_MODE), format!("2 1 0 4 Main 1 {}\0", frame.replace("45.0", layout))).unwrap();
        }
        let acc = fl.wtf_dir().join("Account/ACC");
        for c in ["Realm/Main", "Realm/Alt"] {
            fs::create_dir_all(acc.join(c)).unwrap();
        }
        fs::create_dir_all(fl.wtf_dir().join("Account/ACC2/Realm/Third")).unwrap();
        fs::write(acc.join("Realm/Main").join(EDIT_MODE), format!("3 3 4 0 0 0 4 Heal 1 {frame}\0")).unwrap();
        fs::write(acc.join("Realm/Main/chat-cache.txt"), "CHAT").unwrap();
        fs::write(acc.join("Realm/Alt").join(EDIT_MODE), format!("4 4 4 4 4 4 4 Mine 1 {frame}\0")).unwrap();

        let ctx = ctx(&tmp.path().join("backups"));
        let all = chars::list(&fl.wtf_dir());
        save(&ctx, &fl, chars::find(&all, "Realm/Main").unwrap()).unwrap();
        let profile = list(&root(&ctx)).remove(0);

        // Same account: "Main" is shared, "Heal" added next to the target's own "Mine".
        let summary = apply(&ctx, &fl, &profile, chars::find(&all, "Realm/Alt").unwrap()).unwrap();
        assert!(summary.contains("Heal"), "{summary}");
        assert_eq!(
            fs::read_to_string(acc.join("Realm/Alt").join(EDIT_MODE)).unwrap(),
            format!("3 3 5 3 3 3 4 Mine 1 {frame} 4 Heal 1 {frame}\0")
        );
        // Another account: its own "Main" differs, so the source's comes renamed.
        apply(&ctx, &fl, &profile, chars::find(&all, "Realm/Third").unwrap()).unwrap();
        let main = frame.replace("45.0", "1");
        assert_eq!(
            fs::read_to_string(fl.wtf_dir().join("Account/ACC2/Realm/Third").join(EDIT_MODE)).unwrap(),
            format!("4 4 5 4 4 4 11 Main (Main) 1 {main} 4 Heal 1 {frame}\0")
        );
    }

    #[test]
    fn same_name_elsewhere_is_kept() {
        let mut s = "\r\nDB = {\r\n[\"profileKeys\"] = {\r\n},\r\n[\"profiles\"] = {\r\n[\"Default\"] = {\r\n[\"a\"] = 2,\r\n},\r\n},\r\n}\r\n".to_string();
        let a = AceProfile {
            file: "X.lua".into(),
            var: "DB".into(),
            profile: "Default".into(),
            data: Some("{\r\n[\"a\"] = 1,\r\n}".into()),
            namespaces: Vec::new(),
        };
        assert!(switch(&mut s, &a, "Alt - R", false, "Main - R"));
        assert!(s.contains("[\"Alt - R\"] = \"Default (Main - R)\","), "{s}");
        assert!(s.contains("[\"Default\"] = {\r\n[\"a\"] = 2,"), "the other account's Default stays");
        assert!(s.contains("[\"Default (Main - R)\"] = {\r\n[\"a\"] = 1,"), "{s}");
    }
}
