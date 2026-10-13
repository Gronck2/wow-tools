//! The settings window, in the style of the panel: every field of `config.toml` that matters
//! day to day, as cards with switches, steppers and folder pickers. Changes are saved at once;
//! the `watch` loop re-reads the config every round, so they apply without a restart.

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, PAINTSTRUCT,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::config::{self, APP_NAME, Config, Day, ShotsBy, UpdateWhen, When};
use crate::gfx::{self, Align, Painter, R, Surface, Theme};
use crate::i18n::Lang;
use crate::maintenance;
use crate::tray::{self, Action};
use crate::updater::{self, AddonApp};
use crate::{schedule, wow};

const W: f32 = 560.0;
const PAD: f32 = 20.0;
const ROW_H: f32 = 62.0;
const ROW_GAP: f32 = 4.0;
const SECTION_H: f32 = 40.0;
const FOOTER_H: f32 = 44.0;
/// The tab strip at the top; the cards start below it.
const TABS_H: f32 = 52.0;
const CHIP_STEP: f32 = 38.0;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_MOUSEWHEEL: u32 = 0x020A;

/// Choices of the "how many to keep" stepper.
const KEEPS: &[u64] = &[3, 5, 10, 15, 20, 30, 50, 75, 100, 150, 200];

/// What the window shows and edits.
pub struct Model {
    pub cfg: Config,
    /// The install found automatically (used while `cfg.wow_path` is unset).
    pub detected: Option<PathBuf>,
    /// Flavors found in the install in use.
    pub flavors: Vec<wow::Flavor>,
    pub autostart: bool,
    /// Addon manager apps installed (CurseForge, WowUp, ...).
    pub apps: Vec<AddonApp>,
    /// The WeakAuras addon is installed in some flavor.
    pub weakauras: bool,
    /// When addons were last updated through it.
    pub last_update: Option<chrono::NaiveDateTime>,
    /// A line under the cards: the last error, or a note about the last change.
    pub message: Option<(String, bool)>,
    /// Updates of WoW Tools itself.
    pub update: tray::SelfUpdate,
    /// Addons not made for the client version, per flavor ("Retail 12.1.0"); read with the Addons tab.
    pub outdated: Option<Vec<(String, Vec<String>)>>,
}

impl Model {
    pub fn load(path: &Path) -> Model {
        let (cfg, message) = match Config::load(path) {
            Ok(cfg) => (cfg, None),
            Err(e) => (Config::default(), Some((tr!("settings.load_failed", error = format!("{e:#}")), true))),
        };
        let mut m = Model {
            cfg,
            detected: wow::detect_root(),
            flavors: Vec::new(),
            autostart: schedule::logon_command().is_some(),
            apps: AddonApp::installed(),
            weakauras: false,
            last_update: crate::updater::last_run(),
            message,
            update: tray::self_update(),
            outdated: None,
        };
        m.find_flavors();
        m.weakauras = m.flavors.iter().any(|f| f.dir.join(r"Interface\AddOns\WeakAuras").is_dir());
        m
    }

    fn root(&self) -> Option<&Path> {
        self.cfg.wow_path.as_deref().or(self.detected.as_deref())
    }

    fn find_flavors(&mut self) {
        self.flavors = wow::all_flavors(self.root());
    }

    /// The app updates go through (as `updater::chosen`, over the apps found at load).
    fn chosen(&self) -> Option<AddonApp> {
        updater::pick(self.cfg.update_app, &self.apps)
    }

    fn flavor_on(&self, name: &str) -> bool {
        self.cfg.flavors.is_empty() || self.cfg.flavors.iter().any(|f| f == name)
    }

    fn selected_flavors(&self) -> Vec<wow::Flavor> {
        self.flavors.iter().filter(|f| self.flavor_on(&f.name)).cloned().collect()
    }

    /// Reads what `tab` shows from the disk, once: the outdated addons.
    fn read_for(&mut self, tab: Tab) {
        match tab {
            Tab::Addons if self.outdated.is_none() => {
                let list = self
                    .selected_flavors()
                    .iter()
                    .filter_map(|f| {
                        Some((
                            format!("{} {}", f.title(), maintenance::short_version(f.version.as_deref()?)),
                            maintenance::outdated(f),
                        ))
                    })
                    .collect();
                self.outdated = Some(list);
            }
            _ => {}
        }
    }

    /// Forgets what was read from the disk (the game folder or the versions changed).
    fn forget_reads(&mut self) {
        self.outdated = None;
    }
}

// ---------------------------------------------------------------- layout

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Section(&'static str),
    Wow,
    Flavors,
    When,
    Days,
    Time,
    Keep,
    Addons,
    Game,
    Backups,
    UpdateGame,
    UpdateApp,
    WagoWeakAuras,
    WagoUiPacks,
    Updates,
    UpdateWhen,
    UpdateEvery,
    UpdateDays,
    UpdateTime,
    Language,
    Autostart,
    CheckUpdates,
    Version,
    File,
    Outdated,
    ShotsDir,
    ShotsBy,
    ShotsFlavor,
}

/// The pages of the window, switched by the tabs at the top.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Tab {
    #[default]
    General,
    Backup,
    Addons,
    Screenshots,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::General, Tab::Backup, Tab::Addons, Tab::Screenshots];

    fn label(self) -> &'static str {
        match self {
            Tab::General => tr!("settings.tab_general"),
            Tab::Backup => tr!("panel.tab_backup"),
            Tab::Addons => tr!("panel.tab_addons"),
            Tab::Screenshots => tr!("settings.tab_screenshots"),
        }
    }
}

/// The cards of a tab; the days and times only when their schedule is on.
fn rows(m: &Model, tab: Tab) -> Vec<Row> {
    let mut rows = Vec::new();
    match tab {
        Tab::General => rows.extend([
            Row::Section(tr!("settings.section_game")),
            Row::Wow,
            Row::Flavors,
            Row::Section(tr!("settings.section_system")),
            Row::Language,
            Row::Autostart,
            Row::CheckUpdates,
            Row::Version,
            Row::File,
        ]),
        Tab::Backup => {
            rows.extend([Row::Section(tr!("settings.section_schedule")), Row::When]);
            if m.cfg.backup_when == When::Schedule {
                rows.extend([Row::Days, Row::Time]);
            }
            rows.extend([Row::Section(tr!("settings.section_storage")), Row::Keep, Row::Addons, Row::Game, Row::Backups]);
        }
        Tab::Addons => {
            rows.extend([Row::Section(tr!("settings.section_updates")), Row::UpdateApp]);
            if m.chosen() == Some(AddonApp::Wago) {
                rows.extend([Row::WagoWeakAuras, Row::WagoUiPacks]);
            }
            rows.push(Row::UpdateGame);
            rows.push(Row::Updates);
            if m.cfg.update_addons {
                rows.push(Row::UpdateWhen);
                if m.cfg.update_when == UpdateWhen::Interval {
                    rows.push(Row::UpdateEvery);
                }
                if m.cfg.update_when == UpdateWhen::Schedule {
                    rows.extend([Row::UpdateDays, Row::UpdateTime]);
                }
            }
            rows.extend([Row::Section(tr!("cleanup.section_compat")), Row::Outdated]);
        }
        Tab::Screenshots => rows.extend([Row::Section(tr!("settings.section_screenshots")), Row::ShotsDir, Row::ShotsBy, Row::ShotsFlavor]),
    }
    rows
}

/// The tab strip, fixed above the scrolling cards.
fn tabs(f: &Formats) -> Vec<(R, Tab)> {
    let mut x = PAD;
    Tab::ALL
        .iter()
        .map(|&tab| {
            let w = gfx::text_width(&f.tab, tab.label()) + 32.0;
            let r = R { x, y: 10.0, w, h: 36.0 };
            x += w + 4.0;
            (r, tab)
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    /// `None` = follow Windows.
    Lang(Option<Lang>),
    Tab(Tab),
    WowChange,
    WowAuto,
    Flavor(usize),
    WhenExit,
    WhenSchedule,
    Day(usize),
    TimeDown,
    TimeUp,
    KeepDown,
    KeepUp,
    Addons,
    Game,
    BackupsOpen,
    BackupsChange,
    App(AddonApp),
    UpdateGame,
    WagoWeakAuras,
    WagoUiPacks,
    Updates,
    UpdateInterval,
    UpdateSchedule,
    EveryDown,
    EveryUp,
    UpdateDay(usize),
    UpdateTimeDown,
    UpdateTimeUp,
    Autostart,
    CheckUpdates,
    CheckNow,
    InstallUpdate,
    File,
    ShotsChange,
    /// Back to `Pictures\World of Warcraft`.
    ShotsAuto,
    ShotsBy(ShotsBy),
    ShotsFlavor,
}

/// The chips of a card (versions, days): label, on, what a click does.
fn chip_items(kind: Row, m: &Model) -> Vec<(String, bool, Hit)> {
    match kind {
        Row::Flavors => m
            .flavors
            .iter()
            .enumerate()
            .map(|(i, fl)| (fl.title(), m.flavor_on(&fl.name), Hit::Flavor(i)))
            .collect(),
        Row::Days => Day::ALL
            .iter()
            .enumerate()
            .map(|(i, d)| (d.short().to_string(), m.cfg.backup_days.contains(d), Hit::Day(i)))
            .collect(),
        Row::UpdateApp => m.apps.iter().map(|&a| (a.title().to_string(), m.chosen() == Some(a), Hit::App(a))).collect(),
        Row::Language => std::iter::once((tr!("settings.language_auto").to_string(), m.cfg.language.is_none(), Hit::Lang(None)))
            .chain(
                Lang::ALL
                    .iter()
                    .map(|&l| (l.native().to_string(), m.cfg.language == Some(l), Hit::Lang(Some(l)))),
            )
            .collect(),
        Row::ShotsBy => ShotsBy::ALL
            .iter()
            .map(|&b| (shots_by_label(b).to_string(), m.cfg.screenshots_by == b, Hit::ShotsBy(b)))
            .collect(),
        Row::UpdateDays => Day::ALL
            .iter()
            .enumerate()
            .map(|(i, d)| (d.short().to_string(), m.cfg.update_days.contains(d), Hit::UpdateDay(i)))
            .collect(),
        _ => Vec::new(),
    }
}

/// Chips flowing in lines under the card's title.
fn chips(r: R, kind: Row, m: &Model, f: &Formats) -> Vec<(R, String, bool, Hit)> {
    let (left, right) = (r.x + 56.0, r.x + r.w - 16.0);
    let (mut x, mut y) = (left, r.y + ROW_H - 4.0);
    let mut out = Vec::new();
    for (label, on, hit) in chip_items(kind, m) {
        let w = (gfx::text_width(&f.button, &label) + 28.0).max(48.0);
        if x > left && x + w > right {
            x = left;
            y += CHIP_STEP;
        }
        out.push((R { x, y, w, h: 30.0 }, label, on, hit));
        x += w + 8.0;
    }
    out
}

fn row_height(row: Row, m: &Model, f: &Formats) -> f32 {
    match row {
        Row::Section(_) => SECTION_H,
        Row::Flavors | Row::Days | Row::UpdateDays | Row::UpdateApp | Row::Language | Row::ShotsBy => {
            let probe = R {
                x: PAD,
                y: 0.0,
                w: W - 2.0 * PAD,
                h: ROW_H,
            };
            chips(probe, row, m, f).last().map_or(ROW_H, |(c, ..)| c.y + c.h + 16.0)
        }
        Row::Outdated => match outdated_blocks(m) {
            blocks if blocks.is_empty() => ROW_H,
            blocks => BLOCKS_Y + blocks_height(&blocks, f) + 8.0,
        },
        _ => ROW_H,
    }
}

/// How a paragraph of the outdated card looks.
#[derive(Clone, Copy)]
enum Tone {
    Warn,
    Text,
    Faint,
}

/// Where the paragraphs of the outdated card start, under its title.
const BLOCKS_Y: f32 = 34.0;
const BLOCK_GAP: f32 = 6.0;
const BLOCK_W: f32 = W - 2.0 * PAD - 56.0 - 16.0;

/// The outdated card spelled out under its title: for every game version with such addons a
/// heading and all their names, then the versions without any and a note. Empty when none is outdated.
fn outdated_blocks(m: &Model) -> Vec<(String, Tone)> {
    let list = m.outdated.as_deref().unwrap_or_default();
    if list.iter().all(|(_, names)| names.is_empty()) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (flavor, names) in list.iter().filter(|(_, names)| !names.is_empty()) {
        out.push((tr!("cleanup.outdated_head", flavor = flavor, n = names.len()), Tone::Warn));
        out.push((names.join(", "), Tone::Text));
    }
    for (flavor, _) in list.iter().filter(|(_, names)| names.is_empty()) {
        out.push((tr!("cleanup.outdated_ok", flavor = flavor), Tone::Faint));
    }
    out.push((tr!("cleanup.outdated_note").to_string(), Tone::Faint));
    out
}

fn blocks_height(blocks: &[(String, Tone)], f: &Formats) -> f32 {
    blocks.iter().map(|(s, _)| gfx::text_height(&f.wrap, s, BLOCK_W) + BLOCK_GAP).sum()
}

fn layout(m: &Model, f: &Formats, tab: Tab) -> Vec<(R, Row)> {
    place_rows(rows(m, tab), m, f)
}

fn place_rows(rows: Vec<Row>, m: &Model, f: &Formats) -> Vec<(R, Row)> {
    let mut y = TABS_H;
    rows.into_iter()
        .map(|row| {
            let h = row_height(row, m, f);
            let r = R {
                x: PAD,
                y,
                w: W - 2.0 * PAD,
                h,
            };
            y += h + if matches!(row, Row::Section(_)) { 0.0 } else { ROW_GAP };
            (r, row)
        })
        .collect()
}

fn bottom(rows: &[(R, Row)]) -> f32 {
    rows.last().map_or(TABS_H, |(r, _)| r.y + r.h)
}

fn content_height(m: &Model, f: &Formats, tab: Tab) -> f32 {
    bottom(&layout(m, f, tab)) + FOOTER_H
}

enum Ctl {
    /// The whole card is a click target (switch rows); drawn as the card's hover.
    Card,
    Button(&'static str),
    Chip(String, bool),
    /// One half of a two-way choice; `true` when selected.
    Segment(&'static str, bool),
    /// The value box of a stepper; its arrows are separate controls.
    Value(String),
    Arrow(&'static str, bool),
    Switch(bool),
}

struct Control {
    r: R,
    hit: Option<Hit>,
    ctl: Ctl,
}

/// Switches day `i` in `days` (kept in week order); refuses to leave none.
fn toggle_day(days: &mut Vec<Day>, i: usize) -> bool {
    let day = Day::ALL[i];
    let mut set: Vec<Day> = Day::ALL.into_iter().filter(|d| days.contains(d)).collect();
    match set.iter().position(|&d| d == day) {
        Some(pos) => {
            set.remove(pos);
        }
        None => set.push(day),
    }
    if set.is_empty() {
        return false;
    }
    *days = Day::ALL.into_iter().filter(|d| set.contains(d)).collect();
    true
}

/// Moves a time of day by `minutes`, snapping to the half hour and wrapping around midnight.
fn shift_time(t: chrono::NaiveTime, minutes: i64) -> chrono::NaiveTime {
    use chrono::Timelike;
    let cur = i64::from(t.hour() * 60 + t.minute());
    let snapped = if minutes > 0 {
        cur.div_euclid(30) * 30
    } else {
        (cur + 29).div_euclid(30) * 30
    };
    let m = (snapped + minutes).rem_euclid(24 * 60) as u32;
    chrono::NaiveTime::from_hms_opt(m / 60, m % 60, 0).unwrap()
}

/// The value of the "update every" stepper: "30 min", "2 h".
fn shots_by_label(by: ShotsBy) -> &'static str {
    match by {
        ShotsBy::Year => tr!("settings.shots_by_year"),
        ShotsBy::Month => tr!("settings.shots_by_month"),
        ShotsBy::Day => tr!("settings.shots_by_day"),
        ShotsBy::None => tr!("settings.shots_by_none"),
    }
}

fn every_label(minutes: u64) -> String {
    if minutes >= 60 && minutes.is_multiple_of(60) {
        tr!("settings.hours", n = minutes / 60)
    } else {
        tr!("settings.minutes", n = minutes)
    }
}

fn value_of(list: &[u64], cur: u64, up: bool) -> Option<u64> {
    if up {
        list.iter().copied().find(|&v| v > cur)
    } else {
        list.iter().copied().rev().find(|&v| v < cur)
    }
}

/// The controls at the right end of a card, right to left.
fn controls(row: R, kind: Row, m: &Model, f: &Formats) -> Vec<Control> {
    let mut out = Vec::new();
    let mut right = row.x + row.w - 16.0;
    let mut place = |w: f32, h: f32| {
        let r = R {
            x: right - w,
            y: row.cy() - h / 2.0,
            w,
            h,
        };
        right -= w + 8.0;
        r
    };
    let button_w = |s: &str| (gfx::text_width(&f.button, s) + 28.0).max(84.0);
    let mut button = |out: &mut Vec<Control>, label: &'static str, hit: Hit| {
        out.push(Control {
            r: place(button_w(label), 32.0),
            hit: Some(hit),
            ctl: Ctl::Button(label),
        })
    };
    let switch = |out: &mut Vec<Control>, on: bool, hit: Hit| {
        out.push(Control {
            r: row,
            hit: Some(hit),
            ctl: Ctl::Card,
        });
        out.push(Control {
            r: R {
                x: row.x + row.w - 56.0,
                y: row.cy() - 10.0,
                w: 40.0,
                h: 20.0,
            },
            hit: None,
            ctl: Ctl::Switch(on),
        });
    };
    let stepper = |out: &mut Vec<Control>, value: String, down: Option<Hit>, up: Option<Hit>| {
        let r = R {
            x: row.x + row.w - 16.0 - 156.0,
            y: row.cy() - 16.0,
            w: 156.0,
            h: 32.0,
        };
        out.push(Control {
            r,
            hit: None,
            ctl: Ctl::Value(value),
        });
        for (x, hit, icon) in [(r.x, down, "\u{E949}"), (r.x + r.w - 36.0, up, "\u{E948}")] {
            out.push(Control {
                r: R { x, w: 36.0, ..r },
                hit,
                ctl: Ctl::Arrow(icon, hit.is_some()),
            });
        }
    };
    match kind {
        Row::Section(_) => {}
        Row::Wow => {
            button(&mut out, tr!("settings.change"), Hit::WowChange);
            if m.cfg.wow_path.is_some() {
                button(&mut out, tr!("settings.auto"), Hit::WowAuto);
            }
        }
        Row::Flavors | Row::Days | Row::UpdateDays | Row::UpdateApp | Row::Language | Row::ShotsBy => {
            for (r, label, on, hit) in chips(row, kind, m, f) {
                out.push(Control {
                    r,
                    hit: Some(hit),
                    ctl: Ctl::Chip(label, on),
                });
            }
        }
        Row::When | Row::UpdateWhen => {
            let segments = if kind == Row::When {
                let schedule = m.cfg.backup_when == When::Schedule;
                [
                    (tr!("settings.by_schedule"), schedule, Hit::WhenSchedule),
                    (tr!("settings.after_game"), !schedule, Hit::WhenExit),
                ]
            } else {
                let schedule = m.cfg.update_when == UpdateWhen::Schedule;
                [
                    (tr!("settings.by_schedule"), schedule, Hit::UpdateSchedule),
                    (tr!("settings.periodically"), !schedule, Hit::UpdateInterval),
                ]
            };
            // Placed right to left.
            for (label, selected, hit) in segments {
                out.push(Control {
                    r: place(gfx::text_width(&f.button, label) + 28.0, 32.0),
                    hit: (!selected).then_some(hit),
                    ctl: Ctl::Segment(label, selected),
                });
            }
        }
        Row::UpdateEvery => {
            let cur = m.cfg.update_every;
            let down = value_of(config::UPDATE_EVERY, cur, false).map(|_| Hit::EveryDown);
            let up = value_of(config::UPDATE_EVERY, cur, true).map(|_| Hit::EveryUp);
            stepper(&mut out, every_label(cur), down, up);
        }
        Row::Time => stepper(&mut out, m.cfg.time().format("%H:%M").to_string(), Some(Hit::TimeDown), Some(Hit::TimeUp)),
        Row::Keep => {
            let cur = m.cfg.keep as u64;
            let down = value_of(KEEPS, cur, false).map(|_| Hit::KeepDown);
            let up = value_of(KEEPS, cur, true).map(|_| Hit::KeepUp);
            stepper(&mut out, cur.to_string(), down, up);
        }
        Row::Addons => switch(&mut out, m.cfg.include_addons, Hit::Addons),
        Row::Game => switch(&mut out, m.cfg.skip_while_running, Hit::Game),
        Row::Backups => {
            button(&mut out, tr!("settings.change"), Hit::BackupsChange);
            button(&mut out, tr!("settings.open"), Hit::BackupsOpen);
        }
        Row::Updates => switch(&mut out, m.cfg.update_addons, Hit::Updates),
        Row::UpdateGame => switch(&mut out, m.cfg.update_skip_in_game, Hit::UpdateGame),
        Row::WagoWeakAuras => switch(&mut out, m.cfg.wago_weakauras, Hit::WagoWeakAuras),
        Row::WagoUiPacks => switch(&mut out, m.cfg.wago_ui_packs, Hit::WagoUiPacks),
        Row::UpdateTime => stepper(
            &mut out,
            m.cfg.update_at().format("%H:%M").to_string(),
            Some(Hit::UpdateTimeDown),
            Some(Hit::UpdateTimeUp),
        ),
        Row::Autostart => switch(&mut out, m.autostart, Hit::Autostart),
        Row::CheckUpdates => switch(&mut out, m.cfg.check_updates, Hit::CheckUpdates),
        Row::Version => {
            let u = &m.update;
            if u.available.is_some() && !u.installing {
                button(&mut out, tr!("self_update.install"), Hit::InstallUpdate);
            } else if u.available.is_none() && !u.checking {
                button(&mut out, tr!("self_update.check_now"), Hit::CheckNow);
            }
        }
        Row::File => button(&mut out, tr!("settings.open"), Hit::File),
        Row::ShotsDir => {
            button(&mut out, tr!("settings.change"), Hit::ShotsChange);
            if m.cfg.screenshots_dir.is_some() {
                button(&mut out, tr!("settings.auto"), Hit::ShotsAuto);
            }
        }
        Row::ShotsFlavor => switch(&mut out, m.cfg.screenshots_per_flavor, Hit::ShotsFlavor),
        Row::Outdated => {}
    }
    out
}

/// `y` is in window coordinates; the cards below the tabs are scrolled by `scroll`.
fn hit_test(m: &Model, f: &Formats, tab: Tab, x: f32, y: f32, scroll: f32) -> Option<Hit> {
    if y < TABS_H {
        return tabs(f).into_iter().find(|(r, t)| *t != tab && r.contains(x, y)).map(|(_, t)| Hit::Tab(t));
    }
    let y = y + scroll;
    // Buttons on a card win over the card itself.
    let mut found = None;
    for (r, kind) in layout(m, f, tab) {
        for c in controls(r, kind, m, f) {
            if let Some(hit) = c.hit.filter(|_| c.r.contains(x, y)) {
                if !matches!(c.ctl, Ctl::Card) {
                    return Some(hit);
                }
                found = Some(hit);
            }
        }
    }
    found
}

// ---------------------------------------------------------------- drawing

struct Formats {
    tab: IDWriteTextFormat,
    tab_on: IDWriteTextFormat,
    section: IDWriteTextFormat,
    title: IDWriteTextFormat,
    sub: IDWriteTextFormat,
    wrap: IDWriteTextFormat,
    path: IDWriteTextFormat,
    button: IDWriteTextFormat,
    value: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    icon_small: IDWriteTextFormat,
}

fn formats() -> gfx::Result<Formats> {
    let dw = gfx::dwrite()?;
    Ok(Formats {
        tab: gfx::text_format(&dw, gfx::UI_FONT, 14.0, false, Align::Center)?,
        tab_on: gfx::text_format(&dw, gfx::UI_FONT, 14.0, true, Align::Center)?,
        section: gfx::text_format(&dw, gfx::UI_FONT, 14.0, true, Align::Left)?,
        title: gfx::text_format(&dw, gfx::UI_FONT, 14.0, false, Align::Left)?,
        sub: gfx::text_format(&dw, gfx::UI_FONT, 12.0, false, Align::Left)?,
        wrap: gfx::wrap_format(&dw, 12.0)?,
        path: gfx::path_format(&dw, 12.0)?,
        button: gfx::text_format(&dw, gfx::UI_FONT, 13.0, false, Align::Center)?,
        value: gfx::text_format(&dw, gfx::UI_FONT, 14.0, false, Align::Center)?,
        icon: gfx::text_format(&dw, gfx::ICON_FONT, 18.0, false, Align::Center)?,
        icon_small: gfx::text_format(&dw, gfx::ICON_FONT, 12.0, false, Align::Center)?,
    })
}

fn path_text(p: &Path) -> String {
    p.display().to_string()
}

/// Icon, title and subtitle of a card; the subtitle may be an error.
fn texts(kind: Row, m: &Model) -> (&'static str, &'static str, String, bool) {
    match kind {
        Row::Section(_) => ("", "", String::new(), false),
        Row::Wow => match (&m.cfg.wow_path, &m.detected) {
            (Some(p), _) => ("\u{E7FC}", tr!("settings.wow"), path_text(p), m.flavors.is_empty()),
            (None, Some(p)) => ("\u{E7FC}", tr!("settings.wow"), tr!("settings.wow_found", path = path_text(p)), false),
            (None, None) => ("\u{E7FC}", tr!("settings.wow"), tr!("settings.wow_missing").into(), true),
        },
        Row::Flavors => {
            let sub = if m.flavors.is_empty() {
                tr!("settings.flavors_none").to_string()
            } else {
                tr!("settings.flavors_sub").to_string()
            };
            ("\u{E8F1}", tr!("settings.flavors"), sub, m.flavors.is_empty() && m.root().is_some())
        }
        Row::When => {
            let sub = match m.cfg.backup_when {
                When::GameExit => tr!("settings.when_exit_sub"),
                When::Schedule => tr!("settings.when_schedule_sub"),
            };
            ("\u{E823}", tr!("settings.when"), sub.into(), false)
        }
        Row::Time => ("\u{E121}", tr!("settings.time"), tr!("settings.time_sub").into(), false),
        Row::UpdateApp => {
            let (sub, error) = match (m.apps.len(), m.chosen()) {
                (0, _) => (tr!("settings.apps_none").into(), true),
                (_, Some(AddonApp::Wago)) if !m.cfg.wago_weakauras && !m.cfg.wago_ui_packs => (tr!("settings.wago_all_off").to_string(), false),
                (_, Some(AddonApp::Wago)) => (tr!("settings.wago_only_extras").to_string(), false),
                (1, _) => (tr!("settings.app_single").to_string(), false),
                _ => (tr!("settings.app_one").to_string(), false),
            };
            ("\u{E71D}", tr!("settings.app"), sub, error)
        }
        Row::WagoWeakAuras => {
            let sub = if m.weakauras {
                tr!("settings.weakauras_sub")
            } else {
                tr!("settings.weakauras_missing")
            };
            ("\u{E945}", "WeakAuras", sub.into(), false)
        }
        Row::WagoUiPacks => ("\u{E8A1}", tr!("wago.ui_packs"), tr!("settings.ui_packs_sub").into(), false),
        Row::UpdateGame => ("\u{E7FC}", tr!("settings.update_game"), tr!("settings.update_game_sub").into(), false),
        Row::Updates => {
            let title = tr!("settings.updates");
            let Some(app) = m.chosen() else {
                return ("\u{E895}", title, tr!("settings.updates_no_app").into(), true);
            };
            let how = if app == AddonApp::Wago {
                tr!("settings.how_minimized")
            } else {
                tr!("settings.how_hidden")
            };
            let mut sub = tr!("settings.updates_sub", app = app.title(), how = how);
            if let Some(t) = m.last_update {
                sub = tr!("settings.last_run", time = tray::format_time(t));
            }
            ("\u{E895}", title, sub, false)
        }
        Row::UpdateDays => {
            let none = m.cfg.update_days.is_empty();
            let sub = tr!("settings.update_days_sub", days = config::days_text(&m.cfg.update_days));
            ("\u{E787}", tr!("settings.update_days"), sub, none)
        }
        Row::UpdateTime => ("\u{E121}", tr!("settings.update_time"), tr!("settings.update_time_sub").into(), false),
        Row::UpdateWhen => {
            let sub = match m.cfg.update_when {
                UpdateWhen::Interval => tr!("settings.update_interval_sub"),
                UpdateWhen::Schedule => tr!("settings.when_schedule_sub"),
            };
            ("\u{E823}", tr!("settings.update_when"), sub.into(), false)
        }
        Row::UpdateEvery => ("\u{E916}", tr!("settings.update_every"), tr!("settings.update_every_sub").into(), false),
        Row::Days => {
            let none = m.cfg.backup_days.is_empty();
            let sub = if none {
                tr!("settings.days_none").to_string()
            } else {
                tr!("settings.days_sub", days = config::days_text(&m.cfg.backup_days))
            };
            ("\u{E787}", tr!("settings.days"), sub, none)
        }
        Row::Keep => ("\u{E81C}", tr!("settings.keep"), tr!("settings.keep_sub").into(), false),
        Row::Addons => ("\u{EA86}", tr!("settings.addons"), tr!("settings.addons_sub").into(), false),
        Row::Game => ("\u{E769}", tr!("settings.game"), tr!("settings.game_sub").into(), false),
        Row::Backups => ("\u{E8B7}", tr!("menu.backups_folder"), path_text(&m.cfg.backup_dir), false),
        Row::Language => {
            let sub = match m.cfg.language {
                None => tr!("settings.language_system", lang = Lang::system().native()),
                Some(_) => tr!("settings.language_sub").to_string(),
            };
            ("\u{E774}", tr!("settings.language"), sub, false)
        }
        Row::Autostart => ("\u{E7E8}", tr!("panel.autostart"), tr!("settings.autostart_sub").into(), false),
        Row::CheckUpdates => ("\u{E777}", tr!("self_update.check"), tr!("self_update.check_sub").into(), false),
        Row::Version => {
            let u = &m.update;
            let (sub, error) = match (&u.available, &u.error) {
                (Some(r), _) if u.installing => (tr!("self_update.installing", version = &r.version), false),
                _ if u.checking => (tr!("self_update.checking").to_string(), false),
                (Some(r), _) => (tr!("self_update.available", version = &r.version), false),
                (None, Some(e)) => (tr!("self_update.check_error", error = e), true),
                (None, None) => match u.checked {
                    Some(t) => (tr!("self_update.latest", time = tray::format_time(t)), false),
                    None => (tr!("self_update.never").to_string(), false),
                },
            };
            ("\u{E946}", concat!("WoW Tools ", env!("CARGO_PKG_VERSION")), sub, error)
        }
        Row::File => ("\u{E8A5}", tr!("settings.file"), tr!("settings.file_sub").into(), false),
        Row::ShotsDir => ("\u{E722}", tr!("settings.shots_dir"), path_text(&maintenance::screenshots_dir(&m.cfg)), false),
        Row::ShotsBy => ("\u{E787}", tr!("settings.shots_by"), tr!("settings.shots_by_sub").into(), false),
        Row::ShotsFlavor => ("\u{E8F1}", tr!("settings.shots_flavor"), tr!("settings.shots_flavor_sub").into(), false),
        Row::Outdated => {
            // With outdated addons the card lists them in full (`outdated_blocks`) instead.
            let list = m.outdated.as_deref().unwrap_or_default();
            let sub = if list.is_empty() {
                tr!("cleanup.outdated_unknown").to_string()
            } else {
                let flavors: Vec<&str> = list.iter().map(|(f, _)| f.as_str()).collect();
                tr!("cleanup.outdated_all_ok", flavors = flavors.join(", "))
            };
            let icon = if outdated_blocks(m).is_empty() { "\u{E73E}" } else { "\u{E7BA}" };
            (icon, tr!("cleanup.outdated"), sub, false)
        }
    }
}

fn draw(p: &Painter, m: &Model, t: &Theme, f: &Formats, tab: Tab, hover: Option<Hit>, scroll: f32) {
    p.clear(t.bg);
    p.offset(0.0, -scroll);
    let cards = layout(m, f, tab);
    for &(r, kind) in &cards {
        if let Row::Section(label) = kind {
            p.text(
                label,
                &f.section,
                R {
                    y: r.y + 14.0,
                    h: r.h - 18.0,
                    ..r
                },
                t.text,
            );
            continue;
        }
        let ctls = controls(r, kind, m, f);
        let card_hover = ctls.iter().any(|c| matches!(c.ctl, Ctl::Card) && c.hit.is_some() && c.hit == hover);
        p.fill(r, 6.0, if card_hover { t.card_hover } else { t.card });
        let blocks = if kind == Row::Outdated { outdated_blocks(m) } else { Vec::new() };
        // Outdated addons are a warning, like an error in the panel.
        p.stroke(r, 6.0, if blocks.is_empty() { t.stroke } else { t.warn }, 1.0);

        let (icon, title, sub, sub_error) = texts(kind, m);
        p.text(
            icon,
            &f.icon,
            R {
                x: r.x + 14.0,
                w: 28.0,
                h: ROW_H,
                ..r
            },
            if blocks.is_empty() { t.text } else { t.warn },
        );
        // Text runs up to the leftmost control.
        let text_x = r.x + 56.0;
        let ctl_x = ctls
            .iter()
            .filter(|c| !matches!(c.ctl, Ctl::Card | Ctl::Chip(..)))
            .map(|c| c.r.x)
            .fold(r.x + r.w - 16.0, f32::min);
        let text_w = ctl_x - text_x - 12.0;
        p.text(
            title,
            &f.title,
            R {
                x: text_x,
                y: r.y + 11.0,
                w: text_w,
                h: 20.0,
            },
            t.text,
        );
        if !blocks.is_empty() {
            let mut y = r.y + BLOCKS_Y;
            for (s, tone) in &blocks {
                let h = gfx::text_height(&f.wrap, s, BLOCK_W);
                let color = match tone {
                    Tone::Warn => t.warn,
                    Tone::Text => t.text,
                    Tone::Faint => t.text2,
                };
                p.text(s, &f.wrap, R { x: text_x, y, w: BLOCK_W, h }, color);
                y += h + BLOCK_GAP;
            }
            continue;
        }
        p.text(
            &sub,
            if matches!(kind, Row::Wow | Row::Backups | Row::ShotsDir) {
                &f.path
            } else {
                &f.sub
            },
            R {
                x: text_x,
                y: r.y + 32.0,
                w: text_w,
                h: 18.0,
            },
            if sub_error { t.error } else { t.text2 },
        );

        for c in &ctls {
            let hovered = c.hit.is_some() && c.hit == hover;
            match &c.ctl {
                Ctl::Card => {}
                Ctl::Button(label) => {
                    p.fill(c.r, 4.0, if hovered { t.card_hover } else { t.bg });
                    p.stroke(c.r, 4.0, t.stroke, 1.0);
                    p.text(label, &f.button, c.r, t.text);
                }
                Ctl::Chip(label, on) => {
                    let radius = c.r.h / 2.0;
                    if *on {
                        p.fill(c.r, radius, if hovered { lighten(t.accent, t) } else { t.accent });
                        p.text(label, &f.button, c.r, t.on_accent);
                    } else {
                        p.fill(c.r, radius, if hovered { t.card_hover } else { t.card });
                        p.stroke(c.r, radius, t.text2, 1.0);
                        p.text(label, &f.button, c.r, t.text);
                    }
                }
                Ctl::Value(value) => {
                    p.fill(c.r, 4.0, t.bg);
                    p.stroke(c.r, 4.0, t.stroke, 1.0);
                    p.text(value, &f.value, c.r, t.text);
                }
                Ctl::Arrow(icon, enabled) => {
                    if hovered {
                        p.fill(c.r.inset(3.0), 3.0, t.card_hover);
                    }
                    p.text(icon, &f.icon_small, c.r, if *enabled { t.text } else { faint(t) });
                }
                Ctl::Switch(on) => p.switch(c.r, *on, t),
                Ctl::Segment(label, selected) => {
                    let radius = c.r.h / 2.0;
                    if *selected {
                        p.fill(c.r, radius, t.accent);
                        p.text(label, &f.button, c.r, t.on_accent);
                    } else {
                        p.fill(c.r, radius, if hovered { t.card_hover } else { t.card });
                        p.stroke(c.r, radius, t.text2, 1.0);
                        p.text(label, &f.button, c.r, t.text);
                    }
                }
            }
        }
    }

    let (text, error) = m.message.clone().unwrap_or_else(|| (tr!("settings.saved_at_once").into(), false));
    p.text(
        &text,
        &f.sub,
        R {
            x: PAD + 4.0,
            y: bottom(&cards) + 8.0,
            w: W - 2.0 * PAD - 8.0,
            h: FOOTER_H - 16.0,
        },
        if error { t.error } else { t.text2 },
    );
    p.offset(0.0, 0.0);

    // The tabs stay in place and cover the cards scrolled under them.
    p.fill(
        R {
            x: 0.0,
            y: 0.0,
            w: W,
            h: TABS_H,
        },
        0.0,
        t.bg,
    );
    p.fill(
        R {
            x: 0.0,
            y: TABS_H - 1.0,
            w: W,
            h: 1.0,
        },
        0.0,
        t.stroke,
    );
    for (r, item) in tabs(f) {
        let on = item == tab;
        if !on && hover == Some(Hit::Tab(item)) {
            p.fill(r, 4.0, t.card_hover);
        }
        p.text(item.label(), if on { &f.tab_on } else { &f.tab }, r, if on { t.text } else { t.text2 });
        if on {
            let w = 20.0;
            p.fill(
                R {
                    x: r.cx() - w / 2.0,
                    y: r.y + r.h - 3.0,
                    w,
                    h: 3.0,
                },
                1.5,
                t.accent,
            );
        }
    }
}

fn faint(t: &Theme) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a: 0.3, ..t.text }
}

fn lighten(mut c: D2D1_COLOR_F, t: &Theme) -> D2D1_COLOR_F {
    let k = if t.dark { 0.1 } else { -0.08 };
    c.r = (c.r + k).clamp(0.0, 1.0);
    c.g = (c.g + k).clamp(0.0, 1.0);
    c.b = (c.b + k).clamp(0.0, 1.0);
    c
}

// ---------------------------------------------------------------- changes

/// Picks the install folder; a flavor folder (`_retail_`) picked by mistake means its parent.
fn game_root(picked: PathBuf) -> Result<PathBuf, String> {
    if !wow::flavors(&picked).is_empty() {
        return Ok(picked);
    }
    if let Some(parent) = picked.parent().filter(|p| !wow::flavors(p).is_empty()) {
        return Ok(parent.to_path_buf());
    }
    Err(tr!("settings.no_game_in", path = picked.display()))
}

/// Applies a click; returns the action for the tray to run after the borrow ends.
fn change(m: &mut Model, hit: Hit, hwnd: HWND) -> Option<Action> {
    let mut note = None;
    // Another game folder or other versions: what was read from the disk is stale.
    if matches!(hit, Hit::Flavor(_) | Hit::WowChange | Hit::WowAuto) {
        m.forget_reads();
    }
    match hit {
        // Switched by `click`, nothing to save.
        Hit::Tab(_) => return None,
        Hit::WowChange => {
            let start = m.root().map(Path::to_path_buf);
            let picked = gfx::pick_folder(hwnd, tr!("settings.pick_wow"), start.as_deref())?;
            match game_root(picked) {
                Ok(root) => {
                    m.cfg.wow_path = Some(root);
                    m.cfg.flavors.clear();
                    m.find_flavors();
                }
                Err(e) => {
                    m.message = Some((e, true));
                    return None;
                }
            }
        }
        Hit::WowAuto => {
            m.cfg.wow_path = None;
            m.cfg.flavors.clear();
            m.detected = wow::detect_root();
            m.find_flavors();
        }
        Hit::Flavor(i) => {
            let names: Vec<String> = m.flavors.iter().map(|f| f.name.clone()).collect();
            let name = names.get(i)?.clone();
            let mut on: Vec<String> = names.iter().filter(|f| m.flavor_on(f)).cloned().collect();
            if let Some(pos) = on.iter().position(|f| *f == name) {
                on.remove(pos);
            } else {
                on.push(name);
            }
            if on.is_empty() {
                m.message = Some((tr!("settings.need_flavor").into(), true));
                return None;
            }
            // All of them is stored as "all", so versions installed later are picked up too.
            m.cfg.flavors = if on.len() == names.len() {
                Vec::new()
            } else {
                names.into_iter().filter(|f| on.contains(f)).collect()
            };
        }
        Hit::WhenExit => m.cfg.backup_when = When::GameExit,
        Hit::WhenSchedule => {
            m.cfg.backup_when = When::Schedule;
            if m.cfg.backup_days.is_empty() {
                m.cfg.backup_days = Day::ALL.to_vec();
            }
        }
        Hit::TimeDown | Hit::TimeUp => {
            let step = if hit == Hit::TimeUp { 30 } else { -30 };
            m.cfg.backup_time = shift_time(m.cfg.time(), step).format("%H:%M").to_string();
        }
        Hit::Day(i) => {
            if !toggle_day(&mut m.cfg.backup_days, i) {
                m.message = Some((tr!("settings.need_backup_day").into(), true));
                return None;
            }
        }
        Hit::App(app) => {
            m.cfg.update_app = Some(app);
        }
        Hit::UpdateGame => m.cfg.update_skip_in_game = !m.cfg.update_skip_in_game,
        Hit::WagoWeakAuras => m.cfg.wago_weakauras = !m.cfg.wago_weakauras,
        Hit::WagoUiPacks => m.cfg.wago_ui_packs = !m.cfg.wago_ui_packs,
        Hit::Updates => {
            if !m.cfg.update_addons && m.chosen().is_none() {
                m.message = Some((tr!("settings.install_app").into(), true));
                return None;
            }
            m.cfg.update_addons = !m.cfg.update_addons;
            if m.cfg.update_days.is_empty() {
                m.cfg.update_days = Day::ALL.to_vec();
            }
        }
        Hit::UpdateDay(i) => {
            if !toggle_day(&mut m.cfg.update_days, i) {
                m.message = Some((tr!("settings.need_update_day").into(), true));
                return None;
            }
        }
        Hit::UpdateInterval => m.cfg.update_when = UpdateWhen::Interval,
        Hit::UpdateSchedule => {
            m.cfg.update_when = UpdateWhen::Schedule;
            if m.cfg.update_days.is_empty() {
                m.cfg.update_days = Day::ALL.to_vec();
            }
        }
        Hit::EveryDown | Hit::EveryUp => {
            m.cfg.update_every = value_of(config::UPDATE_EVERY, m.cfg.update_every, hit == Hit::EveryUp)?;
        }
        Hit::UpdateTimeDown | Hit::UpdateTimeUp => {
            let step = if hit == Hit::UpdateTimeUp { 30 } else { -30 };
            m.cfg.update_time = shift_time(m.cfg.update_at(), step).format("%H:%M").to_string();
        }
        Hit::KeepDown | Hit::KeepUp => {
            m.cfg.keep = value_of(KEEPS, m.cfg.keep as u64, hit == Hit::KeepUp)? as usize;
            note = Some(tr!("settings.keep_note"));
        }
        Hit::Addons => m.cfg.include_addons = !m.cfg.include_addons,
        Hit::Game => m.cfg.skip_while_running = !m.cfg.skip_while_running,
        Hit::BackupsChange => {
            let picked = gfx::pick_folder(hwnd, tr!("settings.pick_backups"), Some(&m.cfg.backup_dir))?;
            if picked == m.cfg.backup_dir {
                return None;
            }
            let had_any = std::fs::read_dir(&m.cfg.backup_dir).is_ok_and(|mut d| d.next().is_some());
            m.cfg.backup_dir = picked;
            if had_any {
                note = Some(tr!("settings.backups_note"));
            }
        }
        Hit::BackupsOpen => return Some(Action::OpenBackups),
        Hit::Lang(lang) => {
            m.cfg.language = lang;
            crate::i18n::set(lang);
        }
        Hit::Autostart => return Some(Action::Autostart),
        Hit::CheckUpdates => m.cfg.check_updates = !m.cfg.check_updates,
        Hit::CheckNow => return Some(Action::CheckUpdate),
        Hit::InstallUpdate => return Some(Action::InstallUpdate),
        Hit::File => return Some(Action::OpenConfigFile),
        Hit::ShotsChange => {
            let cur = maintenance::screenshots_dir(&m.cfg);
            let picked = gfx::pick_folder(hwnd, tr!("settings.pick_screenshots"), Some(&cur))?;
            if picked == cur {
                return None;
            }
            m.cfg.screenshots_dir = Some(picked);
        }
        Hit::ShotsAuto => m.cfg.screenshots_dir = None,
        Hit::ShotsBy(by) => m.cfg.screenshots_by = by,
        Hit::ShotsFlavor => m.cfg.screenshots_per_flavor = !m.cfg.screenshots_per_flavor,
    }
    let path = tray::config_path()?;
    m.message = match m.cfg.save(&path) {
        Ok(()) => note.map(|n| (n.to_string(), false)),
        Err(e) => Some((tr!("settings.save_failed", error = format!("{e:#}")), true)),
    };
    Some(Action::SettingsChanged)
}

// ---------------------------------------------------------------- window

struct Window {
    surface: Surface,
    fmt: Formats,
    theme: Theme,
    model: Model,
    tab: Tab,
    hover: Option<Hit>,
    tracking: bool,
    scroll: f32,
}

thread_local! {
    static WINDOW: RefCell<Option<Window>> = const { RefCell::new(None) };
}

fn with_window<T>(f: impl FnOnce(&mut Window) -> T) -> Option<T> {
    WINDOW.with(|w| w.borrow_mut().as_mut().map(f))
}

/// Opens the settings window, or brings it to the front.
pub fn show() {
    if let Some(hwnd) = with_window(|w| w.surface.hwnd) {
        unsafe {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
        return;
    }
    if let Err(e) = open() {
        log::error!("{}", tr!("settings.failed", error = e));
    }
}

fn open() -> gfx::Result<()> {
    let Some(path) = tray::config_path() else { return Ok(()) };
    let theme = gfx::theme();
    let title = tr!("settings.title", app = APP_NAME);
    let (model, fmt) = (Model::load(&path), formats()?);
    let height = content_height(&model, &fmt, Tab::default());
    let (hwnd, scale) = gfx::create_window("WowToolsSettings", &title, Some(wndproc), (W, height), &theme);
    let icon = tray::app_icon();
    let window = Window {
        surface: Surface::new(hwnd, scale)?,
        fmt,
        theme,
        model,
        tab: Tab::default(),
        hover: None,
        tracking: false,
        scroll: 0.0,
    };
    WINDOW.with(|w| *w.borrow_mut() = Some(window));
    unsafe {
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon as isize);
        gfx::show(hwnd);
        SetForegroundWindow(hwnd);
    }
    Ok(())
}

/// Re-reads the autostart state (it can be toggled from the panel and the menu too).
pub fn refresh() {
    if let Some(hwnd) = with_window(|w| {
        w.model.autostart = schedule::logon_command().is_some();
        w.model.update = tray::self_update();
        w.surface.hwnd
    }) {
        fit(hwnd);
        unsafe { InvalidateRect(hwnd, null(), 0) };
    }
}

/// Fits the window to the cards of the open tab, so there is no empty space under them; the top
/// stays in place. Taller than the screen's work area, it stops there and the cards scroll.
fn fit(hwnd: HWND) {
    let Some(want) = with_window(|w| (content_height(&w.model, &w.fmt, w.tab) * w.surface.scale).round() as i32) else {
        return;
    };
    unsafe {
        let (mut wr, mut cr): (RECT, RECT) = (zeroed(), zeroed());
        GetWindowRect(hwnd, &mut wr);
        GetClientRect(hwnd, &mut cr);
        let frame = (wr.bottom - wr.top) - (cr.bottom - cr.top);
        let mut mi: MONITORINFO = zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut mi);
        let work = mi.rcWork;
        let h = (want + frame).min(work.bottom - work.top);
        if h == wr.bottom - wr.top {
            return;
        }
        let y = wr.top.min(work.bottom - h).max(work.top);
        SetWindowPos(hwnd, null_mut(), wr.left, y, wr.right - wr.left, h, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    with_window(|w| {
        w.surface.reset();
        w.scroll = w.scroll.min(max_scroll(w));
    });
}

fn client_height(hwnd: HWND, scale: f32) -> f32 {
    let mut rc: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(hwnd, &mut rc) };
    (rc.bottom - rc.top) as f32 / scale
}

fn max_scroll(w: &Window) -> f32 {
    (content_height(&w.model, &w.fmt, w.tab) - client_height(w.surface.hwnd, w.surface.scale)).max(0.0)
}

fn hit_at(w: &Window, lparam: LPARAM) -> Option<Hit> {
    let x = (lparam & 0xFFFF) as i16 as f32 / w.surface.scale;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as f32 / w.surface.scale;
    hit_test(&w.model, &w.fmt, w.tab, x, y, w.scroll)
}

fn click(hwnd: HWND, hit: Hit) {
    if let Hit::Tab(tab) = hit {
        with_window(|w| {
            w.tab = tab;
            w.scroll = 0.0;
            w.hover = None;
            w.model.read_for(tab);
        });
        fit(hwnd);
        unsafe { InvalidateRect(hwnd, null(), 0) };
        return;
    }
    // The folder dialog runs a modal loop that re-enters `wndproc`, so the model is taken out
    // of the window for the duration of the change.
    let Some(mut model) = with_window(|w| std::mem::replace(&mut w.model, Model::empty())) else {
        return;
    };
    let action = change(&mut model, hit, hwnd);
    with_window(|w| {
        w.model = model;
        let tab = w.tab;
        w.model.read_for(tab);
    });
    if let Hit::Lang(_) = hit {
        let title: Vec<u16> = tr!("settings.title", app = APP_NAME).encode_utf16().chain([0]).collect();
        unsafe { SetWindowTextW(hwnd, title.as_ptr()) };
    }
    // A change can add or remove cards (a schedule's days, the Wago switches).
    fit(hwnd);
    unsafe { InvalidateRect(hwnd, null(), 0) };
    if let Some(action) = action {
        tray::act(action);
    }
}

impl Model {
    fn empty() -> Model {
        Model {
            cfg: Config::default(),
            detected: None,
            flavors: Vec::new(),
            autostart: false,
            apps: Vec::new(),
            weakauras: false,
            last_update: None,
            message: None,
            update: tray::SelfUpdate::default(),
            outdated: None,
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = zeroed();
                BeginPaint(hwnd, &mut ps);
                with_window(|w| {
                    let (model, theme, fmt, tab, hover, scroll) = (&w.model, &w.theme, &w.fmt, w.tab, w.hover, w.scroll);
                    w.surface.paint(|p| draw(p, model, theme, fmt, tab, hover, scroll));
                });
                EndPaint(hwnd, &ps);
                0
            }
            WM_ERASEBKGND => 1,
            WM_MOUSEMOVE => {
                let changed = with_window(|w| {
                    if !w.tracking {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        TrackMouseEvent(&mut tme);
                        w.tracking = true;
                    }
                    let h = hit_at(w, lparam);
                    let changed = h != w.hover;
                    w.hover = h;
                    changed
                });
                if changed == Some(true) {
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_MOUSELEAVE => {
                with_window(|w| {
                    w.tracking = false;
                    w.hover = None;
                });
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam >> 16) & 0xFFFF) as i16 as f32 / 120.0;
                with_window(|w| w.scroll = (w.scroll - delta * 60.0).clamp(0.0, max_scroll(w)));
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_LBUTTONUP => {
                if let Some(hit) = with_window(|w| hit_at(w, lparam)).flatten() {
                    click(hwnd, hit);
                }
                0
            }
            WM_SETCURSOR if (lparam & 0xFFFF) as u32 == HTCLIENT => {
                let hand = with_window(|w| w.hover.is_some()).unwrap_or(false);
                SetCursor(LoadCursorW(null_mut(), if hand { IDC_HAND } else { IDC_ARROW }));
                1
            }
            WM_KEYDOWN if wparam == VK_ESCAPE as usize => {
                DestroyWindow(hwnd);
                0
            }
            WM_DPICHANGED => {
                let suggested = &*(lparam as *const RECT);
                with_window(|w| {
                    w.surface.scale = GetDpiForWindow(hwnd) as f32 / 96.0;
                    w.surface.reset();
                });
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                0
            }
            WM_DESTROY => {
                let w = WINDOW.with(|w| w.borrow_mut().take());
                drop(w);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Renders the window's content into a PNG, for checking the look.
pub fn snapshot(path: &str, dark: bool, model: &Model, tab: Tab) -> gfx::Result<()> {
    let theme = gfx::theme_for(dark);
    let fmt = formats()?;
    let hover = match tab {
        Tab::General => Some(Hit::Flavor(0)),
        _ => Some(Hit::Tab(Tab::General)),
    };
    gfx::render_png(path, W, content_height(model, &fmt, tab), 1.5, |p| {
        draw(p, model, &theme, &fmt, tab, hover, 0.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steppers_walk_the_list_from_any_value() {
        assert_eq!(value_of(KEEPS, 30, true), Some(50));
        assert_eq!(value_of(KEEPS, 30, false), Some(20));
        // A hand-edited value in between snaps to its neighbours.
        assert_eq!(value_of(KEEPS, 33, false), Some(30));
        assert_eq!(value_of(KEEPS, 200, true), None);
        assert_eq!(value_of(KEEPS, 3, false), None);
    }

    #[test]
    fn time_stepper_wraps_and_snaps() {
        let t = |h, m| chrono::NaiveTime::from_hms_opt(h, m, 0).unwrap();
        assert_eq!(shift_time(t(10, 0), 30), t(10, 30));
        assert_eq!(shift_time(t(10, 0), -30), t(9, 30));
        assert_eq!(shift_time(t(23, 30), 30), t(0, 0));
        assert_eq!(shift_time(t(0, 0), -30), t(23, 30));
        // A hand-written 10:10 goes to the neighbouring half hours.
        assert_eq!(shift_time(t(10, 10), 30), t(10, 30));
        assert_eq!(shift_time(t(10, 10), -30), t(10, 0));
    }

    #[test]
    fn flavor_titles() {
        assert_eq!(wow::flavor_title("_retail_"), "Retail");
        assert_eq!(wow::flavor_title("_classic_era_"), "Classic Era");
        assert_eq!(wow::flavor_title("_xptr_"), "XPTR");
    }
}
