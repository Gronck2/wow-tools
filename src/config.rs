use anyhow::{Context, Result};
use chrono::{Datelike, Days, NaiveDate, NaiveDateTime, NaiveTime};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
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

/// Choices of the "update every" stepper, in minutes.
pub const UPDATE_EVERY: &[u64] = &[30, 60, 120, 180, 360, 720];

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
    /// Time of `When::Schedule`, "HH:MM" local.
    pub backup_time: String,
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
    /// Ask GitHub once a day for a newer WoW Tools (`selfupdate`).
    pub check_updates: bool,
    /// Where the cleanup moves screenshots; unset = `Pictures\World of Warcraft`.
    pub screenshots_dir: Option<PathBuf>,
    /// Date folders under it.
    pub screenshots_by: ShotsBy,
    /// A folder per game version ("Retail", "Classic Era") above the date folders.
    pub screenshots_per_flavor: bool,
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
            check_updates: true,
            screenshots_dir: None,
            screenshots_by: ShotsBy::Month,
            screenshots_per_flavor: true,
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
        self.backup_when == When::Schedule && due(&self.backup_days, self.time(), now, last_backup)
    }

    /// `update_time`, or 09:00 when it does not parse.
    pub fn update_at(&self) -> NaiveTime {
        parse_time(&self.update_time, 9)
    }

    /// An automatic addon update is due: by the schedule (same rules as the backups) or when
    /// `update_every` minutes have passed since the last one.
    pub fn update_due(&self, now: NaiveDateTime, last_update: Option<NaiveDateTime>) -> bool {
        if !self.update_addons {
            return false;
        }
        match self.update_when {
            UpdateWhen::Schedule => due(&self.update_days, self.update_at(), now, last_update),
            UpdateWhen::Interval => last_update.is_none_or(|l| now - l >= chrono::Duration::minutes(self.update_every.max(1) as i64)),
        }
    }

    /// "every hour", "on schedule: daily at 09:00", ... (in the interface language)
    pub fn update_text(&self) -> String {
        match self.update_when {
            UpdateWhen::Interval => every_text(self.update_every),
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
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path).with_context(|| tr!("error.read", path = path.display()))?;
        toml::from_str(&text).with_context(|| tr!("config.bad", path = path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, toml::to_string_pretty(self)?).with_context(|| tr!("error.write", path = path.display()))
    }
}

pub const APP_NAME: &str = "WoW Tools";
/// The first name of the app; its folders are moved over by `migrate_legacy`.
const LEGACY_NAME: &str = "wtf_sync";

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", APP_NAME)
}

pub fn default_config_path() -> PathBuf {
    project_dirs()
        .map(|d| d.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("wow-tools.toml"))
}

pub fn data_dir() -> PathBuf {
    project_dirs().map(|d| d.data_local_dir().to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_log_path() -> PathBuf {
    data_dir().join("wow-tools.log")
}

/// Moves the config and snapshots of the `wtf_sync` days to the WoW Tools folders, once,
/// and points a config `backup_dir` that was in the old folder to the new one.
pub fn migrate_legacy() -> Result<()> {
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
        assert!(!cfg.update_due(at(12, 23, 0), None), "off by default");
        cfg.update_addons = true;
        cfg.update_days = vec![Day::Tue];
        cfg.update_time = "09:00".into();
        // Monday: last Tuesday 09:00 was already done.
        assert!(!cfg.update_due(at(12, 12, 0), Some(at(6, 9, 1))));
        assert!(!cfg.update_due(at(13, 8, 59), Some(at(6, 9, 1))));
        assert!(cfg.update_due(at(13, 9, 0), Some(at(6, 9, 1))));
        // Backups on their own schedule do not count as an update.
        cfg.backup_when = When::Schedule;
        assert!(cfg.update_due(at(13, 9, 0), Some(at(6, 9, 1))));
    }

    #[test]
    fn addon_updates_every_n_minutes() {
        let cfg: Config = toml::from_str("update_addons = true\nupdate_when = 'interval'\nupdate_every = 120\n").unwrap();
        assert_eq!(cfg.update_text(), "каждые 2 ч");
        assert!(cfg.update_due(at(12, 9, 0), None), "never updated: right away");
        assert!(!cfg.update_due(at(12, 10, 59), Some(at(12, 9, 0))));
        assert!(cfg.update_due(at(12, 11, 0), Some(at(12, 9, 0))));
        assert_eq!(every_text(60), "каждый час");
        assert_eq!(every_text(30), "каждые 30 мин");
        // Configs from before the interval mode keep their schedule.
        let old: Config = toml::from_str("update_addons = true\nupdate_time = '16:00'\n").unwrap();
        assert_eq!(old.update_when, UpdateWhen::Schedule);
    }
}
