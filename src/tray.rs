//! Tray icon: runs the `watch` loop on a worker thread; a left click opens the panel
//! (`panel.rs`), a right click the menu (`menu.rs`). The icon is drawn at runtime and shows
//! the state: green = ok, ring = game running (backup after exit), blue = working,
//! red = error, gray = no snapshots yet.

use crate::app::Ctx;
use crate::config::{self, APP_NAME, Config};
use crate::panel::{self, View};
use crate::snapshot::{self, Kind};
use crate::watch::{self, Event, Request, Work};
use crate::{menu, schedule, settings, wow};
use anyhow::{Result, bail};
use chrono::{Local, NaiveDateTime};
use std::cell::RefCell;
use std::ffi::OsStr;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject};
use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW, ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 1;
/// The worker changed `STATE`; refresh the icon and the panel, show a pending balloon.
const WM_STATE: u32 = WM_APP + 2;
/// Sent by a second instance: show the settings window.
const WM_SHOW_SETTINGS: u32 = WM_APP + 3;
/// Restore points the panel lists.
const POINTS: usize = 5;

#[derive(Clone, PartialEq, Debug)]
pub enum Phase {
    Idle,
    Busy,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct Status {
    pub phase: Phase,
    /// What `Phase::Busy` is busy with.
    pub work: Work,
    pub last: Option<NaiveDateTime>,
    pub game: Option<String>,
}

/// One WTF snapshot and the addons snapshot a restore to it would use.
#[derive(Clone, Debug)]
pub struct RestorePoint {
    pub flavor: String,
    /// "Classic Era", shown when more than one flavor is backed up.
    pub flavor_title: Option<String>,
    pub id: String,
    pub created: NaiveDateTime,
    pub wtf_size: u64,
    pub addons_size: Option<u64>,
    /// The flavor's client runs, so it cannot be restored now.
    pub running: bool,
    /// Only this series is restored: `Some(Addons)` for the snapshot taken before an addon update.
    pub kind: Option<Kind>,
}

impl RestorePoint {
    pub fn detail(&self) -> String {
        if self.kind == Some(Kind::Addons) {
            return tr!("points.addons_only", size = snapshot::human_size(self.addons_size.unwrap_or(0)));
        }
        let mut s = format!("WTF {}", snapshot::human_size(self.wtf_size));
        if let Some(a) = self.addons_size {
            s.push_str(&tr!("points.plus_addons", size = snapshot::human_size(a)));
        }
        s
    }

    /// Why this snapshot was taken, when it was not a regular one.
    pub fn note(&self) -> Option<&'static str> {
        if self.id.ends_with("-pre-restore") {
            Some(tr!("points.before_restore"))
        } else if self.id.ends_with("-pre-copy") {
            Some(tr!("points.before_copy"))
        } else if self.id.ends_with("-pre-update") {
            Some(tr!("points.before_update"))
        } else {
            None
        }
    }
}

/// UI actions, from the panel and the menu.
pub enum Action {
    OpenPanel,
    BackupNow,
    UpdateAddons,
    Restore {
        flavor: String,
        id: String,
        kind: Option<Kind>,
    },
    OpenBackups,
    /// The settings window.
    OpenSettings,
    /// `config.toml` in Notepad.
    OpenConfigFile,
    /// The settings window saved the config: refresh what shows it.
    SettingsChanged,
    OpenLog,
    Autostart,
    Exit,
}

struct Balloon {
    title: &'static str,
    text: String,
    error: bool,
}

struct State {
    status: Status,
    balloon: Option<Balloon>,
    /// Last error shown as a balloon, so a repeating error does not pop up every round.
    shown_error: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    status: Status {
        phase: Phase::Idle,
        work: Work::Backup,
        last: None,
        game: None,
    },
    balloon: None,
    shown_error: None,
});
static HWND_MAIN: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static ICON: AtomicIsize = AtomicIsize::new(0);

/// UI-thread-only data.
struct Ui {
    requests: Sender<Request>,
    config_path: PathBuf,
    explicit_config: Option<PathBuf>,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

fn copy_wide(dst: &mut [u16], s: &str) {
    for (i, ch) in s.encode_utf16().take(dst.len() - 1).enumerate() {
        dst[i] = ch;
    }
}

pub fn format_time(t: NaiveDateTime) -> String {
    let today = Local::now().date_naive();
    if t.date() == today {
        tr!("time.today", time = t.format("%H:%M"))
    } else if today.pred_opt() == Some(t.date()) {
        tr!("time.yesterday", time = t.format("%H:%M"))
    } else {
        t.format(tr!("time.date")).to_string()
    }
}

pub fn run(ctx: Ctx) -> Result<()> {
    let hwnd = unsafe {
        CreateMutexW(null(), 0, wide("Local\\wow-tools-tray").as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // A second start (a double click on the exe) opens the settings of the running one.
            log::info!("{}", tr!("tray.already_running"));
            let running = FindWindowW(wide("WowToolsTray").as_ptr(), null());
            if !running.is_null() {
                AllowSetForegroundWindow(ASFW_ANY);
                PostMessageW(running, WM_SHOW_SETTINGS, 0, 0);
            }
            return Ok(());
        }
        // Started by a double click: the console window was made just for us, close it.
        let mut ids = [0u32; 2];
        if GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) == 1 {
            FreeConsole();
        }

        let hinst = GetModuleHandleW(null());
        let class = wide("WowToolsTray");
        let mut wc: WNDCLASSW = zeroed();
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = hinst;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        // A hidden top-level window (not message-only) so it gets the TaskbarCreated broadcast.
        let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, null_mut(), null_mut(), hinst, null());
        if hwnd.is_null() {
            bail!(tr!("tray.no_window"));
        }
        HWND_MAIN.store(hwnd as isize, Ordering::Relaxed);
        TASKBAR_CREATED.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()), Ordering::Relaxed);
        hwnd
    };
    log::info!("{}", tr!("tray.started"));

    STATE.lock().unwrap().status.last = ctx.last_backup();
    let (tx, rx) = mpsc::channel();
    let (config_path, explicit_config) = (ctx.config_path.clone(), ctx.explicit_config.clone());
    std::thread::spawn(move || watch::run(&config_path, explicit_config.as_deref(), None, &rx, on_event));
    UI.with(|ui| {
        *ui.borrow_mut() = Some(Ui {
            requests: tx,
            config_path: ctx.config_path,
            explicit_config: ctx.explicit_config,
        })
    });

    unsafe {
        update_tray(hwnd, NIM_ADD);
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    log::info!("{}", tr!("tray.stopped"));
    Ok(())
}

// ---------------------------------------------------------------- state for the UI

fn ui_paths() -> Option<(PathBuf, Option<PathBuf>)> {
    UI.with(|ui| ui.borrow().as_ref().map(|u| (u.config_path.clone(), u.explicit_config.clone())))
}

/// The config file the tray works with.
pub fn config_path() -> Option<PathBuf> {
    ui_paths().map(|(path, _)| path)
}

/// The green tray icon, for the title bar and taskbar button of windows. Made once, never freed.
pub fn app_icon() -> HICON {
    static APP_ICON: AtomicIsize = AtomicIsize::new(0);
    let mut icon = APP_ICON.load(Ordering::Relaxed);
    if icon == 0 {
        icon = unsafe { make_icon((0x3B, 0xA5, 0x5D), false) } as isize;
        APP_ICON.store(icon, Ordering::Relaxed);
    }
    icon as HICON
}

fn load_ctx() -> Option<Ctx> {
    let (path, explicit) = ui_paths()?;
    Ctx::load(&path, explicit.as_deref()).ok()
}

fn restore_points(ctx: &Ctx) -> Vec<RestorePoint> {
    let Ok(flavors) = ctx.flavors(None) else { return Vec::new() };
    let store = ctx.store();
    let games = wow::running_games();
    let multi = flavors.len() > 1;
    let mut per_flavor: Vec<Vec<RestorePoint>> = flavors
        .iter()
        .map(|fl| {
            let wtf = store.list(fl, Kind::Wtf).unwrap_or_default();
            let addons = store.list(fl, Kind::Addons).unwrap_or_default();
            let running = wow::running_in(&games, fl).is_some();
            let point = |id: &str, created, wtf_size, addons_size, kind| RestorePoint {
                flavor: fl.name.clone(),
                flavor_title: multi.then(|| fl.title()),
                id: id.to_string(),
                created,
                wtf_size,
                addons_size,
                running,
                kind,
            };
            let full = wtf
                .iter()
                .map(|s| point(&s.id, s.created, s.size, snapshot::select(&addons, Some(&s.id)).map(|a| a.size), None));
            // Addons saved before an update are a point of their own: they roll back only the addons.
            let before_update = addons
                .iter()
                .filter(|a| a.id.ends_with("-pre-update"))
                .map(|a| point(&a.id, a.created, 0, Some(a.size), Some(Kind::Addons)));
            let mut list: Vec<RestorePoint> = full.chain(before_update).collect();
            list.sort_by_key(|p| std::cmp::Reverse(p.created));
            list.truncate(POINTS);
            list
        })
        .collect();
    pick_points(&mut per_flavor)
}

/// The newest point of every flavor, then the newest of the rest, newest first: a flavor
/// that is backed up rarely is not pushed out of the list by a busy one.
fn pick_points(per_flavor: &mut [Vec<RestorePoint>]) -> Vec<RestorePoint> {
    let mut out: Vec<RestorePoint> = per_flavor.iter_mut().filter(|l| !l.is_empty()).map(|l| l.remove(0)).collect();
    out.sort_by_key(|p| std::cmp::Reverse(p.created));
    out.truncate(POINTS);
    let mut rest: Vec<RestorePoint> = per_flavor.iter_mut().flat_map(std::mem::take).collect();
    rest.sort_by_key(|p| std::cmp::Reverse(p.created));
    out.extend(rest.into_iter().take(POINTS.saturating_sub(out.len())));
    out.sort_by_key(|p| std::cmp::Reverse(p.created));
    out
}

/// Everything the panel and the menu show, read fresh.
pub fn view() -> View {
    let ctx = load_ctx();
    let cfg = ctx.as_ref().map_or_else(Config::default, |c| c.cfg.clone());
    let update_text = if cfg.update_addons {
        upper_first(&cfg.update_text())
    } else {
        tr!("panel.auto_update_off").into()
    };
    let status = STATE.lock().unwrap().status.clone();
    View {
        update_waits_for_game: cfg.update_skip_in_game && status.game.is_some(),
        status,
        points: ctx.as_ref().map(restore_points).unwrap_or_default(),
        autostart: schedule::logon_command().is_some(),
        schedule: cfg.schedule_text(),
        update_app: crate::updater::chosen(cfg.update_app).map(|a| a.title()),
        update_text,
        last_update: crate::updater::last_run(),
        updates: crate::updater::history().into_iter().take(panel::HISTORY_ROWS).collect(),
    }
}

/// "каждый час" -> "Каждый час".
pub fn upper_first(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// Runs a UI action on the UI thread; failures become an error balloon.
pub fn act(action: Action) {
    let result = match action {
        Action::OpenPanel => {
            panel::toggle();
            Ok(())
        }
        Action::BackupNow => send(Request::BackupNow),
        Action::UpdateAddons => send(Request::UpdateAddons),
        Action::Restore { flavor, id, kind } => send(Request::Restore { flavor, id, kind }),
        Action::OpenBackups => open_backups(),
        Action::OpenSettings => {
            settings::show();
            Ok(())
        }
        Action::OpenConfigFile => open_config(),
        Action::SettingsChanged => {
            post_state();
            Ok(())
        }
        Action::OpenLog => open_in_notepad(&config::default_log_path()),
        Action::Autostart => toggle_autostart(),
        Action::Exit => {
            if STATE.lock().unwrap().status.phase == Phase::Busy {
                Err(anyhow::anyhow!(tr!("tray.busy_exit")))
            } else {
                unsafe { DestroyWindow(HWND_MAIN.load(Ordering::Relaxed) as HWND) };
                Ok(())
            }
        }
    };
    if let Err(e) = result {
        log::error!("{e:#}");
        STATE.lock().unwrap().balloon = Some(Balloon {
            title: tr!("tray.failed"),
            text: format!("{e:#}"),
            error: true,
        });
        post_state();
    }
}

fn send(request: Request) -> Result<()> {
    UI.with(|ui| match ui.borrow().as_ref() {
        Some(ui) => ui.requests.send(request).map_err(|_| anyhow::anyhow!(tr!("tray.worker_gone"))),
        None => Ok(()),
    })
}

fn post_state() {
    unsafe {
        PostMessageW(HWND_MAIN.load(Ordering::Relaxed) as HWND, WM_STATE, 0, 0);
    }
}

// ---------------------------------------------------------------- worker events

fn on_event(event: Event) {
    {
        let mut st = STATE.lock().unwrap();
        match event {
            Event::Busy(work) => {
                st.status.phase = Phase::Busy;
                st.status.work = work;
            }
            Event::Game(game) => st.status.game = game,
            Event::Backup { manual, result } => {
                let errors = match result {
                    Ok(report) if report.errors.is_empty() => {
                        if !report.created.is_empty() {
                            st.status.last = Some(Local::now().naive_local());
                        }
                        st.status.phase = Phase::Idle;
                        st.shown_error = None;
                        if manual {
                            let text = if report.created.is_empty() {
                                tr!("tray.backup_unchanged").to_string()
                            } else {
                                report.created.join("\n")
                            };
                            st.balloon = Some(Balloon {
                                title: tr!("tray.backup_done"),
                                text,
                                error: false,
                            });
                        }
                        None
                    }
                    Ok(report) => Some(report.errors.join("\n")),
                    Err(e) => Some(format!("{e:#}")),
                };
                if let Some(msg) = errors {
                    if manual || st.shown_error.as_ref() != Some(&msg) {
                        st.balloon = Some(Balloon {
                            title: tr!("tray.backup_failed"),
                            text: msg.clone(),
                            error: true,
                        });
                        st.shown_error = Some(msg.clone());
                    }
                    st.status.phase = Phase::Error(msg);
                }
            }
            Event::Addons { manual, result } => {
                st.status.phase = Phase::Idle;
                match result {
                    // A scheduled run with nothing new stays quiet.
                    Ok(summary) if manual || !summary.starts_with(tr!("addons.none")) => {
                        st.balloon = Some(Balloon {
                            title: tr!("tray.addons"),
                            text: summary,
                            error: false,
                        })
                    }
                    Ok(_) => {}
                    Err(e) => {
                        st.balloon = Some(Balloon {
                            title: tr!("tray.update_failed"),
                            text: format!("{e:#}"),
                            error: true,
                        })
                    }
                }
            }
            Event::Restore(result) => {
                st.status.phase = Phase::Idle;
                st.balloon = Some(match result {
                    Ok(summary) => Balloon {
                        title: tr!("tray.restored"),
                        text: summary,
                        error: false,
                    },
                    Err(e) => Balloon {
                        title: tr!("tray.restore_failed"),
                        text: format!("{e:#}"),
                        error: true,
                    },
                });
            }
        }
    }
    post_state();
}

// ---------------------------------------------------------------- window

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TRAY => {
                match (lp & 0xFFFF) as u32 {
                    WM_LBUTTONUP => panel::toggle(),
                    WM_RBUTTONUP => menu::show(),
                    _ => {}
                }
                0
            }
            WM_SHOW_SETTINGS => {
                settings::show();
                0
            }
            WM_STATE => {
                update_tray(hwnd, NIM_MODIFY);
                panel::refresh();
                settings::refresh();
                0
            }
            WM_DESTROY => {
                remove_tray(hwnd);
                PostQuitMessage(0);
                0
            }
            // Explorer restarted: the icon is gone and has to be added again.
            m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                update_tray(hwnd, NIM_ADD);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn tooltip(s: &Status) -> String {
    let mut tip = String::from(APP_NAME);
    let line = match s.last {
        Some(t) => tr!("panel.last_backup", time = format_time(t)),
        None => tr!("panel.no_backups").to_string(),
    };
    tip.push_str(&format!("\n{line}"));
    let line = match &s.phase {
        Phase::Busy => panel::busy_text(s.work).to_string(),
        Phase::Error(e) => tr!("panel.error", error = e),
        Phase::Idle if s.game.is_some() => tr!("panel.backup_after_game").to_string(),
        Phase::Idle => String::new(),
    };
    if !line.is_empty() {
        tip.push_str(&format!("\n{line}"));
    }
    tip
}

unsafe fn update_tray(hwnd: HWND, action: u32) {
    let (color, ring, tip, balloon) = {
        let mut st = STATE.lock().unwrap();
        let s = &st.status;
        let color = match (&s.phase, s.last) {
            (Phase::Busy, _) => (0x3B, 0x82, 0xF6),
            (Phase::Error(_), _) => (0xE5, 0x48, 0x4D),
            (Phase::Idle, Some(_)) => (0x3B, 0xA5, 0x5D),
            (Phase::Idle, None) => (0x90, 0x90, 0x90),
        };
        let ring = s.phase == Phase::Idle && s.game.is_some();
        let tip = tooltip(s);
        (color, ring, tip, st.balloon.take())
    };
    unsafe {
        let icon = make_icon(color, ring);
        let mut nid: NOTIFYICONDATAW = zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = icon;
        copy_wide(&mut nid.szTip, &tip);
        if let Some(b) = balloon {
            nid.uFlags |= NIF_INFO;
            copy_wide(&mut nid.szInfoTitle, b.title);
            copy_wide(&mut nid.szInfo, &b.text);
            nid.dwInfoFlags = if b.error { NIIF_ERROR } else { NIIF_INFO };
        }
        Shell_NotifyIconW(action, &nid);
        let old = ICON.swap(icon as isize, Ordering::Relaxed);
        if old != 0 {
            DestroyIcon(old as HICON);
        }
    }
}

unsafe fn remove_tray(hwnd: HWND) {
    unsafe {
        let mut nid: NOTIFYICONDATAW = zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

/// A round 32x32 icon; `ring` leaves the middle empty.
unsafe fn make_icon((r, g, b): (u8, u8, u8), ring: bool) -> HICON {
    const S: i32 = 32;
    unsafe {
        let mut bi: BITMAPINFO = zeroed();
        bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = S;
        bi.bmiHeader.biHeight = -S;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;
        let mut bits = null_mut();
        let color_bmp = CreateDIBSection(null_mut(), &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (S * S) as usize);
        let center = (S as f32 - 1.0) / 2.0;
        for y in 0..S {
            for x in 0..S {
                let (dx, dy) = (x as f32 - center, y as f32 - center);
                let d = (dx * dx + dy * dy).sqrt();
                let mut alpha = (14.5 - d).clamp(0.0, 1.0);
                if ring {
                    alpha *= (d - 8.5).clamp(0.0, 1.0);
                }
                let a = (alpha * 255.0) as u32;
                let pm = |v: u8| v as u32 * a / 255;
                px[(y * S + x) as usize] = (a << 24) | (pm(r) << 16) | (pm(g) << 8) | pm(b);
            }
        }
        let mask_bits = vec![0u8; (S * S / 8) as usize];
        let mask_bmp = CreateBitmap(S, S, 1, 1, mask_bits.as_ptr() as _);
        let ii = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bmp,
            hbmColor: color_bmp,
        };
        let icon = CreateIconIndirect(&ii);
        DeleteObject(color_bmp);
        DeleteObject(mask_bmp);
        icon
    }
}

// ---------------------------------------------------------------- actions

/// Opens a file or folder through the shell. `Command::spawn` is not used on purpose: after
/// `FreeConsole` the inherited std handles are dead and spawning fails with os error 50.
fn shell_open(file: &str, params: Option<&str>) -> Result<()> {
    let params = params.map(wide);
    let res = unsafe {
        ShellExecuteW(
            null_mut(),
            wide("open").as_ptr(),
            wide(file).as_ptr(),
            params.as_ref().map_or(null(), |p| p.as_ptr()),
            null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute reports success with a value above 32.
    if res as isize <= 32 {
        bail!(tr!("tray.open_failed", file = file));
    }
    Ok(())
}

fn open_in_notepad(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!(tr!("tray.no_file", path = path.display()));
    }
    shell_open("notepad.exe", Some(&format!("\"{}\"", path.display())))
}

fn open_backups() -> Result<()> {
    let dir = load_ctx().map_or_else(|| Config::default().backup_dir, |c| c.cfg.backup_dir);
    std::fs::create_dir_all(&dir)?;
    shell_open(&dir.to_string_lossy(), None)
}

/// Opens the config, creating it first (with the detected game path) if needed.
fn open_config() -> Result<()> {
    let Some((path, _)) = ui_paths() else { return Ok(()) };
    if !path.exists() {
        let cfg = Config {
            wow_path: wow::detect_root(),
            ..Config::default()
        };
        cfg.save(&path)?;
    }
    open_in_notepad(&path)
}

fn toggle_autostart() -> Result<()> {
    if schedule::logon_command().is_some() {
        schedule::remove_logon()?;
    } else {
        let explicit = ui_paths().and_then(|(_, e)| e);
        schedule::install_logon(explicit.as_deref(), false)?;
    }
    panel::refresh();
    settings::refresh();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn point(flavor: &str, hours_ago: i64) -> RestorePoint {
        let created = Local::now().naive_local() - Duration::hours(hours_ago);
        RestorePoint {
            flavor: flavor.into(),
            flavor_title: None,
            id: created.format("%Y%m%d-%H%M%S").to_string(),
            created,
            wtf_size: 1,
            addons_size: None,
            running: false,
            kind: None,
        }
    }

    #[test]
    fn rarely_backed_up_flavor_stays_in_the_list() {
        let retail: Vec<RestorePoint> = (0..10).map(|h| point("_retail_", h)).collect();
        let era = vec![point("_classic_era_", 100)];
        let picked = pick_points(&mut [retail, era]);
        let names: Vec<(&str, i64)> = picked
            .iter()
            .map(|p| (p.flavor.as_str(), (Local::now().naive_local() - p.created).num_hours()))
            .collect();
        assert_eq!(
            names,
            [("_retail_", 0), ("_retail_", 1), ("_retail_", 2), ("_retail_", 3), ("_classic_era_", 100)]
        );
    }
}
