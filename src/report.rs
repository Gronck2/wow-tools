//! The macro report: every finding of the last macro check, in full, in a window of its own (opened from the panel's
//! Macros tab). A regular resizable window like the settings, with a header (counts per level, "Copy") that stays
//! put and the findings as cards scrolled by pixels under it. Size and position are remembered between openings.

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_NEXT, VK_PRIOR, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::config::APP_NAME;
use crate::gfx::{self, Align, Painter, R, Surface, Theme};
use crate::macros::{Finding, Level, Scan};
use crate::tray::{self, Action, MacroCheck, Tone};

/// The first size, in DIPs of client area.
const W0: f32 = 900.0;
const H0: f32 = 600.0;
const MIN_W: f32 = 520.0;
const MIN_H: f32 = 360.0;
const PAD: f32 = 20.0;
/// The header: counts and the copy button; it stays when the cards scroll.
const HEAD_H: f32 = 84.0;
const TOP: f32 = 12.0;
const GAP: f32 = 8.0;
/// A card ends this far from the right edge, which leaves room for the scroll bar.
const CARDS_R: f32 = 28.0;
const INSET_L: f32 = 18.0;
const INSET_R: f32 = 14.0;
const INSET_Y: f32 = 10.0;
const WHEEL_STEP: f32 = 64.0;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_MOUSEWHEEL: u32 = 0x020A;

struct Formats {
    head: IDWriteTextFormat,
    title: IDWriteTextFormat,
    sub: IDWriteTextFormat,
    body: IDWriteTextFormat,
    small: IDWriteTextFormat,
    small_right: IDWriteTextFormat,
    button: IDWriteTextFormat,
}

fn formats() -> gfx::Result<Formats> {
    let dw = gfx::dwrite()?;
    Ok(Formats {
        head: gfx::text_format(&dw, gfx::UI_FONT, 16.0, true, Align::Left)?,
        title: gfx::wrap_format(&dw, 14.0, true)?,
        sub: gfx::wrap_format(&dw, 12.0, false)?,
        body: gfx::wrap_format(&dw, 13.0, false)?,
        small: gfx::text_format(&dw, gfx::UI_FONT, 12.0, false, Align::Left)?,
        small_right: gfx::text_format(&dw, gfx::UI_FONT, 12.0, false, Align::Right)?,
        button: gfx::text_format(&dw, gfx::UI_FONT, 13.0, false, Align::Center)?,
    })
}

// ---------------------------------------------------------------- layout

/// What a card says, each part wrapped on its own line(s).
struct Texts {
    title: String,
    place: String,
    problem: String,
    fix: Option<String>,
}

/// `many`: the findings come from several game versions, so the card names it.
fn texts(f: &Finding, many: bool) -> Texts {
    Texts {
        title: if f.line == 0 {
            f.name.clone()
        } else {
            tr!("macros.report_line", name = f.name, line = f.line)
        },
        place: if many { format!("{} · {}", f.flavor, f.scope) } else { f.scope.clone() },
        problem: f.problem(),
        fix: f.fix().map(|x| format!("→ {x}")),
    }
}

fn card_height(f: &Formats, t: &Texts, text_w: f32) -> f32 {
    let h = |fmt, s| gfx::text_height(fmt, s, text_w);
    2.0 * INSET_Y + h(&f.title, &t.title) + 2.0 + h(&f.sub, &t.place) + 6.0 + h(&f.body, &t.problem) + t.fix.as_ref().map_or(0.0, |x| 4.0 + h(&f.body, x))
}

struct Card {
    texts: Texts,
    level: Level,
    top: f32,
    h: f32,
}

/// The cards for a window `width` DIPs wide; `total` is the height of everything, tops relative to the cards' area.
struct Layout {
    cards: Vec<Card>,
    total: f32,
}

fn layout(scan: Option<&Scan>, f: &Formats, width: f32) -> Layout {
    let findings = scan.map_or(&[][..], |s| &s.findings[..]);
    let many = findings.iter().any(|x| x.flavor != findings[0].flavor);
    let text_w = (width - PAD - CARDS_R - INSET_L - INSET_R).max(60.0);
    let mut y = TOP;
    let cards = findings
        .iter()
        .map(|x| {
            let texts = texts(x, many);
            let h = card_height(f, &texts, text_w);
            let card = Card {
                texts,
                level: x.level(),
                top: y,
                h,
            };
            y += h + GAP;
            card
        })
        .collect();
    Layout {
        cards,
        total: if findings.is_empty() { 0.0 } else { y - GAP + TOP },
    }
}

fn max_scroll(total: f32, view_h: f32) -> f32 {
    (total - view_h).max(0.0)
}

/// The scroll bar's track, under the header at the right edge.
fn track(size: (f32, f32)) -> R {
    R {
        x: size.0 - 14.0,
        y: HEAD_H + 4.0,
        w: 6.0,
        h: (size.1 - HEAD_H - 8.0).max(1.0),
    }
}

/// The thumb on the track; `None` when everything fits.
fn thumb(size: (f32, f32), total: f32, scroll: f32) -> Option<R> {
    let view_h = size.1 - HEAD_H;
    let max = max_scroll(total, view_h);
    if max <= 0.0 {
        return None;
    }
    let tr = track(size);
    let h = (tr.h * view_h / total).clamp(30.0, tr.h);
    Some(R {
        y: tr.y + (tr.h - h) * scroll.clamp(0.0, max) / max,
        h,
        ..tr
    })
}

fn copy_rect(f: &Formats, width: f32) -> R {
    let w = (gfx::text_width(&f.button, tr!("macros.copy")) + 28.0).max(84.0);
    R {
        x: width - PAD - w,
        y: 12.0,
        w,
        h: 30.0,
    }
}

// ---------------------------------------------------------------- drawing

fn level_color(l: Level, t: &Theme) -> D2D1_COLOR_F {
    match l {
        Level::Broken => t.error,
        Level::Warn => t.warn,
        Level::Info => t.text2,
    }
}

fn tone_color(tone: Tone, t: &Theme) -> D2D1_COLOR_F {
    match tone {
        Tone::Ok => t.ok,
        Tone::Notice => t.warn,
        Tone::Error => t.error,
    }
}

fn head_text(scan: Option<&Scan>) -> String {
    match scan {
        None => tr!("macros.not_checked").to_string(),
        Some(s) if s.findings.is_empty() => format!("{} · {}", tr!("macros.clean"), tr!("macros.checked", n = s.checked)),
        Some(s) => tr!("macros.report_head", n = s.findings.len(), checked = s.checked),
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(p: &Painter, t: &Theme, f: &Formats, check: &MacroCheck, lay: &Layout, size: (f32, f32), scroll: f32, copy_hover: bool) {
    let (cw, ch) = size;
    let scan = check.scan.as_deref();
    p.clear(t.bg);
    let view_h = ch - HEAD_H;
    let card_w = cw - PAD - CARDS_R;
    let text_w = (card_w - INSET_L - INSET_R).max(60.0);
    p.offset(0.0, HEAD_H - scroll);
    for c in lay.cards.iter().filter(|c| c.top + c.h >= scroll && c.top <= scroll + view_h) {
        let r = R {
            x: PAD,
            y: c.top,
            w: card_w,
            h: c.h,
        };
        p.fill(r, 6.0, t.card);
        p.stroke(r, 6.0, t.stroke, 1.0);
        p.fill(
            R {
                x: r.x + 6.0,
                y: r.y + 8.0,
                w: 4.0,
                h: r.h - 16.0,
            },
            2.0,
            level_color(c.level, t),
        );
        let x = r.x + INSET_L;
        let mut y = r.y + INSET_Y;
        let mut line = |s: &str, fmt: &IDWriteTextFormat, gap: f32, color: D2D1_COLOR_F| {
            let h = gfx::text_height(fmt, s, text_w);
            p.text(s, fmt, R { x, y, w: text_w, h }, color);
            y += h + gap;
        };
        line(&c.texts.title, &f.title, 2.0, t.text);
        line(&c.texts.place, &f.sub, 6.0, t.text2);
        line(&c.texts.problem, &f.body, 4.0, t.text);
        if let Some(fix) = &c.texts.fix {
            line(fix, &f.body, 0.0, t.accent);
        }
    }
    p.offset(0.0, 0.0);
    if lay.cards.is_empty() {
        let none = if scan.is_some() {
            tr!("macros.counts_clean")
        } else {
            tr!("macros.report_none")
        };
        p.text(
            none,
            &f.small,
            R {
                x: PAD,
                y: HEAD_H + TOP,
                w: cw - 2.0 * PAD,
                h: 20.0,
            },
            t.text2,
        );
    }
    if let Some(th) = thumb(size, lay.total, scroll) {
        p.fill(th, 3.0, t.text2);
    }

    // The header covers the cards scrolled under it.
    p.fill(
        R {
            x: 0.0,
            y: 0.0,
            w: cw,
            h: HEAD_H,
        },
        0.0,
        t.bg,
    );
    p.fill(
        R {
            x: 0.0,
            y: HEAD_H - 1.0,
            w: cw,
            h: 1.0,
        },
        0.0,
        t.stroke,
    );
    let copy = copy_rect(f, cw);
    p.text(
        &head_text(scan),
        &f.head,
        R {
            x: PAD,
            y: 12.0,
            w: (copy.x - PAD - 12.0).max(10.0),
            h: 30.0,
        },
        t.text,
    );
    let enabled = scan.is_some();
    p.fill(copy, 4.0, if copy_hover && enabled { t.card_hover } else { t.card });
    p.stroke(copy, 4.0, t.stroke, 1.0);
    p.text(tr!("macros.copy"), &f.button, copy, if enabled { t.text } else { t.text2 });
    let mut x = PAD;
    for (level, n) in scan.map_or(Vec::new(), Scan::levels) {
        let label = format!("{}: {n}", level.label());
        p.circle(x + 5.0, 62.0, 5.0, level_color(level, t));
        let w = gfx::text_width(&f.small, &label);
        p.text(
            &label,
            &f.small,
            R {
                x: x + 16.0,
                y: 52.0,
                w: w + 4.0,
                h: 20.0,
            },
            t.text,
        );
        x += 16.0 + w + 20.0;
    }
    if let Some((text, tone)) = &check.copied
        && cw - PAD - x > 60.0
    {
        let text = text.lines().collect::<Vec<_>>().join(" · ");
        p.text(
            &text,
            &f.small_right,
            R {
                x,
                y: 52.0,
                w: cw - PAD - x,
                h: 20.0,
            },
            tone_color(*tone, t),
        );
    }
}

// ---------------------------------------------------------------- remembered place

fn pos_file() -> std::path::PathBuf {
    crate::config::data_dir().join("macro-report-position.txt")
}

/// "x y w h" of the window, in screen pixels.
fn parse_pos(text: &str) -> Option<(i32, i32, i32, i32)> {
    let mut it = text.split_whitespace().map(|s| s.parse::<i32>());
    let (x, y, w, h) = (it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?);
    (it.next().is_none() && w >= 200 && h >= 150).then_some((x, y, w, h))
}

fn format_pos(x: i32, y: i32, w: i32, h: i32) -> String {
    format!("{x} {y} {w} {h}")
}

/// The remembered place, if its title bar is still on some monitor.
fn saved_pos() -> Option<(i32, i32, i32, i32)> {
    let (x, y, w, h) = parse_pos(&std::fs::read_to_string(pos_file()).ok()?)?;
    gfx::monitor_at(POINT { x: x + w / 2, y: y + 12 })?;
    Some((x, y, w, h))
}

fn save_pos(hwnd: HWND) {
    let mut wp: WINDOWPLACEMENT = unsafe { zeroed() };
    wp.length = size_of::<WINDOWPLACEMENT>() as u32;
    // The restored place, also while the window is minimized or maximized.
    unsafe { GetWindowPlacement(hwnd, &mut wp) };
    let r = wp.rcNormalPosition;
    if let Err(e) = std::fs::write(pos_file(), format_pos(r.left, r.top, r.right - r.left, r.bottom - r.top)) {
        log::warn!("{}: {e}", pos_file().display());
    }
}

// ---------------------------------------------------------------- window

struct Window {
    surface: Surface,
    fmt: Formats,
    theme: Theme,
    check: MacroCheck,
    lay: Layout,
    scroll: f32,
    copy_hover: bool,
    tracking: bool,
    /// The thumb is being dragged: where it was grabbed, from its top.
    grab: Option<f32>,
}

thread_local! {
    static WINDOW: RefCell<Option<Window>> = const { RefCell::new(None) };
}

fn with_window<T>(f: impl FnOnce(&mut Window) -> T) -> Option<T> {
    WINDOW.with(|w| w.borrow_mut().as_mut().map(f))
}

impl Window {
    fn size(&self) -> (f32, f32) {
        let mut rc: RECT = unsafe { zeroed() };
        unsafe { GetClientRect(self.surface.hwnd, &mut rc) };
        (
            (rc.right - rc.left) as f32 / self.surface.scale,
            (rc.bottom - rc.top) as f32 / self.surface.scale,
        )
    }

    fn max_scroll(&self) -> f32 {
        max_scroll(self.lay.total, self.size().1 - HEAD_H)
    }

    fn scroll_to(&mut self, to: f32) {
        self.scroll = to.clamp(0.0, self.max_scroll());
    }

    /// New width or new findings: the cards are measured again.
    fn relayout(&mut self) {
        self.lay = layout(self.check.scan.as_deref(), &self.fmt, self.size().0);
        self.scroll_to(self.scroll);
    }
}

/// Closes the report window if it is open (before the tray exits, see `panel::close`).
pub fn close() {
    if let Some(hwnd) = with_window(|w| w.surface.hwnd) {
        unsafe { DestroyWindow(hwnd) };
    }
}

/// Opens the report, or brings the open one up.
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
    let theme = gfx::theme();
    let title = tr!("macros.report_window", app = APP_NAME);
    let fmt = formats()?;
    let (hwnd, scale) = gfx::create_window("WowToolsMacroReport", &title, Some(wndproc), (W0, H0), &theme, true);
    if let Some((x, y, w, h)) = saved_pos() {
        unsafe { SetWindowPos(hwnd, null_mut(), x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE) };
    }
    let icon = tray::app_icon();
    let check = tray::macro_check();
    let mut window = Window {
        surface: Surface::new(hwnd, scale)?,
        fmt,
        theme,
        lay: Layout { cards: Vec::new(), total: 0.0 },
        check,
        scroll: 0.0,
        copy_hover: false,
        tracking: false,
        grab: None,
    };
    window.relayout();
    WINDOW.with(|w| *w.borrow_mut() = Some(window));
    unsafe {
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon as isize);
        gfx::show(hwnd);
        SetForegroundWindow(hwnd);
    }
    Ok(())
}

/// Takes the tray's latest check (a re-check ended, a copy was done) and redraws.
pub fn refresh() {
    let check = tray::macro_check();
    if let Some(hwnd) = with_window(|w| {
        w.check = check;
        w.relayout();
        w.surface.hwnd
    }) {
        unsafe { InvalidateRect(hwnd, null(), 0) };
    }
}

fn mouse(w: &Window, lparam: LPARAM) -> (f32, f32) {
    (
        (lparam & 0xFFFF) as i16 as f32 / w.surface.scale,
        ((lparam >> 16) & 0xFFFF) as i16 as f32 / w.surface.scale,
    )
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = zeroed();
                BeginPaint(hwnd, &mut ps);
                with_window(|w| {
                    let size = w.size();
                    let (theme, fmt, check, lay, scroll, hover) = (&w.theme, &w.fmt, &w.check, &w.lay, w.scroll, w.copy_hover);
                    w.surface.paint(|p| draw(p, theme, fmt, check, lay, size, scroll, hover));
                });
                EndPaint(hwnd, &ps);
                0
            }
            WM_ERASEBKGND => 1,
            WM_SIZE if wparam != SIZE_MINIMIZED as usize => {
                with_window(|w| {
                    w.surface.reset();
                    w.relayout();
                });
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_GETMINMAXINFO => {
                let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
                let mmi = &mut *(lparam as *mut MINMAXINFO);
                mmi.ptMinTrackSize = POINT {
                    x: (MIN_W * scale) as i32,
                    y: (MIN_H * scale) as i32,
                };
                0
            }
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
                    let (x, y) = mouse(w, lparam);
                    if let Some(grab) = w.grab {
                        let size = w.size();
                        if let Some(th) = thumb(size, w.lay.total, w.scroll) {
                            let tr = track(size);
                            let free = tr.h - th.h;
                            if free > 0.0 {
                                w.scroll_to((y - grab - tr.y) / free * w.max_scroll());
                            }
                        }
                        return true;
                    }
                    let over = w.check.scan.is_some() && copy_rect(&w.fmt, w.size().0).contains(x, y);
                    std::mem::replace(&mut w.copy_hover, over) != over
                });
                if changed == Some(true) {
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_MOUSELEAVE => {
                with_window(|w| {
                    w.tracking = false;
                    w.copy_hover = false;
                });
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam >> 16) & 0xFFFF) as i16 as f32 / 120.0;
                with_window(|w| w.scroll_to(w.scroll - delta * WHEEL_STEP));
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_LBUTTONDOWN => {
                let grabbed = with_window(|w| {
                    let (x, y) = mouse(w, lparam);
                    let size = w.size();
                    if y < HEAD_H || x < size.0 - 24.0 {
                        return false;
                    }
                    let Some(th) = thumb(size, w.lay.total, w.scroll) else { return false };
                    if th.contains(th.x, y) {
                        w.grab = Some(y - th.y);
                    } else {
                        // A click on the track pages toward it.
                        let page = size.1 - HEAD_H - 40.0;
                        w.scroll_to(w.scroll + if y < th.y { -page } else { page });
                    }
                    true
                });
                if grabbed == Some(true) {
                    SetCapture(hwnd);
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_LBUTTONUP => {
                let dragged = with_window(|w| w.grab.take().is_some());
                if dragged == Some(true) {
                    ReleaseCapture();
                } else if with_window(|w| {
                    let (x, y) = mouse(w, lparam);
                    w.check.scan.is_some() && copy_rect(&w.fmt, w.size().0).contains(x, y)
                }) == Some(true)
                {
                    tray::act(Action::CopyMacros);
                }
                0
            }
            WM_SETCURSOR if (lparam & 0xFFFF) as u32 == HTCLIENT => {
                let hand = with_window(|w| w.copy_hover).unwrap_or(false);
                SetCursor(LoadCursorW(null_mut(), if hand { IDC_HAND } else { IDC_ARROW }));
                1
            }
            WM_KEYDOWN => {
                let key = wparam as u16;
                if key == VK_ESCAPE {
                    DestroyWindow(hwnd);
                    return 0;
                }
                with_window(|w| {
                    let page = w.size().1 - HEAD_H - 40.0;
                    match key {
                        VK_UP => w.scroll_to(w.scroll - WHEEL_STEP),
                        VK_DOWN => w.scroll_to(w.scroll + WHEEL_STEP),
                        VK_PRIOR => w.scroll_to(w.scroll - page),
                        VK_NEXT => w.scroll_to(w.scroll + page),
                        VK_HOME => w.scroll_to(0.0),
                        VK_END => w.scroll_to(f32::MAX),
                        _ => {}
                    }
                });
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_EXITSIZEMOVE => {
                save_pos(hwnd);
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
                save_pos(hwnd);
                let w = WINDOW.with(|w| w.borrow_mut().take());
                drop(w);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Renders the window's content into a PNG at the first size, for checking the look.
pub fn snapshot(path: &str, dark: bool, check: &MacroCheck) -> gfx::Result<()> {
    let theme = gfx::theme_for(dark);
    let fmt = formats()?;
    let lay = layout(check.scan.as_deref(), &fmt, W0);
    gfx::render_png(path, W0, H0, 1.5, |p| draw(p, &theme, &fmt, check, &lay, (W0, H0), 0.0, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macros::{How, Kind};

    fn finding(line: usize, kind: Kind) -> Finding {
        Finding {
            flavor: "Retail".into(),
            scope: "Acc".into(),
            name: "Heal".into(),
            line,
            kind,
        }
    }

    fn scan(findings: Vec<Finding>) -> Scan {
        Scan {
            findings,
            checked: 7,
            ..Default::default()
        }
    }

    fn unknown(name: &str) -> Kind {
        Kind::UnknownName { name: name.into() }
    }

    #[test]
    fn cards_grow_with_the_text_and_with_a_narrower_window() {
        let f = formats().unwrap();
        let short = scan(vec![finding(1, Kind::Brackets)]);
        let long = scan(vec![finding(1, unknown(&"Name of a very long spell ".repeat(12)))]);
        let (a, b) = (layout(Some(&short), &f, 900.0), layout(Some(&long), &f, 900.0));
        assert!(b.cards[0].h > a.cards[0].h);
        assert!(layout(Some(&long), &f, 520.0).cards[0].h > b.cards[0].h);
        // Everything is kept: the total covers the cards with the margins, and cards do not overlap.
        let two = layout(Some(&scan(vec![finding(1, Kind::Brackets), finding(2, Kind::Brackets)])), &f, 900.0);
        assert_eq!(two.cards[1].top, two.cards[0].top + two.cards[0].h + GAP);
        assert_eq!(two.total, two.cards[1].top + two.cards[1].h + TOP);
        assert_eq!(layout(None, &f, 900.0).total, 0.0);
    }

    #[test]
    fn a_card_names_the_game_version_only_with_several() {
        let mut other = finding(1, Kind::Brackets);
        other.flavor = "Era".into();
        let same = [finding(1, Kind::Brackets), finding(0, Kind::Brackets)];
        assert_eq!(texts(&same[0], false).place, "Acc");
        assert_eq!(texts(&other, true).place, "Era · Acc");
        assert_eq!(texts(&same[0], false).title, tr!("macros.report_line", name = "Heal", line = 1));
        assert_eq!(texts(&same[1], false).title, "Heal");
        assert!(texts(&same[0], false).fix.unwrap().starts_with("→ "));
        assert!(texts(&finding(1, unknown("x")), false).fix.is_none());
    }

    #[test]
    fn scroll_is_clamped_and_the_thumb_follows_it() {
        assert_eq!(max_scroll(300.0, 500.0), 0.0);
        assert_eq!(max_scroll(1500.0, 500.0), 1000.0);
        let size = (900.0, HEAD_H + 500.0);
        assert!(thumb(size, 400.0, 0.0).is_none());
        let (top, bottom) = (thumb(size, 1500.0, 0.0).unwrap(), thumb(size, 1500.0, 1000.0).unwrap());
        let tr = track(size);
        assert_eq!(top.y, tr.y);
        assert!((bottom.y + bottom.h - (tr.y + tr.h)).abs() < 0.01);
        // A scroll past the end (findings shrank) puts the thumb at the end, not outside the track.
        assert_eq!(thumb(size, 1500.0, 5000.0).unwrap().y, bottom.y);
        assert!(top.h >= 30.0 && top.h < tr.h);
    }

    #[test]
    fn counters_show_only_levels_that_occur() {
        let s = scan(vec![
            finding(1, Kind::Brackets),
            finding(2, Kind::Brackets),
            finding(3, Kind::Secret { func: "f" }),
        ]);
        assert_eq!(s.levels(), [(Level::Broken, 2), (Level::Warn, 1)]);
        let realm = Kind::WrongLanguage {
            name: "a".into(),
            lang: "deDE".into(),
            fix: "b".into(),
            bad: Vec::new(),
            how: How::Realm,
        };
        assert_eq!(scan(vec![finding(1, realm)]).levels(), [(Level::Info, 1)]);
        assert!(scan(Vec::new()).levels().is_empty());
        assert_eq!(head_text(None), tr!("macros.not_checked"));
        assert_eq!(head_text(Some(&s)), tr!("macros.report_head", n = 3, checked = 7));
    }

    #[test]
    fn position_file_round_trips_and_rejects_junk() {
        assert_eq!(parse_pos(&format_pos(-1200, 40, 900, 600)), Some((-1200, 40, 900, 600)));
        assert_eq!(parse_pos(" 10 20 800 500\n"), Some((10, 20, 800, 500)));
        assert_eq!(parse_pos("10 20 800"), None);
        assert_eq!(parse_pos("10 20 800 500 1"), None);
        assert_eq!(parse_pos("10 20 x 500"), None);
        assert_eq!(parse_pos("10 20 0 0"), None);
        assert_eq!(parse_pos(""), None);
    }
}
