//! The flyout opened by a left click on the tray icon, with three tabs: "Бэкап" (status, "backup now",
//! restore points with an inline confirmation), "Аддоны" (last update, "update now", the last runs)
//! and "Уборка" (what can be tidied up, a button each), and the autostart switch under all of them.
//! The window exists only while open.

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::{self, Align, Painter, R, Surface, Theme};
use crate::maintenance::{self, Item, List};
use crate::profiles::{CharEntry, Profile};
use crate::snapshot;
use crate::tray::{self, Action, Phase, RestorePoint, Status};
use crate::tray::{ProfileJob, Tone};
use crate::updater::Entry;
use crate::watch::Work;

const W: f32 = 360.0;
const PAD: f32 = 16.0;
const TABS_Y: f32 = 54.0;
const TABS_H: f32 = 34.0;
const STATUS_Y: f32 = TABS_Y + TABS_H + 14.0;
const STATUS_H: f32 = 66.0;
const BUTTON_Y: f32 = STATUS_Y + STATUS_H + 12.0;
const BUTTON_H: f32 = 36.0;
const LIST_LABEL_Y: f32 = BUTTON_Y + BUTTON_H + 18.0;
const LIST_Y: f32 = LIST_LABEL_Y + 28.0;
const ROW_H: f32 = 52.0;
const ROW_STEP: f32 = 56.0;
const FOOTER_H: f32 = 48.0;
const WM_MOUSELEAVE: u32 = 0x02A3;
/// Redraws the panel while a cleanup runs, for its `spinner`.
const SPIN_TIMER: usize = 1;
const SPIN_MS: u32 = 100;

/// Everything the panel shows; the snapshot command fills it with demo data.
pub struct View {
    pub status: Status,
    pub points: Vec<RestorePoint>,
    pub autostart: bool,
    /// "После каждого выхода из игры", "По расписанию: ежедневно", ...
    pub schedule: String,
    /// The app addon updates go through ("CurseForge"), if any is installed.
    pub update_app: Option<&'static str>,
    /// "Каждый час", "По расписанию: ежедневно в 09:00", "Автообновление выключено".
    pub update_text: String,
    pub last_update: Option<chrono::NaiveDateTime>,
    /// The last update runs, newest first.
    pub updates: Vec<Entry>,
    /// The game runs and updates wait for it to close (`update_skip_in_game`).
    pub update_waits_for_game: bool,
    /// A newer WoW Tools, shown as a button in the header.
    pub self_update: crate::tray::SelfUpdate,
    /// What the cleanup tab shows (read in the background); `None` until the first read is done.
    pub scan: Option<maintenance::Scan>,
    /// The cleanup running now.
    pub cleaning: Option<Item>,
    /// How the last cleanup went: (item, summary, ok); shown in its row.
    pub cleaned: Option<(Item, String, bool)>,
    /// Where the cleanup moves screenshots (`maintenance::screenshots_dir`).
    pub shots_dir: std::path::PathBuf,
    /// Saved UI profiles, newest first.
    pub profiles: Vec<Profile>,
    /// Characters to pick for profiles, last played first; `None` until read.
    pub chars: Option<Vec<CharEntry>>,
    /// The profile job running now, and how the last one went.
    pub profile_job: Option<ProfileJob>,
    pub profile_done: Option<(ProfileJob, String, Tone)>,
    /// The game versions running now (`_retail_`, ...): their profiles cannot be applied.
    pub running: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tab {
    Backup,
    Addons,
    Cleanup,
    Profiles,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Backup => tr!("panel.tab_backup"),
            Tab::Addons => tr!("panel.tab_addons"),
            Tab::Cleanup => tr!("cleanup.tab"),
            Tab::Profiles => tr!("profiles.tab"),
        }
    }
}

/// How many update runs the "Аддоны" tab lists.
pub const HISTORY_ROWS: usize = 5;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Tab(Tab),
    Update,
    /// Hovering a history row shows its full text.
    History(usize),
    Folder,
    Settings,
    Backup,
    Restore(usize),
    Confirm(usize),
    Cancel,
    Autostart,
    /// The "new version" button in the header.
    SelfUpdate,
    /// A cleanup button: the first click asks "sure?".
    Clean(Item),
    CleanConfirm(Item),
    /// Hovering cleanup line `i` (`cleanup_lines`) shows what it is about.
    CleanInfo(usize),
    OpenScreenshots,
    /// The full list of a cleanup line, opened over the tab (`Sheet`).
    List(List),
    SheetClose,
    /// Inside the sheet: takes clicks and hover from the rows under it.
    Sheet,
    /// Visible entry `k` of a character pick list.
    SheetRow(usize),
    /// "Save a profile": opens the character pick list.
    SaveProfile,
    /// Profile `i`: pick the character to put it on.
    ApplyProfile(usize),
    DeleteProfile(usize),
    /// "Sure?" of a profile row (apply or delete).
    ProfileConfirm,
    /// Hovering profile row `i` shows what it holds.
    ProfileInfo(usize),
    /// The apply button of profile `i` while its game version runs: says to close it.
    ApplyBlocked(usize),
    /// Hovering the save card shows how the last save went, in full.
    SaveInfo,
}

/// List rows: the longer of the two lists, so switching tabs does not resize the panel.
fn rows(v: &View) -> usize {
    v.points.len().max(v.updates.len()).max(1)
}

/// The lowest of the tabs (the cleanup tab is fixed), so switching tabs does not resize the panel.
fn divider_y(v: &View) -> f32 {
    let lists = LIST_Y + rows(v) as f32 * ROW_STEP;
    let cleanup = cleanup_lines().last().map_or(0.0, |(r, _)| r.y + r.h);
    lists.max(cleanup) + 6.0
}

/// A line of the cleanup tab.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Line {
    Label(&'static str),
    Clean(Item),
    /// The biggest addon settings: only shown.
    Big,
}

const CLEAN_Y: f32 = TABS_Y + TABS_H + 12.0;
const LABEL_H: f32 = 28.0;

fn cleanup_lines() -> Vec<(R, Line)> {
    let lines = [
        Line::Label(tr!("cleanup.section_wtf")),
        Line::Clean(Item::Orphans),
        Line::Clean(Item::Backups),
        Line::Big,
        Line::Label(tr!("cleanup.section_game")),
        Line::Clean(Item::Logs),
        Line::Clean(Item::Screenshots),
        Line::Clean(Item::Cache),
    ];
    let mut y = CLEAN_Y;
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let h = if let Line::Label(_) = line {
                // A gap over the second heading.
                if i > 0 {
                    y += 8.0;
                }
                22.0
            } else {
                ROW_H
            };
            let r = R {
                x: PAD,
                y,
                w: W - 2.0 * PAD,
                h,
            };
            y += if h == ROW_H { ROW_STEP } else { LABEL_H };
            (r, line)
        })
        .collect()
}

/// The button of a cleanup row ("Удалить", "Перенести", ...), at its right end.
fn clean_label(item: Item) -> &'static str {
    match item {
        Item::Screenshots => tr!("cleanup.move"),
        Item::Cache => tr!("cleanup.clear"),
        _ => tr!("cleanup.delete"),
    }
}

fn clean_rect(row: R, f: &Formats, item: Item) -> R {
    let w = (gfx::text_width(&f.button, clean_label(item)) + 28.0).max(84.0);
    R {
        x: row.x + row.w - 12.0 - w,
        y: row.cy() - 15.0,
        w,
        h: 30.0,
    }
}

/// The icon button of a cleanup line: the folder of the moved screenshots, the full list of the
/// orphans and of the biggest settings (only when there is one). `None` for the others.
fn side_icon(v: &View, line: Line) -> Option<(&'static str, Hit)> {
    let scan = v.scan.as_ref();
    match line {
        Line::Clean(Item::Screenshots) => Some(("\u{E838}", Hit::OpenScreenshots)),
        Line::Clean(Item::Orphans) if scan.is_some_and(|s| !s.orphans.is_empty()) => Some(("\u{E8FD}", Hit::List(List::Orphans))),
        Line::Big if scan.is_some_and(|s| !s.big.is_empty()) => Some(("\u{E8FD}", Hit::List(List::Big))),
        _ => None,
    }
}

/// Where `side_icon` goes: left of the line's button, or at the right end without one.
fn side_rect(v: &View, row: R, f: &Formats, line: Line) -> R {
    let right = match line {
        Line::Clean(item) if has_button(v, item) => clean_rect(row, f, item).x - 6.0,
        _ => row.x + row.w - 12.0,
    };
    R {
        x: right - 30.0,
        y: row.cy() - 15.0,
        w: 30.0,
        h: 30.0,
    }
}

/// A list shown over a tab, scrolled to entry `first`: a long list of the cleanup tab (opened by
/// its list button) or the characters to pick for a profile.
#[derive(Clone, PartialEq, Debug)]
pub struct Sheet {
    pub list: SheetList,
    pub first: usize,
    /// Typed into the search field of a pick list (the panel takes the keys while it is open).
    pub search: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SheetList {
    Clean(List),
    /// Characters to save a profile from.
    SaveFrom,
    /// Characters to put profile `i` on.
    ApplyTo(usize),
}

const SHEET_HEAD: f32 = 52.0;
const SHEET_ROW: f32 = 28.0;

/// The sheet covers the rows of the tab.
fn sheet_rect(v: &View) -> R {
    R {
        x: PAD,
        y: CLEAN_Y,
        w: W - 2.0 * PAD,
        h: divider_y(v) - 10.0 - CLEAN_Y,
    }
}

fn sheet_close_rect(v: &View) -> R {
    let r = sheet_rect(v);
    R {
        x: r.x + r.w - 42.0,
        y: r.y + 10.0,
        w: 30.0,
        h: 30.0,
    }
}

/// The search field of a pick list.
const SEARCH_H: f32 = 40.0;

/// The heading: title and line under it, and the search field of a pick list.
fn sheet_head(list: SheetList) -> f32 {
    if picks(list) { SHEET_HEAD + SEARCH_H } else { SHEET_HEAD }
}

/// A list of characters to pick from (it has a search field).
fn picks(list: SheetList) -> bool {
    !matches!(list, SheetList::Clean(_))
}

/// Entries that fit under the heading.
fn sheet_rows(v: &View, list: SheetList) -> usize {
    ((sheet_rect(v).h - sheet_head(list) - 8.0) / SHEET_ROW).floor().max(1.0) as usize
}

/// Visible entry `k` of the sheet.
fn sheet_row_rect(v: &View, list: SheetList, k: usize) -> R {
    let r = sheet_rect(v);
    R {
        x: r.x + 4.0,
        y: r.y + sheet_head(list) + 4.0 + k as f32 * SHEET_ROW,
        w: r.w - 18.0,
        h: SHEET_ROW,
    }
}

/// The characters a pick list offers (indexes into `View::chars`): any for saving, those of the
/// profile's game version but its own character for applying; with `search`, those whose name or
/// realm has it.
fn sheet_chars(v: &View, list: SheetList, search: &str) -> Vec<usize> {
    let search = search.to_lowercase();
    let Some(chars) = v.chars.as_ref() else { return Vec::new() };
    let profile = match list {
        SheetList::Clean(_) => return Vec::new(),
        SheetList::SaveFrom => None,
        SheetList::ApplyTo(i) => match v.profiles.get(i) {
            Some(p) => Some(&p.meta),
            None => return Vec::new(),
        },
    };
    chars
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            profile.is_none_or(|m| {
                let c = &e.character;
                e.flavor == m.flavor && !(c.account == m.account && c.realm == m.realm && c.name == m.name)
            })
        })
        .filter(|(_, e)| {
            let c = &e.character;
            search.is_empty() || c.name.to_lowercase().contains(&search) || c.realm.to_lowercase().contains(&search)
        })
        .map(|(i, _)| i)
        .collect()
}

/// The rows of a sheet: (left, right) texts.
/// A row of a sheet: the name, a grey note after it, a value at the right end.
struct SheetEntry {
    name: String,
    note: String,
    right: String,
}

fn sheet_entries(v: &View, s: &Sheet) -> Vec<SheetEntry> {
    match s.list {
        SheetList::Clean(list) => v
            .scan
            .as_ref()
            .map_or(&[][..], |s| s.list(list))
            .iter()
            .map(|(name, size)| SheetEntry {
                name: name.clone(),
                note: String::new(),
                right: snapshot::human_size(*size),
            })
            .collect(),
        _ => {
            let chars = v.chars.as_deref().unwrap_or_default();
            // The game version only matters when there is more than one.
            let many = chars.iter().any(|e| e.flavor != chars[0].flavor);
            sheet_chars(v, s.list, &s.search)
                .into_iter()
                .map(|i| {
                    let e = &chars[i];
                    let note = if many {
                        format!("{} · {}", e.character.realm, crate::wow::flavor_title(&e.flavor))
                    } else {
                        e.character.realm.clone()
                    };
                    SheetEntry {
                        name: e.character.name.clone(),
                        note,
                        right: e.played.map(when).unwrap_or_default(),
                    }
                })
                .collect()
        }
    }
}

/// `first` kept in range, so the last page is full.
fn sheet_first(v: &View, s: &Sheet) -> usize {
    s.first.min(sheet_entries(v, s).len().saturating_sub(sheet_rows(v, s.list)))
}

fn draw_sheet(p: &Painter, v: &View, t: &Theme, f: &Formats, hover: Option<Hit>, s: &Sheet) {
    let r = sheet_rect(v);
    // The card is see-through in places: hide the rows under it first.
    p.fill(r, 8.0, t.bg);
    p.fill(r, 8.0, t.card);
    p.stroke(r, 8.0, t.stroke, 1.0);
    let entries = sheet_entries(v, s);
    let (title, sub) = match s.list {
        SheetList::Clean(list) => {
            let total: u64 = v.scan.as_ref().map_or(&[][..], |s| s.list(list)).iter().map(|(_, size)| size).sum();
            let title = match list {
                List::Orphans => tr!("cleanup.orphans_short").to_string(),
                List::Big => tr!("cleanup.big_short").to_string(),
            };
            (title, tr!("cleanup.count_addons", n = entries.len(), size = snapshot::human_size(total)))
        }
        SheetList::SaveFrom => (tr!("profiles.pick_save").to_string(), tr!("profiles.pick_sub").to_string()),
        SheetList::ApplyTo(i) => {
            let name = v.profiles.get(i).map(|p| p.title()).unwrap_or_default();
            (tr!("profiles.pick_apply", name = name), tr!("profiles.pick_sub").to_string())
        }
    };
    let picking = picks(s.list);
    let text_w = r.w - 16.0 - 50.0;
    p.text(
        &title,
        &f.bold,
        R {
            x: r.x + 16.0,
            y: r.y + 8.0,
            w: text_w,
            h: 20.0,
        },
        t.text,
    );
    p.text(
        &sub,
        &f.sub,
        R {
            x: r.x + 16.0,
            y: r.y + 28.0,
            w: text_w,
            h: 16.0,
        },
        t.text2,
    );
    let close = sheet_close_rect(v);
    if hover == Some(Hit::SheetClose) {
        p.fill(close, 4.0, t.card_hover);
    }
    p.text("\u{E711}", &f.icon, close, t.text);
    if picking {
        draw_search(p, t, f, r, &s.search);
    }
    let head = sheet_head(s.list);
    p.fill(
        R {
            x: r.x + 1.0,
            y: r.y + head,
            w: r.w - 2.0,
            h: 1.0,
        },
        0.0,
        t.stroke,
    );
    let rows = sheet_rows(v, s.list);
    let first = sheet_first(v, s);
    if entries.is_empty() {
        let none = match picking {
            true if !s.search.is_empty() => tr!("profiles.no_match"),
            true => tr!("profiles.no_chars"),
            false => tr!("cleanup.none"),
        };
        let row = sheet_row_rect(v, s.list, 0);
        p.text(
            none,
            &f.sub,
            R {
                x: row.x + 12.0,
                w: row.w - 24.0,
                ..row
            },
            t.text2,
        );
    }
    for (k, e) in entries.iter().skip(first).take(rows).enumerate() {
        let row = sheet_row_rect(v, s.list, k);
        if picking && hover == Some(Hit::SheetRow(k)) {
            p.fill(row, 4.0, t.card_hover);
        }
        let right_w = if picking { 104.0 } else { 80.0 };
        let right_r = R {
            x: row.x + row.w - 4.0 - right_w,
            w: right_w,
            ..row
        };
        p.text(&e.right, &f.sub_right, right_r, t.text2);
        let name_r = R {
            x: row.x + 12.0,
            w: right_r.x - row.x - 20.0,
            ..row
        };
        p.text(&e.name, &f.text, name_r, t.text);
        if !e.note.is_empty() {
            let x = name_r.x + gfx::text_width(&f.text, &e.name) + 8.0;
            let note_r = R {
                x,
                y: row.y + 1.0,
                w: (name_r.x + name_r.w - x).max(0.0),
                ..row
            };
            p.text(&e.note, &f.sub, note_r, t.text2);
        }
    }
    // Scroll bar: where the visible part is in the whole list.
    if entries.len() > rows {
        let top = r.y + head + 4.0;
        let track_h = rows as f32 * SHEET_ROW;
        let thumb_h = (track_h * rows as f32 / entries.len() as f32).max(24.0);
        let max_first = (entries.len() - rows) as f32;
        let thumb = R {
            x: r.x + r.w - 9.0,
            y: top + (track_h - thumb_h) * first as f32 / max_first,
            w: 4.0,
            h: thumb_h,
        };
        p.fill(thumb, 2.0, t.text2);
    }
}

/// The search field under the heading of a pick list: what is typed with a caret, or a hint.
fn draw_search(p: &Painter, t: &Theme, f: &Formats, sheet: R, search: &str) {
    let r = R {
        x: sheet.x + 12.0,
        y: sheet.y + SHEET_HEAD - 2.0,
        w: sheet.w - 24.0,
        h: 32.0,
    };
    p.fill(r, 4.0, t.bg);
    p.stroke(r, 4.0, t.accent, 1.0);
    p.text("\u{E721}", &f.icon, R { w: 32.0, ..r }, t.text2);
    let text = R {
        x: r.x + 34.0,
        w: r.w - 42.0,
        ..r
    };
    if search.is_empty() {
        p.text(tr!("profiles.search"), &f.text, text, t.text2);
        p.fill(
            R {
                x: text.x,
                y: r.y + 8.0,
                w: 1.0,
                h: 16.0,
            },
            0.0,
            t.text,
        );
    } else {
        p.text(search, &f.text, text, t.text);
        let caret = (text.x + gfx::text_width(&f.text, search) + 1.0).min(text.x + text.w);
        p.fill(
            R {
                x: caret,
                y: r.y + 8.0,
                w: 1.0,
                h: 16.0,
            },
            0.0,
            t.text,
        );
    }
}

/// Something was found for `item`: its row has a button (off while something runs).
fn has_button(v: &View, item: Item) -> bool {
    v.scan.as_ref().is_some_and(|s| s.get(item).files > 0)
}

/// What asks "sure?" now: a restore point or a cleanup.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Confirm {
    Restore(usize),
    Clean(Item),
    DeleteProfile(usize),
    /// Put profile `.0` on character `.1` (`View::chars`).
    ApplyProfile(usize, usize),
}

/// Something to clean was found for `item` and nothing else runs.
fn clean_allowed(v: &View, item: Item) -> bool {
    v.status.phase != Phase::Busy && v.scan.as_ref().is_some_and(|s| s.get(item).files > 0)
}

fn height(v: &View) -> f32 {
    divider_y(v) + 1.0 + FOOTER_H + 8.0
}

fn folder_rect() -> R {
    R {
        x: W - PAD - 72.0,
        y: 14.0,
        w: 32.0,
        h: 32.0,
    }
}

fn settings_rect() -> R {
    R {
        x: W - PAD - 34.0,
        y: 14.0,
        w: 32.0,
        h: 32.0,
    }
}

/// The "new version" button left of the header icons, while there is one.
fn update_rect(v: &View, f: &Formats) -> Option<R> {
    let release = v.self_update.available.as_ref()?;
    let w = gfx::text_width(&f.button, &release.version) + 44.0;
    Some(R {
        x: folder_rect().x - 8.0 - w,
        y: 16.0,
        w,
        h: 28.0,
    })
}

fn button_rect() -> R {
    R {
        x: PAD,
        y: BUTTON_Y,
        w: W - 2.0 * PAD,
        h: BUTTON_H,
    }
}

fn row_rect(i: usize) -> R {
    R {
        x: PAD,
        y: LIST_Y + i as f32 * ROW_STEP,
        w: W - 2.0 * PAD,
        h: ROW_H,
    }
}

fn restore_rect(row: R) -> R {
    R {
        x: row.x + row.w - 112.0,
        y: row.cy() - 15.0,
        w: 100.0,
        h: 30.0,
    }
}

fn cancel_rect(row: R) -> R {
    R {
        x: row.x + row.w - 42.0,
        y: row.cy() - 15.0,
        w: 30.0,
        h: 30.0,
    }
}

fn confirm_rect(row: R) -> R {
    R {
        x: row.x + row.w - 154.0,
        y: row.cy() - 15.0,
        w: 106.0,
        h: 30.0,
    }
}

fn footer_rect(v: &View) -> R {
    R {
        x: 4.0,
        y: divider_y(v) + 5.0,
        w: W - 8.0,
        h: FOOTER_H,
    }
}

/// Point `i` can be restored: nothing else is running and its flavor's client is closed.
fn restore_allowed(v: &View, i: usize) -> bool {
    v.status.phase != Phase::Busy && v.points.get(i).is_some_and(|p| !p.running)
}

/// The tab strip under the title.
fn tabs(f: &Formats) -> Vec<(R, Tab)> {
    let mut x = PAD - 4.0;
    [Tab::Backup, Tab::Addons, Tab::Cleanup, Tab::Profiles]
        .into_iter()
        .map(|tab| {
            let w = gfx::text_width(&f.tab_on, tab.label()) + 28.0;
            let r = R { x, y: TABS_Y, w, h: TABS_H };
            x += w + 2.0;
            (r, tab)
        })
        .collect()
}

fn hits(v: &View, f: &Formats, tab: Tab, confirm: Option<Confirm>, sheet: Option<&Sheet>, prof_first: usize) -> Vec<(R, Hit)> {
    let mut out = vec![(folder_rect(), Hit::Folder), (settings_rect(), Hit::Settings), (footer_rect(v), Hit::Autostart)];
    if let Some(r) = update_rect(v, f).filter(|_| !v.self_update.installing) {
        out.push((r, Hit::SelfUpdate));
    }
    out.extend(tabs(f).into_iter().filter(|(_, t)| *t != tab).map(|(r, t)| (r, Hit::Tab(t))));
    if tab == Tab::Addons {
        if v.status.phase != Phase::Busy && v.update_app.is_some() && !v.update_waits_for_game {
            out.push((button_rect(), Hit::Update));
        }
        out.extend((0..v.updates.len()).map(|i| (row_rect(i), Hit::History(i))));
        return out;
    }
    if let (Tab::Cleanup | Tab::Profiles, Some(s)) = (tab, sheet) {
        out.push((sheet_close_rect(v), Hit::SheetClose));
        if picks(s.list) {
            let n = sheet_entries(v, s).len().saturating_sub(sheet_first(v, s)).min(sheet_rows(v, s.list));
            out.extend((0..n).map(|k| (sheet_row_rect(v, s.list, k), Hit::SheetRow(k))));
        }
        out.push((sheet_rect(v), Hit::Sheet));
        return out;
    }
    if tab == Tab::Profiles {
        profile_hits(v, f, confirm, prof_first, &mut out);
        return out;
    }
    if tab == Tab::Cleanup {
        let lines = cleanup_lines();
        // Buttons first: they win over the row under them.
        for &(row, line) in &lines {
            let mut asking = false;
            if let Line::Clean(item) = line {
                let allowed = clean_allowed(v, item);
                asking = allowed && confirm == Some(Confirm::Clean(item));
                if asking {
                    out.push((confirm_rect(row), Hit::CleanConfirm(item)));
                    out.push((cancel_rect(row), Hit::Cancel));
                } else if allowed {
                    out.push((clean_rect(row, f, item), Hit::Clean(item)));
                }
            }
            if let Some((_, hit)) = side_icon(v, line).filter(|_| !asking) {
                out.push((side_rect(v, row, f, line), hit));
            }
        }
        out.extend(
            lines
                .iter()
                .enumerate()
                .filter(|(_, (_, l))| !matches!(l, Line::Label(_)))
                .map(|(i, (r, _))| (*r, Hit::CleanInfo(i))),
        );
        return out;
    }
    if v.status.phase != Phase::Busy {
        out.push((button_rect(), Hit::Backup));
    }
    {
        for i in (0..v.points.len()).filter(|&i| restore_allowed(v, i)) {
            let row = row_rect(i);
            if confirm == Some(Confirm::Restore(i)) {
                out.push((confirm_rect(row), Hit::Confirm(i)));
                out.push((cancel_rect(row), Hit::Cancel));
            } else {
                out.push((restore_rect(row), Hit::Restore(i)));
            }
        }
    }
    out
}

struct Formats {
    wrap: IDWriteTextFormat,
    tab: IDWriteTextFormat,
    tab_on: IDWriteTextFormat,
    title: IDWriteTextFormat,
    text: IDWriteTextFormat,
    bold: IDWriteTextFormat,
    sub: IDWriteTextFormat,
    sub_right: IDWriteTextFormat,
    button: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    icon_big: IDWriteTextFormat,
}

fn formats() -> gfx::Result<Formats> {
    let dw = gfx::dwrite()?;
    Ok(Formats {
        wrap: gfx::wrap_format(&dw, 12.0)?,
        tab: gfx::text_format(&dw, gfx::UI_FONT, 14.0, false, Align::Center)?,
        tab_on: gfx::text_format(&dw, gfx::UI_FONT, 14.0, true, Align::Center)?,
        title: gfx::text_format(&dw, gfx::TITLE_FONT, 20.0, true, Align::Left)?,
        text: gfx::text_format(&dw, gfx::UI_FONT, 14.0, false, Align::Left)?,
        bold: gfx::text_format(&dw, gfx::UI_FONT, 14.0, true, Align::Left)?,
        sub: gfx::text_format(&dw, gfx::UI_FONT, 12.0, false, Align::Left)?,
        sub_right: gfx::text_format(&dw, gfx::UI_FONT, 12.0, false, Align::Right)?,
        button: gfx::text_format(&dw, gfx::UI_FONT, 13.0, false, Align::Center)?,
        icon: gfx::text_format(&dw, gfx::ICON_FONT, 16.0, false, Align::Center)?,
        icon_big: gfx::text_format(&dw, gfx::ICON_FONT, 20.0, false, Align::Center)?,
    })
}

#[derive(Clone, Copy)]
enum Style {
    Accent { hover: bool },
    Plain { hover: bool },
    Disabled,
}

/// A button with an optional icon left of the label, centered together.
fn button(p: &Painter, f: &Formats, r: R, icon: &str, label: &str, style: Style, t: &Theme) {
    let fg = match style {
        Style::Accent { hover } => {
            p.fill(r, 4.0, if hover { lighten(t.accent, t) } else { t.accent });
            t.on_accent
        }
        Style::Plain { hover } => {
            p.fill(r, 4.0, if hover { t.card_hover } else { t.card });
            p.stroke(r, 4.0, t.stroke, 1.0);
            t.text
        }
        Style::Disabled => {
            p.fill(r, 4.0, t.card);
            p.stroke(r, 4.0, t.stroke, 1.0);
            t.text2
        }
    };
    if icon.is_empty() {
        p.text(label, &f.button, r, fg);
    } else {
        // Icon and label are centered together.
        let text_w = gfx::text_width(&f.button, label);
        let x = r.x + (r.w - text_w - 24.0) / 2.0;
        p.text(icon, &f.icon, R { x, y: r.y, w: 18.0, h: r.h }, fg);
        p.text(
            label,
            &f.button,
            R {
                x: x + 24.0,
                y: r.y,
                w: text_w + 2.0,
                h: r.h,
            },
            fg,
        );
    }
}

fn lighten(mut c: D2D1_COLOR_F, t: &Theme) -> D2D1_COLOR_F {
    let k = if t.dark { 0.1 } else { -0.08 };
    c.r = (c.r + k).clamp(0.0, 1.0);
    c.g = (c.g + k).clamp(0.0, 1.0);
    c.b = (c.b + k).clamp(0.0, 1.0);
    c
}

#[allow(clippy::too_many_arguments)]
fn draw(p: &Painter, v: &View, t: &Theme, f: &Formats, tab: Tab, hover: Option<Hit>, confirm: Option<Confirm>, sheet: Option<&Sheet>, prof_first: usize) {
    p.clear(t.bg);
    p.text(
        crate::config::APP_NAME,
        &f.title,
        R {
            x: PAD,
            y: 16.0,
            w: 200.0,
            h: 28.0,
        },
        t.text,
    );
    for (r, hit, icon) in [(folder_rect(), Hit::Folder, "\u{E838}"), (settings_rect(), Hit::Settings, "\u{E713}")] {
        if hover == Some(hit) {
            p.fill(r, 4.0, t.card_hover);
        }
        p.text(icon, &f.icon, r, t.text);
    }
    if let (Some(r), Some(release)) = (update_rect(v, f), &v.self_update.available) {
        let style = if v.self_update.installing {
            Style::Disabled
        } else {
            Style::Accent {
                hover: hover == Some(Hit::SelfUpdate),
            }
        };
        button(p, f, r, "\u{E896}", &release.version, style, t);
    }
    draw_tabs(p, t, f, tab, hover);
    match tab {
        Tab::Backup => {
            let confirm = match confirm {
                Some(Confirm::Restore(i)) => Some(i),
                _ => None,
            };
            draw_backup(p, v, t, f, hover, confirm)
        }
        Tab::Addons => draw_addons(p, v, t, f, hover),
        Tab::Cleanup => draw_cleanup(p, v, t, f, hover, confirm),
        Tab::Profiles => draw_profiles(p, v, t, f, hover, confirm, prof_first),
    }
    if let (Tab::Cleanup | Tab::Profiles, Some(s)) = (tab, sheet) {
        draw_sheet(p, v, t, f, hover, s);
    }
    draw_footer(p, v, t, f, hover);
    if let (Tab::Addons, Some(Hit::History(i))) = (tab, hover) {
        draw_history_tip(p, v, t, f, i);
    }
    if let (Tab::Cleanup, Some(Hit::CleanInfo(i))) = (tab, hover) {
        draw_clean_tip(p, v, t, f, i);
    }
    if let (Tab::Profiles, Some(Hit::SaveInfo), Some((_, text, tone))) = (tab, hover, v.profile_done.as_ref().filter(|d| d.0 == ProfileJob::Save)) {
        draw_row_tip(p, v, t, f, save_card(), text, *tone == Tone::Error);
    }
    if let (Tab::Profiles, Some(Hit::ApplyBlocked(i))) = (tab, hover)
        && let Some(profile) = v.profiles.get(i)
    {
        let k = i.saturating_sub(profile_first(v, prof_first));
        let text = tr!("profiles.close_game", game = crate::wow::flavor_title(&profile.meta.flavor));
        draw_row_tip(p, v, t, f, profile_rect(k), &text, false);
    }
    if let (Tab::Profiles, Some(Hit::ProfileInfo(i))) = (tab, hover) {
        let k = i.saturating_sub(profile_first(v, prof_first));
        draw_row_tip(p, v, t, f, profile_rect(k), &profile_tip(v, i), false);
    }
    if let (Some(Hit::SelfUpdate), Some(release)) = (hover, &v.self_update.available) {
        let text = tr!("self_update.pill_tip", version = &release.version);
        let r = R {
            x: PAD,
            y: TABS_Y + 4.0,
            w: W - 2.0 * PAD,
            h: 0.0,
        };
        draw_tip(p, t, f, r, &text, false);
    }
}

/// A card with wrapped text at the top of `at` (its height is the text's).
fn draw_tip(p: &Painter, t: &Theme, f: &Formats, at: R, text: &str, error: bool) {
    let inner = at.w - 28.0;
    let card = R {
        h: gfx::text_height(&f.wrap, text, inner) + 24.0,
        ..at
    };
    p.fill(card, 6.0, t.card_hover);
    p.stroke(card, 6.0, if error { t.error } else { t.stroke }, 1.0);
    p.text(
        text,
        &f.wrap,
        R {
            x: card.x + 14.0,
            y: card.y + 12.0,
            w: inner,
            h: card.h - 24.0,
        },
        if error { t.error } else { t.text },
    );
}

/// The full text of history row `i` (what was updated, what was not and why), in a card over
/// the rows next to it.
fn draw_history_tip(p: &Painter, v: &View, t: &Theme, f: &Formats, i: usize) {
    let Some(e) = v.updates.get(i) else { return };
    draw_row_tip(p, v, t, f, row_rect(i), &e.text, !e.ok);
}

/// A tip card for `row`: under it when it fits above the footer, else over it.
fn draw_row_tip(p: &Painter, v: &View, t: &Theme, f: &Formats, row: R, text: &str, error: bool) {
    let h = gfx::text_height(&f.wrap, text, row.w - 28.0) + 24.0;
    let below = row.y + row.h + 4.0;
    let y = if below + h <= divider_y(v) {
        below
    } else {
        (row.y - 4.0 - h).max(TABS_Y + TABS_H + 4.0)
    };
    draw_tip(p, t, f, R { y, ..row }, text, error);
}

fn draw_tabs(p: &Painter, t: &Theme, f: &Formats, tab: Tab, hover: Option<Hit>) {
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
    p.fill(
        R {
            x: 0.0,
            y: TABS_Y + TABS_H + 2.0,
            w: W,
            h: 1.0,
        },
        0.0,
        t.stroke,
    );
}

fn status_card() -> R {
    R {
        x: PAD,
        y: STATUS_Y,
        w: W - 2.0 * PAD,
        h: STATUS_H,
    }
}

pub fn busy_text(work: Work) -> &'static str {
    match work {
        Work::Backup => tr!("panel.busy_backup"),
        Work::Addons => tr!("panel.busy_addons"),
        Work::Restore => tr!("panel.busy_restore"),
        Work::Cleanup => tr!("panel.busy_cleanup"),
        Work::Profile => tr!("panel.busy_profile"),
    }
}

/// The card at the top of a tab: a big icon, a bold line and a smaller one under it.
fn draw_status(p: &Painter, t: &Theme, f: &Formats, icon: (&str, D2D1_COLOR_F), line1: &str, line2: (&str, D2D1_COLOR_F)) {
    let card = status_card();
    p.fill(card, 6.0, t.card);
    p.stroke(card, 6.0, t.stroke, 1.0);
    p.text(
        icon.0,
        &f.icon_big,
        R {
            x: card.x + 10.0,
            y: card.y,
            w: 40.0,
            h: card.h,
        },
        icon.1,
    );
    let tx = card.x + 56.0;
    p.text(
        line1,
        &f.bold,
        R {
            x: tx,
            y: card.y + 12.0,
            w: card.w - 68.0,
            h: 22.0,
        },
        t.text,
    );
    p.text(
        line2.0,
        &f.sub,
        R {
            x: tx,
            y: card.y + 34.0,
            w: card.w - 68.0,
            h: 20.0,
        },
        line2.1,
    );
}

/// The heading over a list, with a hint at the right.
fn list_label(p: &Painter, t: &Theme, f: &Formats, label: &str, hint: &str) {
    p.text(
        label,
        &f.bold,
        R {
            x: PAD,
            y: LIST_LABEL_Y,
            w: 220.0,
            h: 22.0,
        },
        t.text,
    );
    p.text(
        hint,
        &f.sub_right,
        R {
            x: W - PAD - 170.0,
            y: LIST_LABEL_Y,
            w: 170.0,
            h: 22.0,
        },
        t.text2,
    );
}

fn draw_addons(p: &Painter, v: &View, t: &Theme, f: &Formats, hover: Option<Hit>) {
    // A backup or a restore keeps the worker busy too, but it is shown on its own tab; here only
    // the button is off.
    let s = &v.status;
    let busy = s.phase == Phase::Busy;
    let updating = busy && s.work == Work::Addons;
    let failed = v.updates.first().is_some_and(|e| !e.ok);
    let (icon, icon_color) = match () {
        _ if updating => ("\u{E895}", t.accent),
        _ if v.update_app.is_none() || failed => ("\u{E783}", t.error),
        _ if v.last_update.is_some() => ("\u{E930}", t.ok),
        _ => ("\u{E895}", t.text2),
    };
    let line1 = match v.last_update {
        Some(when) => tr!("panel.last_update", time = tray::format_time(when)),
        None => tr!("panel.never_updated").to_string(),
    };
    let (line2, line2_color) = match v.update_app {
        _ if updating => (busy_text(s.work).to_string(), t.text2),
        None => (tr!("panel.no_addon_app").to_string(), t.error),
        Some(_) if failed => (tr!("panel.last_failed").to_string(), t.error),
        Some(app) if v.update_waits_for_game => (tr!("panel.update_after_game", app = app), t.text2),
        Some(app) => (format!("{} · {app}", v.update_text), t.text2),
    };
    draw_status(p, t, f, (icon, icon_color), &line1, (&line2, line2_color));

    if updating {
        button(p, f, button_rect(), "\u{E895}", busy_text(s.work), Style::Disabled, t);
    } else if busy || v.update_app.is_none() || v.update_waits_for_game {
        button(p, f, button_rect(), "\u{E896}", tr!("menu.update"), Style::Disabled, t);
    } else {
        let style = Style::Accent {
            hover: hover == Some(Hit::Update),
        };
        button(p, f, button_rect(), "\u{E896}", tr!("menu.update"), style, t);
    }

    list_label(p, t, f, tr!("panel.history"), &tr!("panel.history_hint", n = HISTORY_ROWS));
    if v.updates.is_empty() {
        let row = row_rect(0);
        p.fill(row, 6.0, t.card);
        p.stroke(row, 6.0, t.stroke, 1.0);
        p.text(tr!("panel.no_history"), &f.text, R { x: row.x + 16.0, ..row }, t.text2);
    }
    for (i, e) in v.updates.iter().enumerate() {
        let row = row_rect(i);
        p.fill(row, 6.0, t.card);
        p.stroke(row, 6.0, t.stroke, 1.0);
        let tx = row.x + 16.0;
        let text_w = row.w - 32.0;
        let when = tray::format_time(e.time);
        p.text(
            &when,
            &f.text,
            R {
                x: tx,
                y: row.y + 7.0,
                w: text_w,
                h: 20.0,
            },
            t.text,
        );
        let mut nx = tx + gfx::text_width(&f.text, &when) + 8.0;
        let tags = [
            Some((e.app.as_str(), t.text2)),
            (!e.manual).then_some((tr!("panel.tag_auto"), t.text2)),
            (!e.ok).then_some((tr!("panel.tag_error"), t.error)),
        ];
        for (tag, color) in tags.into_iter().flatten() {
            let r = R {
                x: nx,
                y: row.y + 8.0,
                w: (tx + text_w - nx).max(0.0),
                h: 20.0,
            };
            p.text(tag, &f.sub, r, color);
            nx += gfx::text_width(&f.sub, tag) + 8.0;
        }
        p.text(
            e.text.lines().next().unwrap_or_default(),
            &f.sub,
            R {
                x: tx,
                y: row.y + 27.0,
                w: text_w,
                h: 18.0,
            },
            if e.ok { t.text2 } else { t.error },
        );
    }
}

/// Title and the short line under it of a cleanup row; the line may be a busy note or how the
/// last cleanup of it went (`Some(ok)`, colored).
fn clean_texts(v: &View, line: Line) -> (&'static str, String, Option<bool>) {
    let Some(scan) = v.scan.as_ref() else {
        let title = match line {
            Line::Clean(item) => short_title(item),
            _ => tr!("cleanup.big_short"),
        };
        return (title, tr!("cleanup.scanning").to_string(), None);
    };
    let Line::Clean(item) = line else {
        let list: Vec<String> = scan
            .big
            .iter()
            .take(3)
            .map(|(name, size)| format!("{name} {}", snapshot::human_size(*size)))
            .collect();
        let sub = if list.is_empty() {
            tr!("cleanup.none").to_string()
        } else {
            list.join(" · ")
        };
        return (tr!("cleanup.big_short"), sub, None);
    };
    if v.cleaning == Some(item) {
        return (short_title(item), busy_text(Work::Cleanup).to_string(), None);
    }
    if let Some((_, summary, ok)) = v.cleaned.as_ref().filter(|c| c.0 == item) {
        let lines: Vec<&str> = summary.lines().collect();
        return (short_title(item), lines.join(" · "), Some(*ok));
    }
    let found = scan.get(item);
    let size = snapshot::human_size(found.bytes);
    let sub = match item {
        _ if found.files == 0 => tr!("cleanup.none").to_string(),
        Item::Orphans => tr!("cleanup.count_addons", n = found.names.len(), size = size),
        _ => tr!("cleanup.count_files", n = found.files, size = size),
    };
    (short_title(item), sub, None)
}

fn short_title(item: Item) -> &'static str {
    match item {
        Item::Orphans => tr!("cleanup.orphans_short"),
        Item::Backups => tr!("cleanup.backups_short"),
        Item::Logs => tr!("cleanup.logs_short"),
        Item::Screenshots => tr!("cleanup.screenshots_short"),
        Item::Cache => tr!("cleanup.cache_short"),
    }
}

/// What a cleanup row is about: shown while hovering it. Long lists are not in it, its list button
/// opens them (`side_icon`).
fn clean_tip(v: &View, line: Line) -> String {
    let text = match line {
        Line::Label(_) => return String::new(),
        Line::Big => tr!("cleanup.big_tip").to_string(),
        Line::Clean(item) => match item {
            Item::Orphans => tr!("cleanup.orphans_tip").to_string(),
            Item::Backups => tr!("cleanup.backups_tip").to_string(),
            Item::Logs => tr!("cleanup.logs_tip").to_string(),
            Item::Screenshots => tr!("cleanup.screenshots_tip", dir = v.shots_dir.display()),
            Item::Cache => tr!("cleanup.cache_tip").to_string(),
        },
    };
    match side_icon(v, line) {
        Some((_, Hit::List(_))) => format!(
            "{text}

{}",
            tr!("cleanup.list_hint")
        ),
        _ => text,
    }
}

fn draw_clean_tip(p: &Painter, v: &View, t: &Theme, f: &Formats, i: usize) {
    let Some(&(row, line)) = cleanup_lines().get(i) else { return };
    let text = clean_tip(v, line);
    if !text.is_empty() {
        draw_row_tip(p, v, t, f, row, &text, false);
    }
}

fn draw_cleanup(p: &Painter, v: &View, t: &Theme, f: &Formats, hover: Option<Hit>, confirm: Option<Confirm>) {
    for (row, line) in cleanup_lines() {
        if let Line::Label(label) = line {
            p.text(label, &f.bold, row, t.text);
            continue;
        }
        p.fill(row, 6.0, t.card);
        p.stroke(row, 6.0, t.stroke, 1.0);
        let item = match line {
            Line::Clean(item) => Some(item),
            _ => None,
        };
        let asking = item.is_some_and(|i| confirm == Some(Confirm::Clean(i)) && clean_allowed(v, i));
        // Text stops short of whatever buttons the row shows.
        let mut buttons_x = row.x + row.w - 12.0;
        if let Some(item) = item {
            if asking {
                buttons_x = confirm_rect(row).x;
            } else if has_button(v, item) {
                buttons_x = clean_rect(row, f, item).x;
            }
        }
        let icon = side_icon(v, line).filter(|_| !asking);
        if icon.is_some() {
            buttons_x = side_rect(v, row, f, line).x;
        }
        let tx = row.x + 16.0;
        let text_w = buttons_x - tx - 8.0;
        let (title, sub, result) = clean_texts(v, line);
        p.text(
            title,
            &f.text,
            R {
                x: tx,
                y: row.y + 7.0,
                w: text_w,
                h: 20.0,
            },
            t.text,
        );
        p.text(
            &sub,
            &f.sub,
            R {
                x: tx,
                y: row.y + 27.0,
                w: text_w,
                h: 18.0,
            },
            match result {
                Some(true) => t.ok,
                Some(false) => t.error,
                None => t.text2,
            },
        );
        // While asking "sure?" the confirm buttons take the icon's place.
        if let Some((glyph, hit)) = icon {
            let r = side_rect(v, row, f, line);
            if hover == Some(hit) {
                p.fill(r, 4.0, t.card_hover);
            }
            p.text(glyph, &f.icon, r, t.text);
        }
        let Some(item) = item else { continue };
        if asking {
            let style = Style::Accent {
                hover: hover == Some(Hit::CleanConfirm(item)),
            };
            button(p, f, confirm_rect(row), "", tr!("cleanup.confirm"), style, t);
            let cancel = cancel_rect(row);
            if hover == Some(Hit::Cancel) {
                p.fill(cancel, 4.0, t.card_hover);
            }
            p.text("\u{E711}", &f.icon, cancel, t.text);
        } else if v.cleaning == Some(item) {
            spinner(p, clean_rect(row, f, item), t);
        } else if has_button(v, item) {
            let style = if clean_allowed(v, item) {
                Style::Plain {
                    hover: hover == Some(Hit::Clean(item)),
                }
            } else {
                Style::Disabled
            };
            button(p, f, clean_rect(row, f, item), "", clean_label(item), style, t);
        }
    }
}

const PROF_LABEL_Y: f32 = CLEAN_Y + ROW_STEP + 8.0;
const PROF_LIST_Y: f32 = PROF_LABEL_Y + LABEL_H;

/// The "save a profile" card at the top of the profiles tab.
fn save_card() -> R {
    R {
        x: PAD,
        y: CLEAN_Y,
        w: W - 2.0 * PAD,
        h: ROW_H,
    }
}

/// How many profile rows fit above the footer.
fn profile_rows(v: &View) -> usize {
    ((divider_y(v) - 6.0 - PROF_LIST_Y + (ROW_STEP - ROW_H)) / ROW_STEP).floor().max(1.0) as usize
}

/// Visible profile row `k`.
fn profile_rect(k: usize) -> R {
    R {
        x: PAD,
        y: PROF_LIST_Y + k as f32 * ROW_STEP,
        w: W - 2.0 * PAD,
        h: ROW_H,
    }
}

/// `first` kept in range, so the last page is full.
fn profile_first(v: &View, first: usize) -> usize {
    first.min(v.profiles.len().saturating_sub(profile_rows(v)))
}

/// A button with `label` at the right end of `row`.
fn end_button_rect(row: R, f: &Formats, label: &str) -> R {
    let w = (gfx::text_width(&f.button, label) + 28.0).max(84.0);
    R {
        x: row.x + row.w - 12.0 - w,
        y: row.cy() - 15.0,
        w,
        h: 30.0,
    }
}

fn save_button_rect(f: &Formats) -> R {
    end_button_rect(save_card(), f, tr!("profiles.save_button"))
}

/// "Применить…", left of the delete icon.
fn apply_rect(row: R, f: &Formats) -> R {
    let mut r = end_button_rect(row, f, tr!("profiles.apply_button"));
    r.x -= 36.0;
    r
}

fn delete_rect(row: R) -> R {
    R {
        x: row.x + row.w - 42.0,
        y: row.cy() - 15.0,
        w: 30.0,
        h: 30.0,
    }
}

/// Profile jobs and deletes wait while anything else runs.
fn profiles_allowed(v: &View) -> bool {
    v.status.phase != Phase::Busy
}

fn can_save(v: &View) -> bool {
    profiles_allowed(v) && v.chars.as_ref().is_some_and(|c| !c.is_empty())
}

/// Profile `i` asks "sure?" (to delete it, or to put it on a character).
fn profile_asking(confirm: Option<Confirm>, i: usize) -> bool {
    matches!(confirm, Some(Confirm::DeleteProfile(j) | Confirm::ApplyProfile(j, _)) if j == i)
}

fn profile_hits(v: &View, f: &Formats, confirm: Option<Confirm>, first: usize, out: &mut Vec<(R, Hit)>) {
    if can_save(v) {
        out.push((save_button_rect(f), Hit::SaveProfile));
    }
    out.push((save_card(), Hit::SaveInfo));
    let first = profile_first(v, first);
    for k in 0..profile_rows(v) {
        let i = first + k;
        if i >= v.profiles.len() {
            break;
        }
        let row = profile_rect(k);
        if profiles_allowed(v) {
            if profile_asking(confirm, i) {
                out.push((confirm_rect(row), Hit::ProfileConfirm));
                out.push((cancel_rect(row), Hit::Cancel));
            } else {
                let blocked = v.running.contains(&v.profiles[i].meta.flavor);
                out.push((apply_rect(row, f), if blocked { Hit::ApplyBlocked(i) } else { Hit::ApplyProfile(i) }));
                out.push((delete_rect(row), Hit::DeleteProfile(i)));
            }
        }
        out.push((row, Hit::ProfileInfo(i)));
    }
}

/// "сегодня 15:08", "вчера 21:40", older: the day alone.
fn when(t: chrono::NaiveDateTime) -> String {
    if t.date() < chrono::Local::now().date_naive() - chrono::Duration::days(1) {
        t.format(&tr!("time.date").replace(" %H:%M", "")).to_string()
    } else {
        tray::format_time(t)
    }
}

/// A job's result in one line, in its tone's color.
fn result_line(t: &Theme, text: &str, tone: Tone) -> (String, D2D1_COLOR_F) {
    let lines: Vec<&str> = text.lines().collect();
    let color = match tone {
        Tone::Ok => t.ok,
        Tone::Notice => t.warn,
        Tone::Error => t.error,
    };
    (lines.join(" · "), color)
}

/// The line under a profile row's title, and its color: what it asks, its job, how that went,
/// or when it was saved and what it holds.
fn profile_sub(v: &View, t: &Theme, i: usize, confirm: Option<Confirm>) -> (String, D2D1_COLOR_F) {
    let p = &v.profiles[i];
    if let Some(Confirm::ApplyProfile(j, c)) = confirm
        && j == i
        && let Some(e) = v.chars.as_ref().and_then(|cs| cs.get(c))
    {
        let to = format!("{} ({})", e.character.name, e.character.realm);
        return (tr!("profiles.apply_to", name = to), t.text);
    }
    if confirm == Some(Confirm::DeleteProfile(i)) {
        return (tr!("profiles.delete_ask").to_string(), t.text);
    }
    let job = ProfileJob::Apply(p.path.clone());
    if v.profile_job.as_ref() == Some(&job) {
        return (busy_text(Work::Profile).to_string(), t.text2);
    }
    if let Some((_, text, tone)) = v.profile_done.as_ref().filter(|d| d.0 == job) {
        return result_line(t, text, *tone);
    }
    let mut sub = tr!("profiles.row_sub", time = when(p.meta.created), addons = p.meta.addons.len());
    // The game version only matters when profiles of more than one are saved.
    if v.profiles.iter().any(|o| o.meta.flavor != p.meta.flavor) {
        sub = format!("{} · {sub}", crate::wow::flavor_title(&p.meta.flavor));
    }
    (sub, t.text2)
}

/// What a profile holds, after how it was last applied: shown while hovering its row.
fn profile_tip(v: &View, i: usize) -> String {
    let Some(p) = v.profiles.get(i) else { return String::new() };
    let addons = if p.meta.addons.is_empty() {
        tr!("profiles.tip_no_addons").to_string()
    } else {
        p.meta.addons.join(", ")
    };
    let tip = tr!("profiles.tip", files = p.meta.files, addons = addons);
    match v.profile_done.as_ref().filter(|d| d.0 == ProfileJob::Apply(p.path.clone())) {
        Some((_, result, _)) => format!(
            "{result}

{tip}"
        ),
        None => tip,
    }
}

/// Title and sub line of a row card, left of `buttons_x`.
fn row_texts(p: &Painter, f: &Formats, t: &Theme, row: R, buttons_x: f32, title: &str, sub: (&str, D2D1_COLOR_F)) {
    let tx = row.x + 16.0;
    let w = buttons_x - tx - 8.0;
    p.text(
        title,
        &f.text,
        R {
            x: tx,
            y: row.y + 7.0,
            w,
            h: 20.0,
        },
        t.text,
    );
    p.text(
        sub.0,
        &f.sub,
        R {
            x: tx,
            y: row.y + 27.0,
            w,
            h: 18.0,
        },
        sub.1,
    );
}

fn draw_profiles(p: &Painter, v: &View, t: &Theme, f: &Formats, hover: Option<Hit>, confirm: Option<Confirm>, first: usize) {
    let card = save_card();
    p.fill(card, 6.0, t.card);
    p.stroke(card, 6.0, t.stroke, 1.0);
    let save = save_button_rect(f);
    let saving = v.profile_job == Some(ProfileJob::Save);
    let (sub, color) = if saving {
        (busy_text(Work::Profile).to_string(), t.text2)
    } else if let Some((_, text, tone)) = v.profile_done.as_ref().filter(|d| d.0 == ProfileJob::Save) {
        result_line(t, text, *tone)
    } else {
        (tr!("profiles.save_sub").to_string(), t.text2)
    };
    row_texts(p, f, t, card, save.x, tr!("profiles.save_title"), (&sub, color));
    if saving {
        spinner(p, save, t);
    } else {
        let style = if can_save(v) {
            Style::Plain {
                hover: hover == Some(Hit::SaveProfile),
            }
        } else {
            Style::Disabled
        };
        button(p, f, save, "", tr!("profiles.save_button"), style, t);
    }

    let label = R {
        x: PAD,
        y: PROF_LABEL_Y,
        w: W - 2.0 * PAD,
        h: 22.0,
    };
    p.text(tr!("profiles.saved_label"), &f.bold, label, t.text);
    if v.profiles.is_empty() {
        let r = R {
            y: PROF_LIST_Y,
            h: 60.0,
            ..label
        };
        p.text(tr!("profiles.empty"), &f.wrap, r, t.text2);
        return;
    }
    let first = profile_first(v, first);
    let rows = profile_rows(v);
    for (k, profile) in v.profiles.iter().enumerate().skip(first).take(rows).map(|(i, pr)| (i - first, pr)) {
        let i = first + k;
        let row = profile_rect(k);
        p.fill(row, 6.0, t.card);
        p.stroke(row, 6.0, t.stroke, 1.0);
        let asking = profile_asking(confirm, i);
        let busy = v.profile_job.as_ref() == Some(&ProfileJob::Apply(profile.path.clone()));
        let buttons_x = if asking { confirm_rect(row).x } else { apply_rect(row, f).x };
        let (sub, color) = profile_sub(v, t, i, confirm);
        row_texts(p, f, t, row, buttons_x, &profile.title(), (&sub, color));
        if asking {
            let label = match confirm {
                Some(Confirm::DeleteProfile(_)) => tr!("profiles.delete"),
                _ => tr!("profiles.apply"),
            };
            let style = Style::Accent {
                hover: hover == Some(Hit::ProfileConfirm),
            };
            button(p, f, confirm_rect(row), "", label, style, t);
            let cancel = cancel_rect(row);
            if hover == Some(Hit::Cancel) {
                p.fill(cancel, 4.0, t.card_hover);
            }
            p.text("\u{E711}", &f.icon, cancel, t.text);
        } else if busy {
            spinner(p, apply_rect(row, f), t);
        } else {
            let allowed = profiles_allowed(v);
            let style = if allowed && !v.running.contains(&profile.meta.flavor) {
                Style::Plain {
                    hover: hover == Some(Hit::ApplyProfile(i)),
                }
            } else {
                Style::Disabled
            };
            button(p, f, apply_rect(row, f), "", tr!("profiles.apply_button"), style, t);
            let del = delete_rect(row);
            if allowed && hover == Some(Hit::DeleteProfile(i)) {
                p.fill(del, 4.0, t.card_hover);
            }
            p.text("\u{E74D}", &f.icon, del, if allowed { t.text } else { t.text2 });
        }
    }
    // A scroll bar in the right margin when there are more profiles than rows.
    if v.profiles.len() > rows {
        let track_h = rows as f32 * ROW_STEP - (ROW_STEP - ROW_H);
        let thumb_h = (track_h * rows as f32 / v.profiles.len() as f32).max(24.0);
        let max_first = (v.profiles.len() - rows) as f32;
        let thumb = R {
            x: W - PAD + 5.0,
            y: PROF_LIST_Y + (track_h - thumb_h) * first as f32 / max_first,
            w: 4.0,
            h: thumb_h,
        };
        p.fill(thumb, 2.0, t.text2);
    }
}

/// Dots going round, for a job that gives no progress; the panel redraws while it runs (`SPIN_TIMER`).
fn spinner(p: &Painter, r: R, t: &Theme) {
    const DOTS: u64 = 8;
    let head = unsafe { GetTickCount64() } / SPIN_MS as u64 % DOTS;
    for i in 0..DOTS {
        let angle = i as f32 / DOTS as f32 * std::f32::consts::TAU;
        // The head is solid, the dots behind it fade.
        let behind = (head + DOTS - i) % DOTS;
        let mut c = t.accent;
        c.a = 1.0 - behind as f32 / DOTS as f32 * 0.85;
        p.circle(r.cx() + 9.0 * angle.sin(), r.cy() - 9.0 * angle.cos(), 2.2, c);
    }
}

fn draw_backup(p: &Painter, v: &View, t: &Theme, f: &Formats, hover: Option<Hit>, confirm: Option<usize>) {
    // Status card. An addon update keeps the worker busy too, but it is shown on its own tab;
    // here only the button is off.
    let s = &v.status;
    let busy = s.phase == Phase::Busy;
    let phase = if busy && s.work == Work::Addons { &Phase::Idle } else { &s.phase };
    let (icon, icon_color) = match (phase, &s.game, s.last) {
        (Phase::Busy, _, _) => ("\u{E895}", t.accent),
        (Phase::Error(_), _, _) => ("\u{E783}", t.error),
        (Phase::Idle, Some(_), _) => ("\u{E7FC}", t.warn),
        (Phase::Idle, None, Some(_)) => ("\u{E930}", t.ok),
        (Phase::Idle, None, None) => ("\u{E81C}", t.text2),
    };
    let line1 = match s.last {
        Some(when) => tr!("panel.last_backup", time = tray::format_time(when)),
        None => tr!("panel.no_backups").to_string(),
    };
    let (line2, line2_color) = match (phase, &s.game) {
        (Phase::Busy, _) => (busy_text(s.work).to_string(), t.text2),
        (Phase::Error(e), _) => (tr!("panel.error", error = e.lines().next().unwrap_or_default()), t.error),
        (Phase::Idle, Some(_)) => (tr!("panel.backup_after_game").to_string(), t.text2),
        (Phase::Idle, None) => (v.schedule.clone(), t.text2),
    };
    draw_status(p, t, f, (icon, icon_color), &line1, (&line2, line2_color));

    // Backup button.
    if *phase == Phase::Busy {
        button(p, f, button_rect(), "\u{E895}", busy_text(s.work), Style::Disabled, t);
    } else if busy {
        button(p, f, button_rect(), "\u{E74E}", tr!("panel.backup_now"), Style::Disabled, t);
    } else {
        let style = Style::Accent {
            hover: hover == Some(Hit::Backup),
        };
        button(p, f, button_rect(), "\u{E74E}", tr!("panel.backup_now"), style, t);
    }

    // Restore points.
    list_label(p, t, f, tr!("panel.points"), tr!("panel.points_hint"));
    if v.points.is_empty() {
        let row = row_rect(0);
        p.fill(row, 6.0, t.card);
        p.stroke(row, 6.0, t.stroke, 1.0);
        p.text(tr!("panel.no_points"), &f.text, R { x: row.x + 16.0, ..row }, t.text2);
    }
    for (i, point) in v.points.iter().enumerate() {
        let row = row_rect(i);
        p.fill(row, 6.0, t.card);
        p.stroke(row, 6.0, t.stroke, 1.0);
        // Text stops short of whatever buttons the row shows.
        let buttons_x = if confirm == Some(i) { confirm_rect(row).x } else { restore_rect(row).x };
        let tx = row.x + 16.0;
        let text_w = buttons_x - tx - 8.0;
        let when = tray::format_time(point.created);
        p.text(
            &when,
            &f.text,
            R {
                x: tx,
                y: row.y + 7.0,
                w: text_w,
                h: 20.0,
            },
            t.text,
        );
        // After the time: the flavor (when several are backed up) and why the snapshot was taken.
        let mut nx = tx + gfx::text_width(&f.text, &when) + 8.0;
        let tags = [
            point.flavor_title.as_deref().map(|s| (s, t.text2)),
            point.note().filter(|_| confirm != Some(i)).map(|s| (s, t.accent)),
        ];
        for (tag, color) in tags.into_iter().flatten() {
            let r = R {
                x: nx,
                y: row.y + 8.0,
                w: (tx + text_w - nx).max(0.0),
                h: 20.0,
            };
            p.text(tag, &f.sub, r, color);
            nx += gfx::text_width(&f.sub, tag) + 8.0;
        }
        let detail = if confirm == Some(i) {
            tr!("panel.current_kept").to_string()
        } else {
            point.detail()
        };
        p.text(
            &detail,
            &f.sub,
            R {
                x: tx,
                y: row.y + 27.0,
                w: text_w,
                h: 18.0,
            },
            t.text2,
        );
        if confirm == Some(i) {
            let ok = confirm_rect(row);
            button(
                p,
                f,
                ok,
                "",
                tr!("panel.restore"),
                Style::Accent {
                    hover: hover == Some(Hit::Confirm(i)),
                },
                t,
            );
            let cancel = cancel_rect(row);
            if hover == Some(Hit::Cancel) {
                p.fill(cancel, 4.0, t.card_hover);
            }
            p.text("\u{E711}", &f.icon, cancel, t.text);
        } else if restore_allowed(v, i) {
            let style = Style::Plain {
                hover: hover == Some(Hit::Restore(i)),
            };
            button(p, f, restore_rect(row), "\u{E777}", tr!("panel.restore_short"), style, t);
        } else if point.running {
            p.text(tr!("panel.game_running"), &f.sub_right, restore_rect(row), t.text2);
        }
    }
}

fn draw_footer(p: &Painter, v: &View, t: &Theme, f: &Formats, hover: Option<Hit>) {
    // Footer: autostart switch.
    let dy = divider_y(v);
    p.fill(R { x: 0.0, y: dy, w: W, h: 1.0 }, 0.0, t.stroke);
    let foot = footer_rect(v);
    if hover == Some(Hit::Autostart) {
        p.fill(foot.inset(2.0), 4.0, t.card_hover);
    }
    p.text(
        "\u{E7E8}",
        &f.icon,
        R {
            x: foot.x + 8.0,
            y: foot.y,
            w: 28.0,
            h: foot.h,
        },
        t.text,
    );
    p.text(
        tr!("panel.autostart"),
        &f.text,
        R {
            x: foot.x + 44.0,
            y: foot.y,
            w: 220.0,
            h: foot.h,
        },
        t.text,
    );
    let sw = R {
        x: foot.x + foot.w - 56.0,
        y: foot.cy() - 10.0,
        w: 40.0,
        h: 20.0,
    };
    p.switch(sw, v.autostart, t);
}

// ---------------------------------------------------------------- window

struct Panel {
    surface: Surface,
    fmt: Formats,
    theme: Theme,
    view: View,
    tab: Tab,
    hover: Option<Hit>,
    confirm: Option<Confirm>,
    sheet: Option<Sheet>,
    /// The first profile row shown (the list scrolls with the wheel).
    prof_first: usize,
    tracking: bool,
    anchor_bottom: bool,
}

thread_local! {
    static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
}

/// When the panel last closed: a click on the tray icon first deactivates (closes) it,
/// and must not reopen it right away.
static LAST_CLOSE: AtomicU64 = AtomicU64::new(0);
/// The tab open last (`Tab as u8`); the panel reopens on it.
static LAST_TAB: AtomicU8 = AtomicU8::new(0);

fn last_tab() -> Tab {
    match LAST_TAB.load(Ordering::Relaxed) {
        1 => Tab::Addons,
        2 => Tab::Cleanup,
        3 => Tab::Profiles,
        _ => Tab::Backup,
    }
}

fn with_panel<T>(f: impl FnOnce(&mut Panel) -> T) -> Option<T> {
    PANEL.with(|p| p.borrow_mut().as_mut().map(f))
}

pub fn toggle() {
    if let Some(hwnd) = with_panel(|p| p.surface.hwnd) {
        unsafe { DestroyWindow(hwnd) };
        return;
    }
    if unsafe { GetTickCount64() }.saturating_sub(LAST_CLOSE.load(Ordering::Relaxed)) < 300 {
        return;
    }
    if let Err(e) = open() {
        log::error!("{}", tr!("panel.failed", error = e));
    }
}

fn open() -> gfx::Result<()> {
    // How the last cleanup went is news only until the panel closes.
    tray::forget_results();
    let view = tray::view();
    let theme = gfx::theme();
    let (pt, work, scale) = gfx::cursor_monitor();
    let (w, h) = ((W * scale).round() as i32, (height(&view) * scale).round() as i32);
    let margin = (12.0 * scale) as i32;
    let edge = gfx::taskbar_edge();
    let near_x = (pt.x - w / 2).clamp(work.left + margin, work.right - w - margin);
    let near_y = (pt.y - h / 2).clamp(work.top + margin, work.bottom - h - margin);
    let (x, y) = match edge {
        0 => (work.left + margin, near_y),
        1 => (near_x, work.top + margin),
        2 => (work.right - w - margin, near_y),
        _ => (near_x, work.bottom - h - margin),
    };
    let hwnd = gfx::create_popup("WowToolsPanel", Some(wndproc), (x, y, w, h), theme.dark, false);
    let panel = Panel {
        surface: Surface::new(hwnd, scale)?,
        fmt: formats()?,
        theme,
        view,
        tab: last_tab(),
        hover: None,
        confirm: None,
        sheet: None,
        prof_first: 0,
        tracking: false,
        anchor_bottom: edge == 3,
    };
    let cleaning = spinning(&panel.view);
    PANEL.with(|p| *p.borrow_mut() = Some(panel));
    spin(hwnd, cleaning);
    if matches!(last_tab(), Tab::Cleanup | Tab::Profiles) {
        tray::rescan();
    }
    unsafe {
        gfx::show(hwnd);
        SetForegroundWindow(hwnd);
    }
    Ok(())
}

/// Re-reads the state (after a backup, a restore, a settings change) and redraws.
pub fn refresh() {
    let Some((hwnd, cleaning)) = with_panel(|p| {
        p.view = tray::view();
        let still = match p.confirm {
            Some(Confirm::Restore(i)) => restore_allowed(&p.view, i),
            Some(Confirm::Clean(item)) => clean_allowed(&p.view, item),
            Some(Confirm::DeleteProfile(i)) => profiles_allowed(&p.view) && i < p.view.profiles.len(),
            Some(Confirm::ApplyProfile(i, c)) => profiles_allowed(&p.view) && i < p.view.profiles.len() && p.view.chars.as_ref().is_some_and(|cs| c < cs.len()),
            None => true,
        };
        if !still {
            p.confirm = None;
        }
        // A list that emptied (after a cleanup) closes.
        if p.sheet.as_ref().is_some_and(|s| !picks(s.list) && sheet_entries(&p.view, s).is_empty()) {
            p.sheet = None;
        }
        (p.surface.hwnd, spinning(&p.view))
    }) else {
        return;
    };
    spin(hwnd, cleaning);
    unsafe {
        fit_height(hwnd);
        InvalidateRect(hwnd, null(), 0);
    }
}

/// A job with a `spinner` runs.
fn spinning(v: &View) -> bool {
    v.cleaning.is_some() || v.profile_job.is_some()
}

/// Starts or stops the redraws of the `spinner`.
fn spin(hwnd: HWND, on: bool) {
    unsafe {
        if on {
            SetTimer(hwnd, SPIN_TIMER, SPIN_MS, None);
        } else {
            KillTimer(hwnd, SPIN_TIMER);
        }
    }
}

/// Resizes the panel when the list changed length, keeping the taskbar side in place.
unsafe fn fit_height(hwnd: HWND) {
    let Some((want, anchor_bottom)) = with_panel(|p| ((height(&p.view) * p.surface.scale).round() as i32, p.anchor_bottom)) else {
        return;
    };
    unsafe {
        let mut rc: RECT = zeroed();
        GetWindowRect(hwnd, &mut rc);
        if rc.bottom - rc.top == want {
            return;
        }
        let y = if anchor_bottom { rc.bottom - want } else { rc.top };
        SetWindowPos(hwnd, null_mut(), rc.left, y, rc.right - rc.left, want, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    with_panel(|p| p.surface.reset());
}

fn hit_at(p: &Panel, lparam: LPARAM) -> Option<Hit> {
    let x = (lparam & 0xFFFF) as i16 as f32 / p.surface.scale;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as f32 / p.surface.scale;
    hits(&p.view, &p.fmt, p.tab, p.confirm, p.sheet.as_ref(), p.prof_first)
        .into_iter()
        .find(|(r, _)| r.contains(x, y))
        .map(|(_, h)| h)
}

fn click(hwnd: HWND, hit: Hit) {
    let mut close = false;
    let action = with_panel(|p| match hit {
        Hit::Folder => {
            close = true;
            Some(Action::OpenBackups)
        }
        Hit::Settings => {
            close = true;
            Some(Action::OpenSettings)
        }
        Hit::Backup => Some(Action::BackupNow),
        Hit::Update => Some(Action::UpdateAddons),
        Hit::History(_) => None,
        Hit::Tab(tab) => {
            p.tab = tab;
            p.confirm = None;
            p.sheet = None;
            LAST_TAB.store(tab as u8, Ordering::Relaxed);
            if matches!(tab, Tab::Cleanup | Tab::Profiles) {
                tray::rescan();
            }
            None
        }
        Hit::Autostart => Some(Action::Autostart),
        Hit::SelfUpdate => {
            close = true;
            Some(Action::InstallUpdate)
        }
        Hit::Restore(i) => {
            p.confirm = Some(Confirm::Restore(i));
            None
        }
        Hit::CleanInfo(_) => None,
        Hit::Clean(item) => {
            p.confirm = Some(Confirm::Clean(item));
            None
        }
        Hit::CleanConfirm(item) => {
            p.confirm = None;
            Some(Action::Clean(item))
        }
        Hit::OpenScreenshots => {
            close = true;
            Some(Action::OpenScreenshots)
        }
        Hit::List(list) => {
            p.sheet = Some(Sheet {
                list: SheetList::Clean(list),
                first: 0,
                search: String::new(),
            });
            p.confirm = None;
            None
        }
        Hit::SheetClose => {
            p.sheet = None;
            None
        }
        Hit::Sheet | Hit::ProfileInfo(_) | Hit::SaveInfo | Hit::ApplyBlocked(_) => None,
        Hit::SaveProfile => {
            p.sheet = Some(Sheet {
                list: SheetList::SaveFrom,
                first: 0,
                search: String::new(),
            });
            p.confirm = None;
            None
        }
        Hit::ApplyProfile(i) => {
            p.sheet = Some(Sheet {
                list: SheetList::ApplyTo(i),
                first: 0,
                search: String::new(),
            });
            p.confirm = None;
            None
        }
        Hit::DeleteProfile(i) => {
            p.confirm = Some(Confirm::DeleteProfile(i));
            None
        }
        Hit::SheetRow(k) => {
            let s = p.sheet.take()?;
            let c = *sheet_chars(&p.view, s.list, &s.search).get(sheet_first(&p.view, &s) + k)?;
            match s.list {
                SheetList::SaveFrom => p.view.chars.as_ref()?.get(c).cloned().map(Action::SaveProfile),
                SheetList::ApplyTo(i) => {
                    p.confirm = Some(Confirm::ApplyProfile(i, c));
                    None
                }
                SheetList::Clean(_) => None,
            }
        }
        Hit::ProfileConfirm => match p.confirm.take()? {
            Confirm::DeleteProfile(i) => p.view.profiles.get(i).cloned().map(Action::DeleteProfile),
            Confirm::ApplyProfile(i, c) => Some(Action::ApplyProfile {
                profile: p.view.profiles.get(i)?.clone(),
                target: p.view.chars.as_ref()?.get(c)?.clone(),
            }),
            _ => None,
        },
        Hit::Cancel => {
            p.confirm = None;
            None
        }
        Hit::Confirm(i) => {
            p.confirm = None;
            p.view.points.get(i).map(|pt| Action::Restore {
                flavor: pt.flavor.clone(),
                id: pt.id.clone(),
                kind: pt.kind,
            })
        }
    })
    .flatten();
    if close {
        unsafe { DestroyWindow(hwnd) };
    } else {
        unsafe { InvalidateRect(hwnd, null(), 0) };
    }
    if let Some(action) = action {
        tray::act(action);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = zeroed();
                BeginPaint(hwnd, &mut ps);
                with_panel(|p| {
                    let (view, theme, fmt, tab, hover, confirm, sheet, first) =
                        (&p.view, &p.theme, &p.fmt, p.tab, p.hover, p.confirm, p.sheet.as_ref(), p.prof_first);
                    p.surface.paint(|painter| draw(painter, view, theme, fmt, tab, hover, confirm, sheet, first));
                });
                EndPaint(hwnd, &ps);
                0
            }
            WM_ERASEBKGND => 1,
            WM_MOUSEMOVE => {
                let changed = with_panel(|p| {
                    if !p.tracking {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        TrackMouseEvent(&mut tme);
                        p.tracking = true;
                    }
                    let h = hit_at(p, lparam);
                    let changed = h != p.hover;
                    p.hover = h;
                    changed
                });
                if changed == Some(true) {
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_MOUSELEAVE => {
                with_panel(|p| {
                    p.tracking = false;
                    p.hover = None;
                });
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_LBUTTONUP => {
                if let Some(hit) = with_panel(|p| hit_at(p, lparam)).flatten() {
                    click(hwnd, hit);
                }
                0
            }
            WM_SETCURSOR => {
                let hand = with_panel(|p| {
                    p.hover.is_some_and(|h| {
                        !matches!(
                            h,
                            Hit::History(_) | Hit::CleanInfo(_) | Hit::Sheet | Hit::ProfileInfo(_) | Hit::SaveInfo | Hit::ApplyBlocked(_)
                        )
                    })
                })
                .unwrap_or(false);
                SetCursor(LoadCursorW(null_mut(), if hand { IDC_HAND } else { IDC_ARROW }));
                1
            }
            WM_KEYDOWN if wparam == VK_ESCAPE as usize => {
                // Esc clears the search, then closes an open list, then the panel.
                let handled = with_panel(|p| match p.sheet.as_mut() {
                    Some(s) if !s.search.is_empty() => {
                        s.search.clear();
                        s.first = 0;
                        true
                    }
                    Some(_) => {
                        p.sheet = None;
                        true
                    }
                    None => false,
                });
                if handled == Some(true) {
                    InvalidateRect(hwnd, null(), 0);
                } else {
                    DestroyWindow(hwnd);
                }
                0
            }
            // Typing goes to the search field of a pick list; Enter picks the first one shown.
            WM_CHAR => {
                let typed = char::from_u32(wparam as u32);
                let enter = with_panel(|p| {
                    let s = p.sheet.as_mut().filter(|s| picks(s.list))?;
                    match typed? {
                        '\r' => return Some(true),
                        '\u{8}' => {
                            s.search.pop();
                        }
                        c if !c.is_control() => s.search.push(c),
                        _ => return None,
                    }
                    s.first = 0;
                    Some(false)
                })
                .flatten();
                match enter {
                    Some(true) => click(hwnd, Hit::SheetRow(0)),
                    Some(false) => {
                        InvalidateRect(hwnd, null(), 0);
                    }
                    None => {}
                }
                0
            }
            WM_TIMER if wparam == SPIN_TIMER => {
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_MOUSEWHEEL => {
                let up = ((wparam >> 16) & 0xFFFF) as u16 as i16 > 0;
                let changed = with_panel(|p| {
                    let Some(s) = p.sheet.as_mut() else {
                        if p.tab != Tab::Profiles {
                            return false;
                        }
                        let first = profile_first(&p.view, p.prof_first);
                        p.prof_first = profile_first(&p.view, if up { first.saturating_sub(1) } else { first + 1 });
                        return p.prof_first != first;
                    };
                    let first = sheet_first(&p.view, s);
                    s.first = if up { first.saturating_sub(3) } else { first + 3 };
                    s.first = sheet_first(&p.view, s);
                    s.first != first
                });
                if changed == Some(true) {
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            // Clicking anywhere else closes the panel, like Windows flyouts.
            WM_ACTIVATE if (wparam & 0xFFFF) as u32 == WA_INACTIVE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                LAST_CLOSE.store(GetTickCount64(), Ordering::Relaxed);
                let p = PANEL.with(|p| p.borrow_mut().take());
                drop(p);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Renders the panel with `view` into a PNG, for checking the look.
/// `tip`: hover history row `tip`, or cleanup line `tip` on that tab (its full text shows);`update_tip`: hover the new version button.
#[allow(clippy::too_many_arguments)]
pub fn snapshot(
    path: &str,
    dark: bool,
    view: &View,
    tab: Tab,
    hover_button: bool,
    confirm: Option<Confirm>,
    tip: Option<usize>,
    update_tip: bool,
    sheet: Option<Sheet>,
) -> gfx::Result<()> {
    let theme = gfx::theme_for(dark);
    let fmt = formats()?;
    let button = match tab {
        Tab::Addons => Hit::Update,
        Tab::Backup => Hit::Backup,
        Tab::Cleanup => Hit::Clean(Item::Backups),
        Tab::Profiles => Hit::SaveProfile,
    };
    let hover = tip
        .map(match tab {
            Tab::Cleanup => Hit::CleanInfo,
            // With a game running the demo hovers the blocked apply button.
            Tab::Profiles if !view.running.is_empty() => Hit::ApplyBlocked,
            Tab::Profiles => Hit::ProfileInfo,
            _ => Hit::History,
        })
        .or(update_tip.then_some(Hit::SelfUpdate))
        .or(hover_button.then_some(button));
    gfx::render_png(path, W, height(view), 1.5, |p| {
        draw(p, view, &theme, &fmt, tab, hover, confirm, sheet.as_ref(), 0)
    })
}
