//! Tray icon: runs the `watch` loop on a worker thread; a left click opens the panel
//! (`panel.rs`), a right click the menu (`menu.rs`). The icon is drawn at runtime and shows
//! the state: green = ok, ring = game running (backup after exit), blue = working,
//! red = error, gray = no snapshots yet.

use crate::app::Ctx;
use crate::config::{self, APP_NAME, Config};
use crate::panel::{self, View};
use crate::profiles::{self, CharEntry, Profile};
use crate::snapshot::{self, Kind};
use crate::watch::{self, Event, Request, Work};
use crate::{maintenance, menu, schedule, selfupdate, settings, wow};
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
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, ReleaseDC,
};
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
/// A new exe was installed: start it and exit (once nothing is running).
const WM_RESTART: u32 = WM_APP + 4;
/// Read the cleanup tab again (posted by the worker thread; the config paths live on the UI thread).
const WM_RESCAN: u32 = WM_APP + 5;
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
        } else if self.id.ends_with("-pre-patch") {
            Some(tr!("points.before_patch"))
        } else if self.id.ends_with("-pre-cleanup") {
            Some(tr!("points.before_cleanup"))
        } else if self.id.ends_with("-pre-profile") {
            Some(tr!("points.before_profile"))
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
    /// Ask GitHub for a newer WoW Tools now.
    CheckUpdate,
    /// Download and install the newer WoW Tools, then restart.
    InstallUpdate,
    /// Tidy one thing up (settings of removed addons, old combat logs, ...).
    Clean(maintenance::Item),
    /// The folder screenshots are moved to.
    OpenScreenshots,
    SaveProfile(CharEntry),
    ApplyProfile {
        profile: Profile,
        target: CharEntry,
    },
    DeleteProfile(Profile),
    Exit,
}

/// Updates of WoW Tools itself, as the panel, the menu and the settings show them.
#[derive(Clone, Debug, Default)]
pub struct SelfUpdate {
    /// A newer release than this exe.
    pub available: Option<selfupdate::Release>,
    pub checked: Option<NaiveDateTime>,
    pub checking: bool,
    pub installing: bool,
    /// Why the last check failed.
    pub error: Option<String>,
}

enum SelfRequest {
    Check { manual: bool },
    Install,
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
    update: SelfUpdate,
    /// A new exe is in place; restart when the current job is done.
    restart_pending: bool,
    /// What the cleanup tab shows; read in the background (`rescan`).
    scan: Option<maintenance::Scan>,
    scanning: bool,
    /// The cleanup running now.
    cleaning: Option<maintenance::Item>,
    /// How the last cleanup went, shown in its row of the panel: (item, summary, ok).
    cleaned: Option<(maintenance::Item, String, bool)>,
    /// Characters to pick for profiles, last played first; read with the cleanup scan.
    chars: Option<Vec<CharEntry>>,
    /// The profile job running now, and how the last one went (shown in its row).
    profile_job: Option<ProfileJob>,
    profile_done: Option<(ProfileJob, String, Tone)>,
    /// The saved profiles (`saved_profiles`).
    profiles: Option<Vec<Profile>>,
}

/// How a job ended, as its row shows it: done (green), nothing done but worth a look (warning), failed (red).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tone {
    Ok,
    Notice,
    Error,
}

/// What a profile job is about: where the panel shows its spinner and result.
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileJob {
    Save,
    Apply(PathBuf),
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
    update: SelfUpdate {
        available: None,
        checked: None,
        checking: false,
        installing: false,
        error: None,
    },
    restart_pending: false,
    scan: None,
    scanning: false,
    cleaning: None,
    cleaned: None,
    chars: None,
    profile_job: None,
    profile_done: None,
    profiles: None,
});
static HWND_MAIN: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static ICON: AtomicIsize = AtomicIsize::new(0);

/// UI-thread-only data.
struct Ui {
    requests: Sender<Request>,
    self_requests: Sender<SelfRequest>,
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

    selfupdate::cleanup();
    {
        let mut st = STATE.lock().unwrap();
        st.status.last = ctx.last_backup();
        st.update.available = selfupdate::available();
        st.update.checked = selfupdate::last_check();
        if let Some(version) = selfupdate::take_installed() {
            st.balloon = Some(Balloon {
                title: tr!("self_update.done_title"),
                text: tr!("self_update.done_text", version = version),
                error: false,
            });
        }
    }
    let (tx, rx) = mpsc::channel();
    let (config_path, explicit_config) = (ctx.config_path.clone(), ctx.explicit_config.clone());
    std::thread::spawn(move || watch::run(&config_path, explicit_config.as_deref(), None, &rx, on_event));
    let (self_tx, self_rx) = mpsc::channel();
    let config_path = ctx.config_path.clone();
    std::thread::spawn(move || self_update_loop(&config_path, &self_rx));
    UI.with(|ui| {
        *ui.borrow_mut() = Some(Ui {
            requests: tx,
            self_requests: self_tx,
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

/// The app icon, for the title bar and taskbar button of windows. Loaded once, never freed.
pub fn app_icon() -> HICON {
    static APP_ICON: AtomicIsize = AtomicIsize::new(0);
    let mut icon = APP_ICON.load(Ordering::Relaxed);
    if icon == 0 {
        icon = load_icon(ICON_APP, unsafe { GetSystemMetrics(SM_CXICON) }) as isize;
        APP_ICON.store(icon, Ordering::Relaxed);
    }
    icon as HICON
}

/// Icons in the exe (`wow-tools.rc`): the app (gold, also the tray while a game runs) and the
/// tray while none does (grey).
const ICON_APP: u16 = 1;
const ICON_IDLE: u16 = 2;

/// Icon `id` of the exe at `size` pixels (the closest of the sizes in the .ico, scaled).
fn load_icon(id: u16, size: i32) -> HICON {
    unsafe { LoadImageW(GetModuleHandleW(null()), id as usize as *const u16, IMAGE_ICON, size, size, LR_DEFAULTCOLOR) as HICON }
}

fn load_ctx() -> Option<Ctx> {
    let (path, explicit) = ui_paths()?;
    Ctx::load(&path, explicit.as_deref()).ok()
}

fn restore_points(ctx: &Ctx, flavors: &[wow::Flavor], games: &[wow::Running]) -> Vec<RestorePoint> {
    let store = ctx.store();
    let multi = flavors.len() > 1;
    let mut per_flavor: Vec<Vec<RestorePoint>> = flavors
        .iter()
        .map(|fl| {
            let wtf = store.list(fl, Kind::Wtf).unwrap_or_default();
            let addons = store.list(fl, Kind::Addons).unwrap_or_default();
            let running = wow::running_in(games, fl).is_some();
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
            // Backups and the state before a cleanup or a profile; the safety nets before a copy
            // (`app::is_safety`) and the addons saved before an update are not listed.
            let mut list: Vec<RestorePoint> = wtf
                .iter()
                .filter(|s| !crate::app::is_safety(&s.id) || s.id.ends_with("-pre-cleanup") || s.id.ends_with("-pre-profile"))
                .map(|s| point(&s.id, s.created, s.size, snapshot::select(&addons, Some(&s.id)).map(|a| a.size), None))
                .collect();
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
    let flavors = ctx.as_ref().and_then(|c| c.flavors(None).ok()).unwrap_or_default();
    let games = wow::running_games();
    let update_text = if cfg.update_addons {
        upper_first(&cfg.update_text())
    } else {
        tr!("panel.auto_update_off").into()
    };
    let st = STATE.lock().unwrap();
    let (status, self_update, scan, cleaning, cleaned) = (st.status.clone(), st.update.clone(), st.scan.clone(), st.cleaning, st.cleaned.clone());
    let (chars, profile_job, profile_done) = (st.chars.clone(), st.profile_job.clone(), st.profile_done.clone());
    drop(st);
    View {
        self_update,
        update_waits_for_game: cfg.update_skip_in_game && status.game.is_some(),
        status,
        points: ctx.as_ref().map(|c| restore_points(c, &flavors, &games)).unwrap_or_default(),
        autostart: schedule::logon_command().is_some(),
        schedule: cfg.schedule_text(),
        update_app: crate::updater::chosen(cfg.update_app).map(|a| a.title()),
        update_text,
        last_update: crate::updater::last_run(),
        updates: crate::updater::history().into_iter().take(panel::HISTORY_ROWS).collect(),
        scan,
        cleaning,
        cleaned,
        profiles: ctx.as_ref().map(saved_profiles).unwrap_or_default(),
        chars,
        profile_job,
        profile_done,
        shots_dir: maintenance::screenshots_dir(&cfg),
        running: flavors
            .iter()
            .filter(|fl| wow::running_in(&games, fl).is_some())
            .map(|fl| fl.name.clone())
            .collect(),
    }
}

/// The saved profiles, read once and kept until a profile job, a delete or a settings change
/// (`forget_profiles`): `view` runs on every refresh and each profile is a zip to open.
fn saved_profiles(ctx: &Ctx) -> Vec<Profile> {
    let mut st = STATE.lock().unwrap();
    st.profiles.get_or_insert_with(|| profiles::list(&profiles::root(ctx))).clone()
}

fn forget_profiles() {
    STATE.lock().unwrap().profiles = None;
}

/// Reads what the cleanup tab shows again, on a thread of its own (it takes a moment on a big WTF).
/// UI thread only.
pub fn rescan() {
    {
        let mut st = STATE.lock().unwrap();
        if st.scanning {
            return;
        }
        st.scanning = true;
    }
    let Some(ctx) = load_ctx() else {
        STATE.lock().unwrap().scanning = false;
        return;
    };
    std::thread::spawn(move || {
        let flavors = ctx.flavors(None).unwrap_or_default();
        let scan = maintenance::scan(&flavors);
        let chars = profiles::characters(&flavors);
        {
            let mut st = STATE.lock().unwrap();
            st.scan = Some(scan);
            st.chars = Some(chars);
            st.scanning = false;
        }
        post_state();
    });
}

/// Drops how the last cleanup and profile job went (the panel shows it until it closes).
pub fn forget_results() {
    let mut st = STATE.lock().unwrap();
    st.cleaned = None;
    st.profile_done = None;
}

fn start_profile_job(job: ProfileJob) {
    let mut st = STATE.lock().unwrap();
    st.profile_job = Some(job);
    st.profile_done = None;
}

/// Updates of WoW Tools itself, for the settings window.
pub fn self_update() -> SelfUpdate {
    STATE.lock().unwrap().update.clone()
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
            // The backup folder, and the profiles in it, may have moved.
            forget_profiles();
            post_state();
            Ok(())
        }
        Action::OpenLog => open_in_notepad(&config::default_log_path()),
        Action::Autostart => toggle_autostart(),
        Action::CheckUpdate => send_self(SelfRequest::Check { manual: true }),
        Action::InstallUpdate => send_self(SelfRequest::Install),
        Action::Clean(item) => {
            {
                let mut st = STATE.lock().unwrap();
                st.cleaning = Some(item);
                st.cleaned = None;
            }
            send(Request::Cleanup(item))
        }
        Action::SaveProfile(entry) => {
            start_profile_job(ProfileJob::Save);
            send(Request::SaveProfile {
                flavor: entry.flavor,
                character: entry.character,
            })
        }
        Action::ApplyProfile { profile, target } => {
            start_profile_job(ProfileJob::Apply(profile.path.clone()));
            send(Request::ApplyProfile {
                profile,
                target: target.character,
            })
        }
        Action::DeleteProfile(profile) => {
            let result = profiles::delete(&profile);
            forget_profiles();
            post_state();
            result
        }
        Action::OpenScreenshots => {
            let dir = maintenance::screenshots_dir(&load_ctx().map_or_else(Config::default, |c| c.cfg));
            std::fs::create_dir_all(&dir)
                .map_err(Into::into)
                .and_then(|()| shell_open(&dir.to_string_lossy(), None))
        }
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

fn send_self(request: SelfRequest) -> Result<()> {
    UI.with(|ui| match ui.borrow().as_ref() {
        Some(ui) => ui.self_requests.send(request).map_err(|_| anyhow::anyhow!(tr!("tray.worker_gone"))),
        None => Ok(()),
    })
}

fn post_state() {
    post(WM_STATE);
}

fn post(msg: u32) {
    unsafe {
        PostMessageW(HWND_MAIN.load(Ordering::Relaxed) as HWND, msg, 0, 0);
    }
}

// ---------------------------------------------------------------- updates of WoW Tools

/// The thread asking GitHub for a newer WoW Tools: a minute after start, then hourly whether a
/// daily check is due (`check_updates`), and whenever the UI asks.
fn self_update_loop(config_path: &Path, requests: &mpsc::Receiver<SelfRequest>) {
    let mut wait = std::time::Duration::from_secs(60);
    loop {
        let request = match requests.recv_timeout(wait) {
            Ok(r) => Some(r),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        wait = std::time::Duration::from_secs(60 * 60);
        match request {
            None => {
                let on = Config::load(config_path).map_or(true, |c| c.check_updates);
                if on && selfupdate::due(Local::now().naive_local()) {
                    check_for_update(false);
                }
            }
            Some(SelfRequest::Check { manual }) => check_for_update(manual),
            Some(SelfRequest::Install) => install_update(),
        }
    }
}

fn check_for_update(manual: bool) {
    STATE.lock().unwrap().update.checking = true;
    post_state();
    let result = selfupdate::check();
    {
        let mut st = STATE.lock().unwrap();
        st.update.checking = false;
        st.update.checked = selfupdate::last_check();
        match result {
            Ok(found) => {
                st.update.error = None;
                match &found {
                    // A balloon once per version (and on every manual check).
                    Some(r) if selfupdate::take_notify(r) || manual => {
                        st.balloon = Some(Balloon {
                            title: tr!("self_update.found_title"),
                            text: tr!("self_update.found_text", version = &r.version),
                            error: false,
                        })
                    }
                    Some(_) => {}
                    None if manual => {
                        st.balloon = Some(Balloon {
                            title: APP_NAME,
                            text: tr!("self_update.up_to_date", version = selfupdate::current()),
                            error: false,
                        })
                    }
                    None => {}
                }
                st.update.available = found;
            }
            Err(e) => {
                let msg = format!("{e:#}");
                log::warn!("{}: {msg}", tr!("self_update.check_failed"));
                if manual {
                    st.balloon = Some(Balloon {
                        title: tr!("self_update.check_failed"),
                        text: msg.clone(),
                        error: true,
                    });
                }
                st.update.error = Some(msg);
            }
        }
    }
    post_state();
}

fn install_update() {
    let release = {
        let mut st = STATE.lock().unwrap();
        let Some(r) = st.update.available.clone() else { return };
        if st.update.installing {
            return;
        }
        st.update.installing = true;
        r
    };
    post_state();
    let result = selfupdate::download(&release).and_then(|new| selfupdate::install(&new, &selfupdate::exe_path()?));
    match result {
        Ok(()) => {
            selfupdate::mark_installed(&release.version);
            log::info!("{}", tr!("self_update.installed", version = &release.version));
            post(WM_RESTART);
        }
        Err(e) => {
            log::error!("{e:#}");
            let mut st = STATE.lock().unwrap();
            st.update.installing = false;
            st.balloon = Some(Balloon {
                title: tr!("self_update.failed"),
                text: format!("{e:#}"),
                error: true,
            });
            drop(st);
            post_state();
        }
    }
}

/// Starts the new exe (it waits for this process to exit) and exits; while a job runs, waits for it.
fn restart() {
    {
        let mut st = STATE.lock().unwrap();
        if st.status.phase == Phase::Busy {
            if !st.restart_pending {
                st.restart_pending = true;
                st.balloon = Some(Balloon {
                    title: tr!("self_update.done_title"),
                    text: tr!("self_update.wait_busy").into(),
                    error: false,
                });
                drop(st);
                post_state();
            }
            return;
        }
        st.restart_pending = false;
    }
    let started = selfupdate::exe_path().and_then(|exe| {
        let mut params = format!("--headless \"{}\" --log-to-file", exe.display());
        if let Some((_, Some(cfg))) = ui_paths() {
            params.push_str(&format!(" --config \"{}\"", cfg.display()));
        }
        params.push_str(&format!(" tray --wait-pid {}", std::process::id()));
        shell_run("conhost.exe", &params, SW_HIDE)
    });
    match started {
        Ok(()) => unsafe {
            DestroyWindow(HWND_MAIN.load(Ordering::Relaxed) as HWND);
        },
        Err(e) => {
            log::error!("{e:#}");
            let mut st = STATE.lock().unwrap();
            st.update.installing = false;
            st.balloon = Some(Balloon {
                title: tr!("self_update.failed"),
                text: format!("{e:#}"),
                error: true,
            });
            drop(st);
            post_state();
        }
    }
}

// ---------------------------------------------------------------- worker events

fn on_event(event: Event) {
    let mut rescan_after = false;
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
            Event::Patch(patches) => {
                let text: Vec<String> = patches.iter().map(|p| p.summary()).collect();
                st.balloon = Some(Balloon {
                    title: tr!("patch.title"),
                    text: text.join("\n"),
                    error: false,
                });
            }
            Event::Profile { result, notice } => {
                st.profiles = None;
                st.status.phase = Phase::Idle;
                st.profile_done = st.profile_job.take().map(|job| match &result {
                    Ok(summary) => (job, summary.clone(), if notice { Tone::Notice } else { Tone::Ok }),
                    Err(e) => (job, format!("{e:#}"), Tone::Error),
                });
                st.balloon = Some(match result {
                    Ok(summary) => Balloon {
                        title: tr!("profiles.title"),
                        text: summary,
                        error: false,
                    },
                    Err(e) => Balloon {
                        title: tr!("profiles.failed"),
                        text: format!("{e:#}"),
                        error: true,
                    },
                });
            }
            Event::Cleanup(result) => {
                st.status.phase = Phase::Idle;
                rescan_after = true;
                st.cleaned = st.cleaning.take().map(|item| match &result {
                    Ok(summary) => (item, summary.clone(), true),
                    Err(e) => (item, format!("{e:#}"), false),
                });
                st.balloon = Some(match result {
                    Ok(summary) => Balloon {
                        title: tr!("cleanup.title"),
                        text: summary,
                        error: false,
                    },
                    Err(e) => Balloon {
                        title: tr!("cleanup.failed"),
                        text: format!("{e:#}"),
                        error: true,
                    },
                });
            }
        }
        // An installed update waited for this job.
        if st.restart_pending && st.status.phase != Phase::Busy {
            post(WM_RESTART);
        }
    }
    // The cleanup tab shows what is left.
    if rescan_after {
        post(WM_RESCAN);
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
            WM_RESTART => {
                restart();
                0
            }
            WM_RESCAN => {
                rescan();
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
    let (id, badge, tip, balloon) = {
        let mut st = STATE.lock().unwrap();
        let s = &st.status;
        // Gold while the game runs, grey otherwise; a dot for work (blue) or a failure (red).
        let id = if s.game.is_some() { ICON_APP } else { ICON_IDLE };
        let badge = match s.phase {
            Phase::Busy => Some((0x3B, 0x82, 0xF6)),
            Phase::Error(_) => Some((0xE5, 0x48, 0x4D)),
            Phase::Idle => None,
        };
        let tip = tooltip(s);
        (id, badge, tip, st.balloon.take())
    };
    // Read only when there is something to show; unreadable = shown.
    let balloon = balloon.filter(|_| config_path().and_then(|p| Config::load(&p).ok()).is_none_or(|c| c.notifications));
    unsafe {
        let icon = tray_icon(id, badge, GetSystemMetrics(SM_CXSMICON));
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

/// Tray icon `id` at the small icon size, with a dot of color `badge` in the lower right corner.
unsafe fn tray_icon(id: u16, badge: Option<(u8, u8, u8)>, s: i32) -> HICON {
    unsafe {
        let base = load_icon(id, s);
        let Some((r, g, b)) = badge else { return base };
        // The icon's pixels (32-bit with alpha), top-down.
        let mut ii: ICONINFO = zeroed();
        GetIconInfo(base, &mut ii);
        let mut bi: BITMAPINFO = zeroed();
        bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = s;
        bi.bmiHeader.biHeight = -s;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;
        let mut bits = null_mut();
        let color_bmp = CreateDIBSection(null_mut(), &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (s * s) as usize);
        let dc = GetDC(null_mut());
        GetDIBits(dc, ii.hbmColor, 0, s as u32, bits, &mut bi, DIB_RGB_COLORS);
        ReleaseDC(null_mut(), dc);
        DeleteObject(ii.hbmColor);
        DeleteObject(ii.hbmMask);
        DestroyIcon(base);
        // A dark rim, then the dot, each blended over what is there.
        let radius = s as f32 * 0.24;
        let (cx, cy) = (s as f32 - radius - 1.0, s as f32 - radius - 1.0);
        for y in 0..s {
            for x in 0..s {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let p = &mut px[(y * s + x) as usize];
                for (rad, (cr, cg, cb)) in [(radius + 1.2, (0x20, 0x20, 0x20)), (radius, (r, g, b))] {
                    let a = (rad - d + 0.5).clamp(0.0, 1.0);
                    if a > 0.0 {
                        *p = over(*p, (cr, cg, cb), a);
                    }
                }
            }
        }
        // Rows of a monochrome bitmap are padded to 16 bits.
        let mask_bits = vec![0u8; ((s + 15) / 16 * 2 * s) as usize];
        let mask_bmp = CreateBitmap(s, s, 1, 1, mask_bits.as_ptr() as _);
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

/// The tray icon at `size` px: no game, game running, busy, failed (for `ui-snapshot`).
pub fn demo_icons(size: i32) -> [HICON; 4] {
    let (busy, error) = (Some((0x3B, 0x82, 0xF6)), Some((0xE5, 0x48, 0x4D)));
    unsafe {
        [
            tray_icon(ICON_IDLE, None, size),
            tray_icon(ICON_APP, None, size),
            tray_icon(ICON_IDLE, busy, size),
            tray_icon(ICON_APP, error, size),
        ]
    }
}

/// `(r, g, b)` at coverage `a` over the ARGB pixel `p`.
fn over(p: u32, (r, g, b): (u8, u8, u8), a: f32) -> u32 {
    let mix = |top: u8, shift: u32| ((top as f32 * a + ((p >> shift) & 0xFF) as f32 * (1.0 - a)).round() as u32) << shift;
    let alpha = ((a * 255.0 + ((p >> 24) & 0xFF) as f32 * (1.0 - a)).round() as u32).min(255) << 24;
    alpha | mix(r, 16) | mix(g, 8) | mix(b, 0)
}

// ---------------------------------------------------------------- actions

/// Opens a file or folder through the shell. `Command::spawn` is not used on purpose: after
/// `FreeConsole` the inherited std handles are dead and spawning fails with os error 50.
fn shell_open(file: &str, params: Option<&str>) -> Result<()> {
    shell_execute(file, params, SW_SHOWNORMAL)
}

/// Starts a program with no window of its own (`conhost --headless ...`).
fn shell_run(file: &str, params: &str, show: SHOW_WINDOW_CMD) -> Result<()> {
    shell_execute(file, Some(params), show)
}

fn shell_execute(file: &str, params: Option<&str>, show: SHOW_WINDOW_CMD) -> Result<()> {
    let params = params.map(wide);
    let res = unsafe {
        ShellExecuteW(
            null_mut(),
            wide("open").as_ptr(),
            wide(file).as_ptr(),
            params.as_ref().map_or(null(), |p| p.as_ptr()),
            null(),
            show,
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
