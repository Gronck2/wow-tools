//! Addon updates through an addon manager the user already has: CurseForge, WowUp, WowUp-CF or
//! the Wago App (that one only WeakAuras and UI packs, see `wago.rs`).
//! Their APIs need per-developer keys, so instead of downloading ourselves the chosen app is
//! started without a window, left to install its updates, and closed again (only when we started
//! it). One app at a time: two managers over the same AddOns folder fight.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};
use sysinfo::{ProcessesToUpdate, System};

use crate::{curseforge, wago, wowup};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddonApp {
    #[serde(rename = "curseforge")]
    CurseForge,
    #[serde(rename = "wowup")]
    WowUp,
    #[serde(rename = "wowup_cf")]
    WowUpCf,
    #[serde(rename = "wago")]
    Wago,
}

impl AddonApp {
    pub const ALL: [AddonApp; 4] = [AddonApp::CurseForge, AddonApp::WowUp, AddonApp::WowUpCf, AddonApp::Wago];

    pub fn title(self) -> &'static str {
        match self {
            AddonApp::CurseForge => "CurseForge",
            AddonApp::WowUp => "WowUp",
            AddonApp::WowUpCf => "WowUp-CF",
            AddonApp::Wago => "Wago App",
        }
    }

    /// The installed app, if any.
    pub fn exe(self) -> Option<PathBuf> {
        match self {
            AddonApp::CurseForge => curseforge::exe(),
            AddonApp::WowUp => wowup::WowUp::FREE.exe_path(),
            AddonApp::WowUpCf => wowup::WowUp::CF.exe_path(),
            AddonApp::Wago => wago::exe(),
        }
    }

    pub fn installed() -> Vec<AddonApp> {
        AddonApp::ALL.into_iter().filter(|a| a.exe().is_some()).collect()
    }

    /// Starts the app hidden, waits for its updates, closes it. `wago`: what the Wago App updates
    /// (WeakAuras, UI packs).
    pub fn update(self, wago: (bool, bool)) -> Result<Outcome> {
        let exe = self.exe().with_context(|| tr!("addons.app_not_found", app = self.title()))?;
        match self {
            AddonApp::CurseForge => curseforge::update(&exe),
            AddonApp::WowUp => wowup::WowUp::FREE.update(&exe),
            AddonApp::WowUpCf => wowup::WowUp::CF.update(&exe),
            AddonApp::Wago => wago::update(&exe, wago.0, wago.1),
        }
    }

    /// What the app manages, for `status`: one line per WoW install it knows.
    pub fn describe(self) -> Result<Vec<String>> {
        match self {
            AddonApp::CurseForge => curseforge::describe(),
            AddonApp::WowUp => wowup::WowUp::FREE.describe(),
            AddonApp::WowUpCf => wowup::WowUp::CF.describe(),
            AddonApp::Wago => wago::describe(),
        }
    }
}

/// The app updates go through: the configured one while it is installed, else the first one
/// installed.
pub fn chosen(configured: Option<AddonApp>) -> Option<AddonApp> {
    pick(configured, &AddonApp::installed())
}

pub fn pick(configured: Option<AddonApp>, installed: &[AddonApp]) -> Option<AddonApp> {
    configured.filter(|a| installed.contains(a)).or_else(|| installed.first().copied())
}

#[derive(Default)]
pub struct Outcome {
    /// Names of the addons that got a new version.
    pub updated: Vec<String>,
    /// Addons still not on their newest version (auto-update off for them, or failed).
    pub outdated: Vec<String>,
    /// Something worth telling: the app was already open, the wait timed out, ...
    pub note: Option<String>,
    /// The same as `updated`, as data (versions, flavor).
    pub changes: Vec<Change>,
    /// Updates the app tried and did not finish (some other addons did get through).
    pub failed: Vec<Failed>,
    /// Addons left to the user ("update by hand" in the app).
    pub manual: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Addon,
    Weakauras,
    UiPacks,
}

/// One addon that got a new version. `from`/`to` are the manager's own strings; `flavor` is the
/// `_name_` folder when known.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub name: String,
    pub kind: ChangeKind,
    pub from: Option<String>,
    pub to: Option<String>,
    pub flavor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailReason {
    /// WowUp got no Wago token (its free Wago access needs the ad window).
    NoToken,
    Unknown,
}

/// An update the app tried and that did not end up installed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failed {
    pub name: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub flavor: Option<String>,
    pub reason: FailReason,
}

/// Why a run failed or was skipped: `errors::classify` without the text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodedError {
    pub code: String,
    pub params: serde_json::Value,
}

/// What a history row says as data. `Entry::details` is `None` for rows written before it existed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunDetails {
    pub updated: Vec<Change>,
    pub failed: Vec<Failed>,
    pub outdated: Vec<String>,
    pub by_hand: Vec<String>,
    pub error: Option<CodedError>,
}

impl RunDetails {
    /// For a run that ended in `e`; an all-failed run keeps its list.
    pub fn of_error(e: &anyhow::Error) -> RunDetails {
        let (code, params) = crate::errors::classify(e);
        RunDetails {
            failed: e.downcast_ref::<AddonsFailed>().map(|f| f.failed.clone()).unwrap_or_default(),
            error: Some(CodedError { code: code.into(), params }),
            ..Default::default()
        }
    }
}

impl From<&Outcome> for RunDetails {
    fn from(o: &Outcome) -> RunDetails {
        RunDetails {
            updated: o.changes.clone(),
            failed: o.failed.clone(),
            outdated: o.outdated.clone(),
            by_hand: o.manual.clone(),
            error: None,
        }
    }
}

/// Every update the app tried failed: an error with the list, so it is not lost with the text.
#[derive(Debug)]
pub struct AddonsFailed {
    pub failed: Vec<Failed>,
    pub text: String,
}

impl std::fmt::Display for AddonsFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::error::Error for AddonsFailed {}

/// The app could not run its update (it is open with auto-update off): nothing was checked. Not
/// counted as a run, so the schedule tries again; the history shows it as a warning, not an error.
#[derive(Debug)]
pub struct NotRun(pub String);

impl std::fmt::Display for NotRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotRun {}

impl Outcome {
    pub fn note(text: impl Into<String>) -> Outcome {
        Outcome {
            note: Some(text.into()),
            ..Default::default()
        }
    }

    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        match self.updated.len() {
            0 => lines.push(tr!("addons.none").to_string()),
            n => lines.push(tr!("addons.updated", n = n, names = self.updated.join(", "))),
        }
        if !self.outdated.is_empty() {
            lines.push(tr!("addons.left_outdated", names = self.outdated.join(", ")));
        }
        lines.extend(self.note.clone());
        lines.join("\n")
    }
}

// ---------------------------------------------------------------- processes

pub fn running(exe_name: &str) -> bool {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.processes().values().any(|p| p.name().eq_ignore_ascii_case(exe_name))
}

/// Runs a program without a window and without inheriting our (possibly dead) std handles.
pub fn quiet_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Keeps the top-level windows of a process hidden until dropped. Started "hidden", an app can
/// still show a window of its own above everything (WowUp's Wago ad consent page, since 2.2x).
pub struct HiddenWindows {
    #[cfg(windows)]
    thread: u32,
}

#[cfg(windows)]
pub fn hide_windows(pid: u32) -> HiddenWindows {
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    unsafe fn hide(hwnd: HWND) {
        if unsafe { GetAncestor(hwnd, GA_ROOT) } == hwnd && unsafe { IsWindowVisible(hwnd) } != 0 {
            unsafe { ShowWindowAsync(hwnd, SW_HIDE) };
        }
    }
    unsafe extern "system" fn on_show(_: HWINEVENTHOOK, _: u32, hwnd: HWND, object: i32, child: i32, _: u32, _: u32) {
        if object == OBJID_WINDOW && child == CHILDID_SELF as i32 {
            unsafe { hide(hwnd) };
        }
    }
    unsafe extern "system" fn hide_if_of(hwnd: HWND, pid: LPARAM) -> i32 {
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut owner) };
        if owner as LPARAM == pid {
            unsafe { hide(hwnd) };
        }
        1
    }

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || unsafe {
        let mut msg: MSG = std::mem::zeroed();
        // The message queue must exist before the thread id is handed out for WM_QUIT.
        PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE);
        let hook = SetWinEventHook(EVENT_OBJECT_SHOW, EVENT_OBJECT_SHOW, null_mut(), Some(on_show), pid, 0, WINEVENT_OUTOFCONTEXT);
        let _ = tx.send(GetCurrentThreadId());
        // Shown before the hook was set.
        EnumWindows(Some(hide_if_of), pid as LPARAM);
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            DispatchMessageW(&msg);
        }
        if !hook.is_null() {
            UnhookWinEvent(hook);
        }
    });
    HiddenWindows {
        thread: rx.recv().unwrap_or(0),
    }
}

#[cfg(not(windows))]
pub fn hide_windows(_pid: u32) -> HiddenWindows {
    HiddenWindows {}
}

#[cfg(windows)]
impl Drop for HiddenWindows {
    fn drop(&mut self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        unsafe { PostThreadMessageW(self.thread, WM_QUIT, 0, 0) };
    }
}

/// Closes an app: politely first, then for sure.
pub fn close(exe_name: &str) {
    close_within(exe_name, Duration::from_secs(15));
}

/// Like `close`, giving the app `grace` to quit by itself.
pub fn close_within(exe_name: &str, grace: Duration) {
    let _ = quiet_command("taskkill").args(["/IM", exe_name, "/T"]).status();
    let until = Instant::now() + grace;
    while running(exe_name) && Instant::now() < until {
        std::thread::sleep(Duration::from_secs(1));
    }
    if running(exe_name) {
        let _ = quiet_command("taskkill").args(["/IM", exe_name, "/T", "/F"]).status();
    }
}

pub fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Writes JSON the way Electron apps (`JSON.stringify(v, null, "\t")`) do, keeping key order
/// and numbers as they were (`preserve_order`, `arbitrary_precision`).
pub fn to_tab_json(value: &serde_json::Value) -> Result<String> {
    use serde::Serialize;
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(b"\t"));
    value.serialize(&mut ser)?;
    Ok(String::from_utf8(out)?)
}

// ---------------------------------------------------------------- last run

/// Where the time of the last update run is kept (the schedule catches up from it).
fn last_file() -> PathBuf {
    crate::config::data_dir().join("addon-update.last")
}

pub fn last_run() -> Option<chrono::NaiveDateTime> {
    let text = fs::read_to_string(last_file()).ok()?;
    chrono::NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%d %H:%M:%S").ok()
}

pub fn set_last_run(t: chrono::NaiveDateTime) {
    let _ = fs::create_dir_all(crate::config::data_dir());
    if let Err(e) = fs::write(last_file(), t.format("%Y-%m-%d %H:%M:%S").to_string()) {
        log::warn!("{}", tr!("addons.last_run_write_failed", error = e));
    }
}

// ---------------------------------------------------------------- history

/// How many past runs are kept.
const HISTORY_KEEP: usize = 20;

/// One update run, for the panel's history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub time: chrono::NaiveDateTime,
    /// The app's title ("CurseForge").
    pub app: String,
    pub ok: bool,
    /// The summary, or the error.
    pub text: String,
    /// Started from the menu or the panel, not by the schedule.
    pub manual: bool,
    /// Not run (`NotRun`): `ok` is false, shown as a warning.
    #[serde(default)]
    pub skipped: bool,
    /// The run as data; absent in rows written by older versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<RunDetails>,
}

fn history_file() -> PathBuf {
    crate::config::data_dir().join("addon-updates.json")
}

/// Past runs, newest first.
pub fn history() -> Vec<Entry> {
    read_history(&history_file())
}

fn read_history(path: &Path) -> Vec<Entry> {
    fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn record(entry: Entry) {
    if let Err(e) = append_history(&history_file(), entry) {
        log::warn!("{}", tr!("addons.history_write_failed", error = format!("{e:#}")));
    }
}

fn append_history(path: &Path, entry: Entry) -> Result<()> {
    let mut list = read_history(path);
    list.insert(0, entry);
    list.truncate(HISTORY_KEEP);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, serde_json::to_string_pretty(&list)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window of this process, off screen and never activated, is hidden as soon as it shows.
    #[cfg(windows)]
    #[test]
    fn hides_windows_of_the_process() {
        use std::ptr::null_mut;
        use std::sync::mpsc;
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::WindowsAndMessaging::*;

        let _hidden = hide_windows(std::process::id());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || unsafe {
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                null_mut(),
                WS_POPUP,
                -32000,
                -32000,
                10,
                10,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            );
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = tx.send(hwnd as usize);
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                DispatchMessageW(&msg);
            }
        });
        let hwnd = rx.recv().unwrap() as HWND;
        let until = Instant::now() + Duration::from_secs(3);
        while unsafe { IsWindowVisible(hwnd) } != 0 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(unsafe { IsWindowVisible(hwnd) }, 0);
        unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
    }

    #[test]
    fn history_keeps_the_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("h.json");
        let at = |m| chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap().and_hms_opt(10, m, 0).unwrap();
        for m in 0..25 {
            let entry = Entry {
                time: at(m),
                app: "WowUp".into(),
                ok: m % 2 == 0,
                text: format!("run {m}"),
                manual: false,
                skipped: false,
                details: None,
            };
            append_history(&path, entry).unwrap();
        }
        let list = read_history(&path);
        assert_eq!(list.len(), HISTORY_KEEP);
        assert_eq!(list[0].text, "run 24");
        assert_eq!(list[0].time, at(24));
    }

    #[test]
    fn old_rows_load_without_details_and_are_written_back_without_them() {
        // The exact shapes older versions wrote: with and without `skipped`.
        let old = r#"[{"time":"2026-10-12T10:00:00","app":"WowUp","ok":true,"text":"t","manual":false,"skipped":false},
                       {"time":"2026-10-11T10:00:00","app":"CurseForge","ok":true,"text":"u","manual":true}]"#;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("h.json");
        fs::write(&path, old).unwrap();
        let list = read_history(&path);
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|e| e.details.is_none()));
        assert!(!serde_json::to_string(&list).unwrap().contains("details"));
        // A row with details survives a round trip, and so does one with a partial object.
        let mut e = list[0].clone();
        e.details = Some(RunDetails {
            failed: vec![Failed {
                name: "BugSack".into(),
                from: Some("1".into()),
                to: None,
                flavor: None,
                reason: FailReason::NoToken,
            }],
            ..Default::default()
        });
        assert_eq!(serde_json::from_str::<Entry>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
        let partial: Entry =
            serde_json::from_str(r#"{"time":"2026-10-12T10:00:00","app":"A","ok":true,"text":"","manual":false,"details":{"by_hand":["X"]}}"#).unwrap();
        assert_eq!(partial.details.unwrap().by_hand, ["X"]);
    }

    #[test]
    fn an_error_becomes_details_and_keeps_the_failed_list() {
        let failed = Failed {
            name: "BugSack".into(),
            from: None,
            to: None,
            flavor: None,
            reason: FailReason::Unknown,
        };
        let e = anyhow::Error::new(AddonsFailed {
            failed: vec![failed.clone()],
            text: "all failed".into(),
        });
        let d = RunDetails::of_error(&e);
        assert_eq!(d.failed, [failed]);
        assert_eq!(d.error.unwrap().code, "failed");
        let d = RunDetails::of_error(&NotRun("open".into()).into());
        assert!(d.failed.is_empty());
        assert_eq!(d.error.unwrap().code, "not_run");
    }

    #[test]
    fn an_outcome_becomes_details_and_old_rows_rewrite_byte_for_byte() {
        let o = Outcome {
            outdated: vec!["Plater".into()],
            manual: vec!["WeakAuras".into()],
            ..Default::default()
        };
        let d = RunDetails::from(&o);
        assert_eq!(
            (d.outdated, d.by_hand, d.error),
            (vec!["Plater".to_string()], vec!["WeakAuras".to_string()], None)
        );
        // A file as an older exe wrote it (pretty JSON, `skipped` present): reading and writing it again changes nothing.
        let old = "[
  {
    \"time\": \"2026-10-12T10:00:00\",
    \"app\": \"WowUp\",
    \"ok\": true,
    \"text\": \"t\",
    \"manual\": false,
    \"skipped\": false
  }
]";
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("h.json");
        fs::write(&path, old).unwrap();
        assert_eq!(serde_json::to_string_pretty(&read_history(&path)).unwrap(), old);
    }

    #[test]
    fn summary_text() {
        let o = Outcome {
            updated: vec!["BigWigs".into(), "Details!".into()],
            ..Default::default()
        };
        assert_eq!(o.summary(), "Обновлено 2: BigWigs, Details!");
    }

    #[test]
    fn app_names_in_config() {
        #[derive(Deserialize)]
        struct T {
            a: Vec<AddonApp>,
        }
        let t: T = toml::from_str("a = ['curseforge', 'wowup', 'wowup_cf', 'wago']").unwrap();
        assert_eq!(t.a, AddonApp::ALL);
    }

    #[test]
    fn tab_json_matches_electron() {
        let text = "{\n\t\"b\": {\n\t\t\"x\": 1.50,\n\t\t\"list\": [],\n\t\t\"o\": {}\n\t},\n\t\"a\": [\n\t\t\"п\"\n\t]\n}";
        let v: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(to_tab_json(&v).unwrap(), text);
    }
}
