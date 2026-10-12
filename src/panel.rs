//! The flyout opened by a left click on the tray icon, with two tabs: "Бэкап" (status, "backup now",
//! restore points with an inline confirmation) and "Аддоны" (last update, "update now", the last
//! runs), and the autostart switch under both. The window exists only while open.

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::{self, Align, Painter, R, Surface, Theme};
use crate::tray::{self, Action, Phase, RestorePoint, Status};
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
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tab {
    Backup,
    Addons,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Backup => tr!("panel.tab_backup"),
            Tab::Addons => tr!("panel.tab_addons"),
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
}

/// List rows: the longer of the two tabs, so switching tabs does not resize the panel.
fn rows(v: &View) -> usize {
    v.points.len().max(v.updates.len()).max(1)
}

fn divider_y(v: &View) -> f32 {
    LIST_Y + rows(v) as f32 * ROW_STEP + 6.0
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
    [Tab::Backup, Tab::Addons]
        .into_iter()
        .map(|tab| {
            let w = gfx::text_width(&f.tab_on, tab.label()) + 28.0;
            let r = R { x, y: TABS_Y, w, h: TABS_H };
            x += w + 2.0;
            (r, tab)
        })
        .collect()
}

fn hits(v: &View, f: &Formats, tab: Tab, confirm: Option<usize>) -> Vec<(R, Hit)> {
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
    if v.status.phase != Phase::Busy {
        out.push((button_rect(), Hit::Backup));
    }
    {
        for i in (0..v.points.len()).filter(|&i| restore_allowed(v, i)) {
            let row = row_rect(i);
            if confirm == Some(i) {
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

fn draw(p: &Painter, v: &View, t: &Theme, f: &Formats, tab: Tab, hover: Option<Hit>, confirm: Option<usize>) {
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
        Tab::Backup => draw_backup(p, v, t, f, hover, confirm),
        Tab::Addons => draw_addons(p, v, t, f, hover),
    }
    draw_footer(p, v, t, f, hover);
    if let (Tab::Addons, Some(Hit::History(i))) = (tab, hover) {
        draw_history_tip(p, v, t, f, i);
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
    let row = row_rect(i);
    let h = gfx::text_height(&f.wrap, &e.text, row.w - 28.0) + 24.0;
    // Under the row when it fits above the footer, else over it.
    let below = row.y + row.h + 4.0;
    let y = if below + h <= divider_y(v) { below } else { (row.y - 4.0 - h).max(STATUS_Y) };
    draw_tip(p, t, f, R { y, ..row }, &e.text, !e.ok);
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
    confirm: Option<usize>,
    tracking: bool,
    anchor_bottom: bool,
}

thread_local! {
    static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
}

/// When the panel last closed: a click on the tray icon first deactivates (closes) it,
/// and must not reopen it right away.
static LAST_CLOSE: AtomicU64 = AtomicU64::new(0);
/// The "Аддоны" tab was the last one open; the panel reopens on it.
static ADDONS_TAB: AtomicBool = AtomicBool::new(false);

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
        tab: if ADDONS_TAB.load(Ordering::Relaxed) { Tab::Addons } else { Tab::Backup },
        hover: None,
        confirm: None,
        tracking: false,
        anchor_bottom: edge == 3,
    };
    PANEL.with(|p| *p.borrow_mut() = Some(panel));
    unsafe {
        gfx::show(hwnd);
        SetForegroundWindow(hwnd);
    }
    Ok(())
}

/// Re-reads the state (after a backup, a restore, a settings change) and redraws.
pub fn refresh() {
    let Some(hwnd) = with_panel(|p| {
        p.view = tray::view();
        if p.confirm.is_some_and(|i| !restore_allowed(&p.view, i)) {
            p.confirm = None;
        }
        p.surface.hwnd
    }) else {
        return;
    };
    unsafe {
        fit_height(hwnd);
        InvalidateRect(hwnd, null(), 0);
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
    hits(&p.view, &p.fmt, p.tab, p.confirm)
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
            ADDONS_TAB.store(tab == Tab::Addons, Ordering::Relaxed);
            None
        }
        Hit::Autostart => Some(Action::Autostart),
        Hit::SelfUpdate => {
            close = true;
            Some(Action::InstallUpdate)
        }
        Hit::Restore(i) => {
            p.confirm = Some(i);
            None
        }
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
                    let (view, theme, fmt, tab, hover, confirm) = (&p.view, &p.theme, &p.fmt, p.tab, p.hover, p.confirm);
                    p.surface.paint(|painter| draw(painter, view, theme, fmt, tab, hover, confirm));
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
                let hand = with_panel(|p| p.hover.is_some_and(|h| !matches!(h, Hit::History(_)))).unwrap_or(false);
                SetCursor(LoadCursorW(null_mut(), if hand { IDC_HAND } else { IDC_ARROW }));
                1
            }
            WM_KEYDOWN if wparam == VK_ESCAPE as usize => {
                DestroyWindow(hwnd);
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
/// `tip`: hover history row `tip` (its full text shows); `update_tip`: hover the new version button.
#[allow(clippy::too_many_arguments)]
pub fn snapshot(
    path: &str,
    dark: bool,
    view: &View,
    tab: Tab,
    hover_button: bool,
    confirm: Option<usize>,
    tip: Option<usize>,
    update_tip: bool,
) -> gfx::Result<()> {
    let theme = gfx::theme_for(dark);
    let fmt = formats()?;
    let button = if tab == Tab::Addons { Hit::Update } else { Hit::Backup };
    let hover = tip
        .map(Hit::History)
        .or(update_tip.then_some(Hit::SelfUpdate))
        .or(hover_button.then_some(button));
    gfx::render_png(path, W, height(view), 1.5, |p| draw(p, view, &theme, &fmt, tab, hover, confirm))
}
