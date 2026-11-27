use crate::errors::Coded;
use anyhow::{Context, Result};
use chrono::{Datelike, Days, NaiveDate, NaiveDateTime, NaiveTime};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// When the automatic snapshots are taken (the manual "backup now" is always there).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum When {
    /// Every time the game closes.
    #[default]
    GameExit,
    /// On `backup_days` at `backup_time`; a time missed while the computer was off is caught up
    /// as soon as it is on.
    Schedule,
    /// Every `backup_every_days` days (counted from the last snapshot's day) at `backup_time`,
    /// caught up like the schedule.
    EveryDays,
}

/// When addons are updated automatically (the manual "update now" is always there).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateWhen {
    /// Every `update_every` minutes (counted from the last update; the game must be closed).
    Interval,
    /// On `update_days` at `update_time`, caught up like the backup schedule.
    #[default]
    Schedule,
    /// Once when WoW Tools starts (with Windows, before playing), unless it updated within the last hour.
    Startup,
}

/// How the cleanup files moved screenshots: a folder per year, month or day of the shot, or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShotsBy {
    Year,
    #[default]
    Month,
    Day,
    /// All in one folder.
    None,
}

impl ShotsBy {
    pub const ALL: [ShotsBy; 4] = [ShotsBy::Day, ShotsBy::Month, ShotsBy::Year, ShotsBy::None];
}

/// A tab of the tray panel; the user can reorder and hide them (`panel_tabs`, `hidden_tabs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelTab {
    Backup,
    Addons,
    Cleanup,
    Profiles,
    Macros,
}

impl PanelTab {
    pub const ALL: [PanelTab; 5] = [PanelTab::Backup, PanelTab::Addons, PanelTab::Cleanup, PanelTab::Profiles, PanelTab::Macros];

    pub fn label(self) -> &'static str {
        match self {
            PanelTab::Backup => tr!("panel.tab_backup"),
            PanelTab::Addons => tr!("panel.tab_addons"),
            PanelTab::Cleanup => tr!("cleanup.tab"),
            PanelTab::Profiles => tr!("profiles.tab"),
            PanelTab::Macros => tr!("macros.tab"),
        }
    }
}

/// A list of tabs that skips names it does not know (a tab of a newer version, a typo) instead
/// of rejecting the whole config.
fn known_tabs<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<PanelTab>, D::Error> {
    let names = Vec::<String>::deserialize(d)?;
    Ok(names
        .into_iter()
        .filter_map(|n| PanelTab::deserialize(serde::de::value::StringDeserializer::<D::Error>::new(n)).ok())
        .collect())
}

const MAX_KEEP: u64 = 10_000;
const MAX_EVERY_DAYS: u64 = 365;
const MAX_EVERY_MINUTES: u64 = 525_600;

/// Choices of the "update every" stepper, in minutes.
pub const UPDATE_EVERY: &[u64] = &[30, 60, 120, 180, 360, 720];

/// Choices of the "back up every" stepper, in days.
pub const BACKUP_EVERY_DAYS: &[u64] = &[2, 3, 4, 5, 7, 10, 14, 21, 30];

/// How long after its start WoW Tools runs a `UpdateWhen::Startup` update (the network and the
/// rest of the sign-in come first).
const STARTUP_DELAY_MIN: i64 = 1;
/// An update this recent makes the one at start unnecessary (a restart after a self-update).
const STARTUP_SKIP_MIN: i64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Day {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Day {
    pub const ALL: [Day; 7] = [Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri, Day::Sat, Day::Sun];

    pub fn short(self) -> &'static str {
        match self {
            Day::Mon => tr!("day.mon"),
            Day::Tue => tr!("day.tue"),
            Day::Wed => tr!("day.wed"),
            Day::Thu => tr!("day.thu"),
            Day::Fri => tr!("day.fri"),
            Day::Sat => tr!("day.sat"),
            Day::Sun => tr!("day.sun"),
        }
    }

    pub fn of(date: NaiveDate) -> Day {
        Day::ALL[date.weekday().num_days_from_monday() as usize]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// WoW install root (the folder holding `_retail_`, `_classic_`, ...).
    /// Auto-detected when unset.
    pub wow_path: Option<PathBuf>,
    /// Flavors to back up. Empty means all found.
    pub flavors: Vec<String>,
    /// Where snapshots are stored.
    pub backup_dir: PathBuf,
    /// When automatic snapshots are taken.
    pub backup_when: When,
    /// Days of `When::Schedule`.
    pub backup_days: Vec<Day>,
    /// Time of `When::Schedule` and `When::EveryDays`, "HH:MM" local.
    pub backup_time: String,
    /// Days between snapshots of `When::EveryDays`.
    pub backup_every_days: u64,
    /// Update addons automatically through an addon manager app (started hidden, closed after).
    pub update_addons: bool,
    /// The app to update through; unset or not installed = the first one installed.
    pub update_app: Option<crate::updater::AddonApp>,
    /// Never update while a game client runs (addons and WeakAuras swapped under it can break it).
    pub update_skip_in_game: bool,
    /// With the Wago App chosen: what it updates (it cannot update addons unattended).
    pub wago_weakauras: bool,
    pub wago_ui_packs: bool,
    /// Update ElvUI from tukui.org with the addon updates (the managers do not carry it).
    pub update_elvui: bool,
    pub update_when: UpdateWhen,
    /// Minutes of `UpdateWhen::Interval`.
    pub update_every: u64,
    pub update_days: Vec<Day>,
    /// "HH:MM" local.
    pub update_time: String,
    /// Snapshots to keep per series (WTF and AddOns separately).
    pub keep: usize,
    /// Whether to back up Interface/AddOns.
    pub include_addons: bool,
    /// Skip snapshots while the game runs (WoW writes WTF on logout).
    pub skip_while_running: bool,
    /// File extensions left out of snapshots.
    pub exclude_ext: Vec<String>,
    /// Interface language; unset = the Windows one.
    pub language: Option<crate::i18n::Lang>,
    /// Tray balloons (snapshots, updates, errors).
    pub notifications: bool,
    /// Ask GitHub once a day for a newer WoW Tools (`selfupdate`).
    pub check_updates: bool,
    /// A balloon about a newer WoW Tools, even with `notifications` off (the icon shows it either way).
    pub notify_updates: bool,
    /// Where the cleanup moves screenshots; unset = `Pictures\World of Warcraft`.
    pub screenshots_dir: Option<PathBuf>,
    /// Date folders under it.
    pub screenshots_by: ShotsBy,
    /// A folder per game version ("Retail", "Classic Era") above the date folders.
    pub screenshots_per_flavor: bool,
    /// The order of the panel's tabs; tabs missing here follow in the default order (`tab_order`).
    #[serde(deserialize_with = "known_tabs")]
    pub panel_tabs: Vec<PanelTab>,
    /// Tabs the panel does not show; at least one always stays (`shown_tabs`).
    #[serde(deserialize_with = "known_tabs")]
    pub hidden_tabs: Vec<PanelTab>,
    /// With `screenshots_per_flavor`: a folder chosen for a game version (`_retail_` -> path) instead of
    /// `<screenshots_dir>\<title>`. Last: TOML wants tables after the plain values.
    pub screenshots_dirs: std::collections::BTreeMap<String, PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            wow_path: None,
            flavors: Vec::new(),
            backup_dir: data_dir().join("backups"),
            backup_when: When::GameExit,
            backup_days: Day::ALL.to_vec(),
            backup_time: "10:00".into(),
            backup_every_days: 7,
            update_addons: false,
            update_app: None,
            update_skip_in_game: true,
            wago_weakauras: true,
            wago_ui_packs: true,
            update_elvui: true,
            update_when: UpdateWhen::Schedule,
            update_every: 60,
            update_days: Day::ALL.to_vec(),
            update_time: "09:00".into(),
            keep: 30,
            include_addons: true,
            skip_while_running: true,
            exclude_ext: vec!["bak".into(), "old".into()],
            language: None,
            notifications: true,
            check_updates: true,
            notify_updates: true,
            screenshots_dir: None,
            screenshots_by: ShotsBy::Month,
            screenshots_per_flavor: true,
            panel_tabs: PanelTab::ALL.to_vec(),
            hidden_tabs: Vec::new(),
            screenshots_dirs: Default::default(),
        }
    }
}

impl Config {
    /// `backup_time`, or 10:00 when it does not parse.
    pub fn time(&self) -> NaiveTime {
        parse_time(&self.backup_time, 10)
    }

    /// A scheduled snapshot is due: a scheduled moment has passed with no snapshot since.
    /// This is also what catches up a moment missed while the computer was off.
    pub fn schedule_due(&self, now: NaiveDateTime, last_backup: Option<NaiveDateTime>) -> bool {
        match self.backup_when {
            When::GameExit => false,
            When::Schedule => due(&self.backup_days, self.time(), now, last_backup),
            // The latest `backup_time` (today's or yesterday's) is `backup_every_days` days after the last snapshot's day.
            When::EveryDays => last_scheduled(&Day::ALL, self.time(), now)
                .is_some_and(|at| last_backup.is_none_or(|l| (at.date() - l.date()).num_days() >= self.backup_every_days.clamp(1, MAX_EVERY_DAYS) as i64)),
        }
    }

    /// Every panel tab once, in the user's order.
    pub fn tab_order(&self) -> Vec<PanelTab> {
        let mut out: Vec<PanelTab> = Vec::new();
        for &t in self.panel_tabs.iter().chain(PanelTab::ALL.iter()) {
            if !out.contains(&t) {
                out.push(t);
            }
        }
        out
    }

    /// The tabs the panel shows, in order; all of them hidden = all shown.
    pub fn shown_tabs(&self) -> Vec<PanelTab> {
        let order = self.tab_order();
        let shown: Vec<PanelTab> = order.iter().copied().filter(|t| !self.hidden_tabs.contains(t)).collect();
        if shown.is_empty() { order } else { shown }
    }

    /// Takes `shown` (the shown tabs, rearranged in the panel) as their new order; hidden tabs
    /// keep their places between them. Anything but a rearrangement of `shown_tabs` is ignored.
    pub fn set_shown_order(&mut self, shown: &[PanelTab]) {
        let mut old = self.shown_tabs();
        let mut new = shown.to_vec();
        old.sort_by_key(|&t| t as u8);
        new.sort_by_key(|&t| t as u8);
        if old != new {
            return;
        }
        let mut next = shown.iter();
        let visible = self.shown_tabs();
        self.panel_tabs = self
            .tab_order()
            .into_iter()
            .map(|t| if visible.contains(&t) { *next.next().unwrap_or(&t) } else { t })
            .collect();
    }

    /// Shows or hides `tab`; refuses (false) to hide the last one shown.
    pub fn toggle_tab(&mut self, tab: PanelTab) -> bool {
        if let Some(pos) = self.hidden_tabs.iter().position(|&t| t == tab) {
            self.hidden_tabs.remove(pos);
            return true;
        }
        if self.shown_tabs() == [tab] {
            return false;
        }
        self.hidden_tabs.push(tab);
        true
    }

    /// `update_time`, or 09:00 when it does not parse.
    pub fn update_at(&self) -> NaiveTime {
        parse_time(&self.update_time, 9)
    }

    /// An automatic addon update is due: by the schedule (same rules as the backups) or when
    /// `update_every` minutes have passed since the last one.
    /// `started`: when WoW Tools started (`UpdateWhen::Startup`).
    pub fn update_due(&self, now: NaiveDateTime, last_update: Option<NaiveDateTime>, started: NaiveDateTime) -> bool {
        if !self.update_addons {
            return false;
        }
        match self.update_when {
            UpdateWhen::Schedule => due(&self.update_days, self.update_at(), now, last_update),
            UpdateWhen::Interval => last_update.is_none_or(|l| now - l >= chrono::Duration::minutes(self.update_every.clamp(1, MAX_EVERY_MINUTES) as i64)),
            UpdateWhen::Startup => {
                now - started >= chrono::Duration::minutes(STARTUP_DELAY_MIN)
                    && last_update.is_none_or(|l| l < started - chrono::Duration::minutes(STARTUP_SKIP_MIN))
            }
        }
    }

    /// "every hour", "on schedule: daily at 09:00", ... (in the interface language)
    pub fn update_text(&self) -> String {
        match self.update_when {
            UpdateWhen::Interval => every_text(self.update_every),
            UpdateWhen::Startup => tr!("schedule.at_startup").into(),
            UpdateWhen::Schedule => tr!(
                "schedule.update_at",
                days = days_text(&self.update_days),
                time = self.update_at().format("%H:%M")
            ),
        }
    }

    /// "After every game exit", "On schedule: Mon, Wed, Fri at 10:00", ...
    pub fn schedule_text(&self) -> String {
        match self.backup_when {
            When::GameExit => tr!("schedule.after_exit").into(),
            When::Schedule => tr!("schedule.backup_at", days = days_text(&self.backup_days), time = self.time().format("%H:%M")),
            When::EveryDays => tr!(
                "schedule.backup_every",
                days = every_days_text(self.backup_every_days),
                time = self.time().format("%H:%M")
            ),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path).with_context(|| tr!("error.read", path = path.display()))?;
        toml::from_str(&text).with_context(|| Coded::config_invalid(path, tr!("config.bad", path = path.display())))
    }

    /// Written to a temp file next to it and renamed over it, so a reader never sees half a file.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // A temp of its own per save: the tray and the CLI may save at once.
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let tmp = PathBuf::from(tmp);
        let written = fs::write(&tmp, toml::to_string_pretty(self)?).and_then(|()| fs::rename(&tmp, path));
        if written.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        written.with_context(|| tr!("error.write", path = path.display()))
    }

    /// Every setting under its file name; the unset ones are `null`.
    pub fn image(&self) -> serde_json::Map<String, Value> {
        match serde_json::to_value(self) {
            Ok(Value::Object(m)) => m,
            _ => unreachable!("Config serializes to an object"),
        }
    }

    pub fn get_key(&self, key: &str) -> Result<Value> {
        Ok(self.image().remove(key).ok_or_else(|| unknown_key(key))?)
    }

    /// Sets one setting from its JSON form; the whole config is checked, and nothing changes on an error.
    pub fn set_key(&mut self, key: &str, value: Value) -> Result<()> {
        let mut image = self.image();
        let slot = image.get_mut(key).ok_or_else(|| unknown_key(key))?;
        let asked = value.clone();
        *slot = value;
        let next: Config =
            serde_json::from_value(Value::Object(image)).map_err(|e| Coded::invalid_value(key, "type", tr!("config.bad_type", key = key, detail = e)))?;
        // Something the types dropped silently (an unknown tab name) is not what was asked for.
        if next.get_key(key)? != asked {
            return Err(Coded::invalid_value(key, "type", tr!("config.bad_type", key = key, detail = "unknown value")).into());
        }
        if let Some((bad, reason)) = next.invalid() {
            let message = if reason == "range" {
                tr!("config.bad_range", key = bad)
            } else {
                tr!("config.bad_format", key = bad)
            };
            return Err(Coded::invalid_value(bad, reason, message).into());
        }
        *self = next;
        Ok(())
    }

    /// The default of one setting.
    pub fn unset_key(&mut self, key: &str) -> Result<()> {
        let default = Config::default().get_key(key)?;
        self.set_key(key, default)
    }

    /// What the types do not check: the first setting that is out of range (`range`) or malformed (`format`).
    fn invalid(&self) -> Option<(&'static str, &'static str)> {
        let time = |s: &str| NaiveTime::parse_from_str(s, "%H:%M").is_ok();
        let flavor = |f: &String| f.len() > 2 && f.starts_with('_') && f.ends_with('_') && f.chars().all(|c| c == '_' || c.is_ascii_alphanumeric());
        if !time(&self.backup_time) {
            return Some(("backup_time", "format"));
        }
        if !time(&self.update_time) {
            return Some(("update_time", "format"));
        }
        if !self.flavors.iter().all(flavor) {
            return Some(("flavors", "format"));
        }
        let absolute = |p: &Path| p.is_absolute();
        if !absolute(&self.backup_dir) {
            return Some(("backup_dir", "format"));
        }
        if !self.wow_path.iter().all(|p| absolute(p)) {
            return Some(("wow_path", "format"));
        }
        if !self.screenshots_dir.iter().all(|p| absolute(p)) {
            return Some(("screenshots_dir", "format"));
        }
        if !self.screenshots_dirs.values().all(|p| absolute(p)) {
            return Some(("screenshots_dirs", "format"));
        }
        [
            ("keep", self.keep as u64, MAX_KEEP),
            ("backup_every_days", self.backup_every_days, MAX_EVERY_DAYS),
            ("update_every", self.update_every, MAX_EVERY_MINUTES),
        ]
        .into_iter()
        .find(|(_, n, max)| !(1..=*max).contains(n))
        .map(|(k, ..)| (k, "range"))
    }
}

fn unknown_key(key: &str) -> Coded {
    Coded::not_found("config_key", key, tr!("config.unknown_key", key = key))
}

/// A command line value: JSON when it is JSON (`10`, `true`, `["mon"]`, `null`), else a plain string (`game_exit`, `10:00`, `D:\Shots`).
pub fn parse_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

pub const APP_NAME: &str = "WoW Tools";
/// The first name of the app; its folders are moved over by `migrate_legacy`.
const LEGACY_NAME: &str = "wtf_sync";

/// A file of this name next to the exe makes it portable: the config and the data live in
/// `config\` and `data\` beside it instead of `%APPDATA%` and `%LOCALAPPDATA%`.
pub const PORTABLE_MARKER: &str = "portable.txt";

/// The exe's folder when it is portable, read once.
pub fn portable_dir() -> Option<&'static Path> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?;
        dir.join(PORTABLE_MARKER).is_file().then(|| dir.to_path_buf())
    })
    .as_deref()
}

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", APP_NAME)
}

pub fn default_config_path() -> PathBuf {
    if let Some(dir) = portable_dir() {
        return dir.join("config").join("config.toml");
    }
    project_dirs()
        .map(|d| d.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("wow-tools.toml"))
}

pub fn data_dir() -> PathBuf {
    if let Some(dir) = portable_dir() {
        return dir.join("data");
    }
    project_dirs().map(|d| d.data_local_dir().to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_log_path() -> PathBuf {
    data_dir().join("wow-tools.log")
}

/// Moves the config and snapshots of the `wtf_sync` days to the WoW Tools folders, once,
/// and points a config `backup_dir` that was in the old folder to the new one.
pub fn migrate_legacy() -> Result<()> {
    if portable_dir().is_some() {
        return Ok(());
    }
    let (Some(new), Some(old)) = (project_dirs(), ProjectDirs::from("", "", LEGACY_NAME)) else {
        return Ok(());
    };
    let roots = |d: &ProjectDirs| {
        (
            d.config_dir().parent().map(Path::to_path_buf),
            d.data_local_dir().parent().map(Path::to_path_buf),
        )
    };
    let (old_roaming, old_local) = roots(&old);
    let (new_roaming, new_local) = roots(&new);
    for (from, to) in [(&old_roaming, &new_roaming), (&old_local, &new_local)] {
        if let (Some(from), Some(to)) = (from, to)
            && from.is_dir()
            && !to.exists()
        {
            fs::rename(from, to).with_context(|| tr!("config.migrate_failed", from = from.display(), to = to.display()))?;
        }
    }
    let log = data_dir().join("wtf_sync.log");
    if log.exists() && !default_log_path().exists() {
        fs::rename(&log, default_log_path())?;
    }
    let config_path = default_config_path();
    if let (Some(old_local), Some(new_local)) = (old_local, new_local)
        && config_path.exists()
    {
        let mut cfg = Config::load(&config_path)?;
        if let Ok(rest) = cfg.backup_dir.strip_prefix(&old_local) {
            cfg.backup_dir = new_local.join(rest);
            cfg.save(&config_path)?;
        }
    }
    Ok(())
}

fn parse_time(s: &str, default_hour: u32) -> NaiveTime {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").unwrap_or(NaiveTime::from_hms_opt(default_hour, 0, 0).unwrap())
}

/// The latest moment of `days` at `time` that is at or before `now` (within the last week).
fn last_scheduled(days: &[Day], time: NaiveTime, now: NaiveDateTime) -> Option<NaiveDateTime> {
    (0..8)
        .filter_map(|back| now.date().checked_sub_days(Days::new(back)))
        .filter(|d| days.contains(&Day::of(*d)))
        .map(|d| d.and_time(time))
        .find(|t| *t <= now)
}

/// A scheduled moment has passed and nothing was done since; this also catches up a moment
/// missed while the computer was off.
fn due(days: &[Day], time: NaiveTime, now: NaiveDateTime, last: Option<NaiveDateTime>) -> bool {
    last_scheduled(days, time, now).is_some_and(|at| last.is_none_or(|l| l < at))
}
/// "every 30 min", "every hour", "every 6 h".
pub fn every_text(minutes: u64) -> String {
    match minutes {
        60 => tr!("schedule.every_hour").into(),
        m if m.is_multiple_of(60) => tr!("schedule.every_hours", n = m / 60),
        m => tr!("schedule.every_minutes", n = m),
    }
}

/// "every 3 days", "every 2 weeks" (a whole number of weeks is said in weeks).
pub fn every_days_text(days: u64) -> String {
    match days.max(1) {
        7 => tr!("schedule.every_week").into(),
        d if d.is_multiple_of(7) => tr!("schedule.every_weeks", n = d / 7),
        d => tr!("schedule.every_days", n = d),
    }
}

/// "daily", "on weekdays", "Mon, Wed, Fri", ...
pub fn days_text(days: &[Day]) -> String {
    let set: Vec<Day> = Day::ALL.into_iter().filter(|d| days.contains(d)).collect();
    match set.as_slice() {
        [] => tr!("schedule.no_days").into(),
        s if s.len() == 7 => tr!("schedule.daily").into(),
        [Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri] => tr!("schedule.weekdays").into(),
        [Day::Sat, Day::Sun] => tr!("schedule.weekends").into(),
        s => s.iter().map(|d| d.short()).collect::<Vec<_>>().join(", "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn old_interval_config_still_loads() {
        let cfg: Config = toml::from_str("interval_minutes = 30\nkeep = 10\n").unwrap();
        assert_eq!(cfg.keep, 10);
        assert_eq!(cfg.backup_when, When::GameExit);
    }

    #[test]
    fn schedule_text_and_days() {
        let cfg: Config = toml::from_str("backup_when = 'schedule'\nbackup_days = ['mon', 'fri']\nbackup_time = '09:30'\n").unwrap();
        assert_eq!(cfg.schedule_text(), "По расписанию: Пн, Пт в 09:30");
        assert_eq!(days_text(&Day::ALL), "ежедневно");
        assert_eq!(days_text(&[Day::Sun, Day::Sat]), "по выходным");
        assert_eq!(days_text(&Day::ALL[..5]), "по будням");
        let broken: Config = toml::from_str("backup_time = 'утром'\n").unwrap();
        assert_eq!(broken.time(), NaiveTime::from_hms_opt(10, 0, 0).unwrap());
    }

    fn at(day: u32, h: u32, m: u32) -> NaiveDateTime {
        // October 2026: the 12th is a Monday.
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn schedule_runs_at_its_time_and_catches_up() {
        let cfg: Config = toml::from_str("backup_when = 'schedule'\nbackup_days = ['mon', 'thu']\nbackup_time = '10:00'\n").unwrap();
        let last_week = Some(at(8, 10, 5)); // Thursday's snapshot
        // Monday before 10:00: nothing yet; at 10:00: due.
        assert!(!cfg.schedule_due(at(12, 9, 59), last_week));
        assert!(cfg.schedule_due(at(12, 10, 0), last_week));
        // Done at 10:01 — not again that day.
        assert!(!cfg.schedule_due(at(12, 18, 0), Some(at(12, 10, 1))));
        // The computer was off on Monday and is turned on on Tuesday: Monday's backup is caught up.
        assert!(cfg.schedule_due(at(13, 8, 0), last_week));
        // A manual snapshot after Monday 10:00 counts.
        assert!(!cfg.schedule_due(at(13, 8, 0), Some(at(12, 21, 0))));
        // Thursday 10:00 is the next one.
        assert!(!cfg.schedule_due(at(15, 9, 0), Some(at(12, 10, 1))));
        assert!(cfg.schedule_due(at(15, 10, 30), Some(at(12, 10, 1))));
        // Never backed up: the last scheduled moment has passed, so right away.
        assert!(cfg.schedule_due(at(14, 12, 0), None));
    }

    #[test]
    fn game_exit_mode_has_no_schedule() {
        let cfg = Config::default();
        assert!(!cfg.schedule_due(at(12, 23, 0), None));
    }

    #[test]
    fn addon_updates_have_their_own_schedule() {
        let mut cfg = Config::default();
        assert!(!cfg.update_due(at(12, 23, 0), None, at(1, 0, 0)), "off by default");
        cfg.update_addons = true;
        cfg.update_days = vec![Day::Tue];
        cfg.update_time = "09:00".into();
        // Monday: last Tuesday 09:00 was already done.
        assert!(!cfg.update_due(at(12, 12, 0), Some(at(6, 9, 1)), at(1, 0, 0)));
        assert!(!cfg.update_due(at(13, 8, 59), Some(at(6, 9, 1)), at(1, 0, 0)));
        assert!(cfg.update_due(at(13, 9, 0), Some(at(6, 9, 1)), at(1, 0, 0)));
        // Backups on their own schedule do not count as an update.
        cfg.backup_when = When::Schedule;
        assert!(cfg.update_due(at(13, 9, 0), Some(at(6, 9, 1)), at(1, 0, 0)));
    }

    #[test]
    fn addon_updates_every_n_minutes() {
        let cfg: Config = toml::from_str("update_addons = true\nupdate_when = 'interval'\nupdate_every = 120\n").unwrap();
        assert_eq!(cfg.update_text(), "каждые 2 ч");
        assert!(cfg.update_due(at(12, 9, 0), None, at(1, 0, 0)), "never updated: right away");
        assert!(!cfg.update_due(at(12, 10, 59), Some(at(12, 9, 0)), at(1, 0, 0)));
        assert!(cfg.update_due(at(12, 11, 0), Some(at(12, 9, 0)), at(1, 0, 0)));
        assert_eq!(every_text(60), "каждый час");
        assert_eq!(every_text(30), "каждые 30 мин");
        // Configs from before the interval mode keep their schedule.
        let old: Config = toml::from_str("update_addons = true\nupdate_time = '16:00'\n").unwrap();
        assert_eq!(old.update_when, UpdateWhen::Schedule);
    }

    #[test]
    fn backup_every_n_days() {
        let cfg: Config = toml::from_str("backup_when = 'every_days'\nbackup_every_days = 14\nbackup_time = '10:00'\n").unwrap();
        assert_eq!(cfg.schedule_text(), "Периодически: раз в 2 недели в 10:00");
        let last = Some(at(12, 10, 1));
        // Thirteen days later: not yet; the fourteenth day at 10:00: due.
        assert!(!cfg.schedule_due(at(25, 23, 0), last));
        assert!(!cfg.schedule_due(at(26, 9, 59), last));
        assert!(cfg.schedule_due(at(26, 10, 0), last));
        // The computer was off that day: caught up the next morning.
        assert!(cfg.schedule_due(at(27, 8, 0), last));
        // Never backed up: right away.
        assert!(cfg.schedule_due(at(12, 12, 0), None));
        assert_eq!(every_days_text(3), "каждые 3 дн.");
        assert_eq!(every_days_text(7), "раз в неделю");
    }

    #[test]
    fn addon_update_at_startup() {
        let cfg: Config = toml::from_str("update_addons = true\nupdate_when = 'startup'\n").unwrap();
        let started = at(12, 8, 0);
        // A minute for the sign-in to settle, then once.
        assert!(!cfg.update_due(at(12, 8, 0), None, started));
        assert!(cfg.update_due(at(12, 8, 1), Some(at(11, 20, 0)), started));
        assert!(!cfg.update_due(at(12, 9, 0), Some(at(12, 8, 1)), started), "done since the start");
        // A restart soon after an update (a self-update) does not update again.
        assert!(!cfg.update_due(at(12, 8, 30), Some(at(12, 7, 30)), started));
    }

    #[test]
    fn screenshot_folders_per_flavor_round_trip() {
        let mut cfg = Config::default();
        cfg.screenshots_dirs.insert("_retail_".into(), PathBuf::from(r"D:\Shots\Retail"));
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.screenshots_dirs.get("_retail_"), Some(&PathBuf::from(r"D:\Shots\Retail")));
    }

    #[test]
    fn panel_tabs_order_and_hiding() {
        use PanelTab::*;
        // Unknown names are skipped, missing tabs follow in the default order.
        let mut cfg: Config = toml::from_str("panel_tabs = ['profiles', 'mail', 'backup']\nhidden_tabs = ['cleanup', 'x']\n").unwrap();
        assert_eq!(cfg.tab_order(), [Profiles, Backup, Addons, Cleanup, Macros]);
        assert_eq!(cfg.shown_tabs(), [Profiles, Backup, Addons, Macros]);

        // The saved name of the new tab, and an old four-tab list that gets it appended.
        let old: Config = toml::from_str("panel_tabs = ['backup', 'addons', 'cleanup', 'profiles']\nhidden_tabs = ['macros']\n").unwrap();
        assert_eq!((old.tab_order()[4], old.shown_tabs().len()), (Macros, 4));

        // Rearranged in the panel: the hidden tab keeps its place.
        cfg.hidden_tabs = vec![Backup];
        cfg.set_shown_order(&[Cleanup, Profiles, Addons, Macros]);
        assert_eq!(cfg.tab_order(), [Cleanup, Backup, Profiles, Addons, Macros]);
        // Not a rearrangement of the shown tabs (a stale list): ignored.
        cfg.set_shown_order(&[Backup, Cleanup, Profiles, Addons, Macros]);
        assert_eq!(cfg.tab_order(), [Cleanup, Backup, Profiles, Addons, Macros]);

        // The last shown tab cannot be hidden.
        assert!(cfg.toggle_tab(Cleanup) && cfg.toggle_tab(Addons) && cfg.toggle_tab(Macros));
        assert!(!cfg.toggle_tab(Profiles));
        assert_eq!(cfg.shown_tabs(), [Profiles]);
        assert!(cfg.toggle_tab(Backup));
        assert_eq!(cfg.shown_tabs(), [Backup, Profiles]);

        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!((back.tab_order(), back.shown_tabs()), (cfg.tab_order(), cfg.shown_tabs()));
    }

    fn code(e: &anyhow::Error) -> (&'static str, serde_json::Value) {
        crate::errors::classify(e)
    }

    #[test]
    fn set_get_unset_one_key_of_each_kind() {
        let defaults = Config::default();
        let mut cfg = Config::default();
        let cases = [
            ("update_addons", json!(true)),
            ("keep", json!(5)),
            ("backup_time", json!("07:30")),
            ("backup_when", json!("every_days")),
            ("backup_days", json!(["mon", "fri"])),
            ("update_app", json!("curseforge")),
            ("screenshots_dir", json!(r"D:\Shots")),
            ("screenshots_dirs", json!({ "_retail_": r"D:\Retail" })),
            ("hidden_tabs", json!(["macros"])),
            ("language", json!("de")),
            ("update_when", json!("interval")),
            ("flavors", json!(["_classic_era_", "_retail_"])),
        ];
        for (key, value) in cases {
            let before = cfg.image();
            cfg.set_key(key, value.clone()).unwrap_or_else(|e| panic!("{key}: {e:#}"));
            assert_eq!(cfg.get_key(key).unwrap(), value, "{key}");
            // Only that key changed.
            let mut expect = before;
            expect.insert(key.into(), value);
            assert_eq!(cfg.image(), expect, "{key}");
            cfg.unset_key(key).unwrap();
            assert_eq!(cfg.get_key(key).unwrap(), defaults.get_key(key).unwrap(), "{key}");
        }
        assert_eq!(cfg.image(), defaults.image());
    }

    #[test]
    fn a_bad_set_is_refused_and_changes_nothing() {
        let mut cfg = Config::default();
        let bad = [
            ("keep", json!("many"), "type"),
            ("keep", json!(0), "range"),
            ("backup_every_days", json!(0), "range"),
            ("update_every", json!(0), "range"),
            ("backup_when", json!("never"), "type"),
            ("backup_days", json!(["mon", "someday"]), "type"),
            ("backup_time", json!("25:00"), "format"),
            ("update_time", json!("9"), "format"),
            ("flavors", json!(["retail"]), "format"),
            ("update_addons", json!("yes"), "type"),
            ("language", json!("xx"), "type"),
            ("update_when", json!("sometimes"), "type"),
            ("screenshots_dir", json!(5), "type"),
            ("backup_time", json!(null), "type"),
            ("update_every", json!(1e15 as u64), "range"),
            ("update_every", json!(525_601), "range"),
            ("backup_every_days", json!(366), "range"),
            ("keep", json!(10_001), "range"),
            ("hidden_tabs", json!(["macro"]), "type"),
            ("backup_dir", json!(""), "format"),
            ("backup_dir", json!(r"rel\dir"), "format"),
            ("wow_path", json!("games"), "format"),
            ("screenshots_dir", json!("shots"), "format"),
            ("screenshots_dirs", json!({ "_retail_": "shots" }), "format"),
        ];
        for (key, value, reason) in bad {
            let e = cfg.set_key(key, value.clone()).unwrap_err();
            let (c, params) = code(&e);
            assert_eq!(
                (c, params["key"].clone(), params["reason"].clone()),
                ("invalid_args", json!(key), json!(reason)),
                "{key}={value}"
            );
            assert_eq!(cfg.image(), Config::default().image());
        }
        let e = cfg.set_key("nope", json!(1)).unwrap_err();
        assert_eq!(code(&e), ("not_found", json!({ "what": "config_key", "key": "nope" })));
        assert!(cfg.get_key("nope").is_err() && cfg.unset_key("nope").is_err());
    }

    #[test]
    fn an_option_setting_can_be_set_to_null_and_back() {
        let mut cfg = Config::default();
        cfg.set_key("update_app", json!("curseforge")).unwrap();
        cfg.set_key("update_app", Value::Null).unwrap();
        assert!(cfg.update_app.is_none());
    }

    #[test]
    fn command_line_values_are_json_or_text() {
        assert_eq!(parse_value("10"), json!(10));
        assert_eq!(parse_value("true"), json!(true));
        assert_eq!(parse_value("null"), Value::Null);
        assert_eq!(parse_value("[\"mon\"]"), json!(["mon"]));
        assert_eq!(parse_value("10:00"), json!("10:00"));
        assert_eq!(parse_value("game_exit"), json!("game_exit"));
        assert_eq!(parse_value(r"D:\Shots"), json!(r"D:\Shots"));
    }

    #[test]
    fn a_huge_hand_edited_interval_does_not_panic() {
        let cfg = Config {
            update_addons: true,
            backup_when: When::EveryDays,
            update_when: UpdateWhen::Interval,
            update_every: u64::MAX,
            backup_every_days: u64::MAX,
            ..Config::default()
        };
        let now = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap().and_hms_opt(12, 0, 0).unwrap();
        let _ = cfg.update_due(now, Some(now), now);
        let _ = cfg.schedule_due(now, Some(now));
    }

    #[test]
    fn concurrent_saves_never_leave_a_broken_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let (a, b) = (Config { keep: 3, ..Config::default() }, Config { keep: 9, ..Config::default() });
        a.save(&path).unwrap();
        std::thread::scope(|s| {
            for cfg in [&a, &b] {
                let path = &path;
                s.spawn(move || {
                    for _ in 0..200 {
                        // A rename may lose to the other writer's on Windows; the file must still be whole.
                        let _ = cfg.save(path);
                    }
                });
            }
        });
        let keep = Config::load(&path).unwrap().keep;
        assert!(keep == 3 || keep == 9);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn save_replaces_the_file_whole_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("config.toml");
        let mut cfg = Config::default();
        cfg.save(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), toml::to_string_pretty(&cfg).unwrap());
        cfg.set_key("keep", json!(7)).unwrap();
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().keep, 7);
        let names: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("config.toml")]);
    }
}
