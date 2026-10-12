//! Right-click menu of the tray icon, drawn in the style of the panel.

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::null;

use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::{self, Align, Painter, R, Surface, Theme};
use crate::tray::{self, Action};

const W: f32 = 280.0;
const PAD: f32 = 4.0;
const ITEM_H: f32 = 36.0;
const GAP: f32 = 9.0;
const WM_MOUSELEAVE: u32 = 0x02A3;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Item {
    Open,
    Backup,
    Update,
    Folder,
    Settings,
    Log,
    Autostart,
    Exit,
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Item(Item),
    Divider,
    Version,
}

const ROWS: [Row; 11] = [
    Row::Item(Item::Open),
    Row::Item(Item::Backup),
    Row::Item(Item::Update),
    Row::Divider,
    Row::Item(Item::Folder),
    Row::Item(Item::Settings),
    Row::Item(Item::Log),
    Row::Item(Item::Autostart),
    Row::Divider,
    Row::Version,
    Row::Item(Item::Exit),
];

fn row_h(row: Row) -> f32 {
    match row {
        Row::Divider => GAP,
        Row::Version => 28.0,
        Row::Item(_) => ITEM_H,
    }
}

fn height() -> f32 {
    2.0 * PAD + ROWS.iter().map(|&r| row_h(r)).sum::<f32>()
}

fn layout() -> Vec<(R, Row)> {
    let mut y = PAD;
    ROWS.iter()
        .map(|&row| {
            let r = R {
                x: PAD,
                y,
                w: W - 2.0 * PAD,
                h: row_h(row),
            };
            y += r.h;
            (r, row)
        })
        .collect()
}

struct Formats {
    text: IDWriteTextFormat,
    small: IDWriteTextFormat,
    icon: IDWriteTextFormat,
}

fn formats() -> gfx::Result<Formats> {
    let dw = gfx::dwrite()?;
    Ok(Formats {
        text: gfx::text_format(&dw, gfx::UI_FONT, 13.0, false, Align::Left)?,
        small: gfx::text_format(&dw, gfx::UI_FONT, 12.0, false, Align::Left)?,
        icon: gfx::text_format(&dw, gfx::ICON_FONT, 14.0, false, Align::Center)?,
    })
}

/// What can change between openings of the menu.
struct State {
    busy: bool,
    work: crate::watch::Work,
    autostart: bool,
    /// The addon manager updates go through, for the label.
    update_via: Option<&'static str>,
}

fn enabled(state: &State, item: Item) -> bool {
    !matches!(item, Item::Backup | Item::Update) || !state.busy
}

fn draw(p: &Painter, t: &Theme, f: &Formats, state: &State, hover: Option<Item>) {
    p.clear(t.card);
    for (r, row) in layout() {
        match row {
            Row::Divider => p.fill(
                R {
                    x: 0.0,
                    y: r.y + r.h / 2.0 - 0.5,
                    w: W,
                    h: 1.0,
                },
                0.0,
                t.stroke,
            ),
            Row::Version => {
                let label = format!("{} {}", crate::config::APP_NAME, env!("CARGO_PKG_VERSION"));
                p.text(
                    &label,
                    &f.small,
                    R {
                        x: r.x + 40.0,
                        y: r.y,
                        w: r.w - 44.0,
                        h: r.h,
                    },
                    t.text2,
                );
            }
            Row::Item(item) => {
                let on = enabled(state, item);
                if on && hover == Some(item) {
                    p.fill(r, 4.0, t.card_hover);
                }
                let fg = if on { t.text } else { t.text2 };
                let update = match state.update_via {
                    Some(app) => tr!("menu.update_via", app = app),
                    None => tr!("menu.update").into(),
                };
                let (icon, label) = match item {
                    Item::Open => ("\u{E8A7}", tr!("menu.open_panel")),
                    Item::Update => ("\u{E895}", update.as_str()),
                    Item::Backup => (
                        "\u{E74E}",
                        if state.busy {
                            crate::panel::busy_text(state.work)
                        } else {
                            tr!("panel.backup_now")
                        },
                    ),
                    Item::Folder => ("\u{E838}", tr!("menu.backups_folder")),
                    Item::Settings => ("\u{E713}", tr!("menu.settings")),
                    Item::Log => ("\u{E9F9}", tr!("menu.log")),
                    Item::Autostart => ("\u{E7E8}", tr!("panel.autostart")),
                    Item::Exit => ("\u{E711}", tr!("menu.exit")),
                };
                p.text(
                    icon,
                    &f.icon,
                    R {
                        x: r.x + 6.0,
                        y: r.y,
                        w: 28.0,
                        h: r.h,
                    },
                    fg,
                );
                let switch = item == Item::Autostart;
                let label_w = if switch { r.w - 100.0 } else { r.w - 44.0 };
                p.text(
                    label,
                    &f.text,
                    R {
                        x: r.x + 40.0,
                        y: r.y,
                        w: label_w,
                        h: r.h,
                    },
                    fg,
                );
                if switch {
                    let sw = R {
                        x: r.x + r.w - 46.0,
                        y: r.cy() - 9.0,
                        w: 36.0,
                        h: 18.0,
                    };
                    p.switch(sw, state.autostart, t);
                }
            }
        }
    }
}

struct Menu {
    surface: Surface,
    fmt: Formats,
    theme: Theme,
    state: State,
    hover: Option<Item>,
    tracking: bool,
}

thread_local! {
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
}

fn with_menu<T>(f: impl FnOnce(&mut Menu) -> T) -> Option<T> {
    MENU.with(|m| m.borrow_mut().as_mut().map(f))
}

/// Opens the menu at the mouse cursor, away from the taskbar.
pub fn show() {
    if MENU.with(|m| m.borrow().is_some()) {
        return;
    }
    if let Err(e) = open() {
        log::error!("{}", tr!("menu.failed", error = e));
    }
}

fn open() -> gfx::Result<()> {
    let theme = gfx::theme();
    let (pt, work, scale) = gfx::cursor_monitor();
    let (w, h) = ((W * scale).round() as i32, (height() * scale).round() as i32);
    // Opens from the cursor towards the screen center, inside the work area.
    let edge = gfx::taskbar_edge();
    let x = match edge {
        0 => pt.x,
        2 => pt.x - w,
        _ => pt.x - w / 2,
    }
    .clamp(work.left, work.right - w);
    let y = match edge {
        1 => pt.y,
        3 => pt.y - h,
        _ => pt.y - h / 2,
    }
    .clamp(work.top, work.bottom - h);
    let hwnd = gfx::create_popup("WowToolsMenu", Some(wndproc), (x, y, w, h), theme.dark, true);
    let view = tray::view();
    let menu = Menu {
        surface: Surface::new(hwnd, scale)?,
        fmt: formats()?,
        theme,
        state: State {
            busy: view.status.phase == tray::Phase::Busy,
            work: view.status.work,
            autostart: view.autostart,
            update_via: tray::config_path()
                .and_then(|p| crate::config::Config::load(&p).ok())
                .and_then(|c| crate::updater::chosen(c.update_app))
                .map(|a| a.title()),
        },
        hover: None,
        tracking: false,
    };
    MENU.with(|m| *m.borrow_mut() = Some(menu));
    unsafe {
        gfx::show(hwnd);
        SetForegroundWindow(hwnd);
    }
    Ok(())
}

fn item_at(lparam: LPARAM, scale: f32) -> Option<Item> {
    let x = (lparam & 0xFFFF) as i16 as f32 / scale;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as f32 / scale;
    layout().into_iter().find(|(r, _)| r.contains(x, y)).and_then(|(_, row)| match row {
        Row::Item(i) => Some(i),
        _ => None,
    })
}

fn run(item: Item) {
    let action = match item {
        Item::Open => Action::OpenPanel,
        Item::Backup => Action::BackupNow,
        Item::Update => Action::UpdateAddons,
        Item::Folder => Action::OpenBackups,
        Item::Settings => Action::OpenSettings,
        Item::Log => Action::OpenLog,
        Item::Autostart => Action::Autostart,
        Item::Exit => Action::Exit,
    };
    tray::act(action);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = zeroed();
                BeginPaint(hwnd, &mut ps);
                with_menu(|m| {
                    let (theme, fmt, state, hover) = (&m.theme, &m.fmt, &m.state, m.hover);
                    m.surface.paint(|p| draw(p, theme, fmt, state, hover));
                });
                EndPaint(hwnd, &ps);
                0
            }
            WM_ERASEBKGND => 1,
            WM_MOUSEMOVE => {
                let changed = with_menu(|m| {
                    if !m.tracking {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        TrackMouseEvent(&mut tme);
                        m.tracking = true;
                    }
                    let h = item_at(lparam, m.surface.scale);
                    let changed = h != m.hover;
                    m.hover = h;
                    changed
                });
                if changed == Some(true) {
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_MOUSELEAVE => {
                with_menu(|m| {
                    m.tracking = false;
                    m.hover = None;
                });
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_LBUTTONUP => {
                let picked = with_menu(|m| item_at(lparam, m.surface.scale).filter(|&i| enabled(&m.state, i))).flatten();
                if let Some(item) = picked {
                    DestroyWindow(hwnd);
                    run(item);
                }
                0
            }
            WM_KEYDOWN if wparam == VK_ESCAPE as usize => {
                DestroyWindow(hwnd);
                0
            }
            // Clicking anywhere else closes the menu, like a regular one.
            WM_ACTIVATE if (wparam & 0xFFFF) as u32 == WA_INACTIVE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                let m = MENU.with(|m| m.borrow_mut().take());
                drop(m);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Renders the menu into a PNG file, for checking the look.
pub fn snapshot(path: &str, dark: bool) -> gfx::Result<()> {
    let theme = gfx::theme_for(dark);
    let fmt = formats()?;
    let state = State {
        busy: false,
        work: crate::watch::Work::Backup,
        autostart: true,
        update_via: Some("WowUp"),
    };
    gfx::render_png(path, W, height(), 1.5, |p| draw(p, &theme, &fmt, &state, Some(Item::Backup)))
}
