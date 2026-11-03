//! "Copy diagnostics" (Settings → General): what a user sends when something does not work, addon
//! updates above all. Plain text in English (it is read by the developer, not shown in the UI):
//! version, Windows build, the game, the addon managers, the update settings and history, the
//! config file and the end of the log. The user profile folder is replaced with `%USERPROFILE%`.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::config::{self, Config};
use crate::updater::{self, AddonApp};
use crate::{schedule, wow};

/// How much of the end of the log goes in.
const LOG_TAIL_BYTES: u64 = 48 * 1024;

pub fn report(config_path: &Path) -> String {
    let mut out = String::new();
    let mut line = |s: String| {
        out.push_str(&s);
        out.push('\n');
    };
    let cfg = Config::load(config_path);
    let mode = if config::portable_dir().is_some() { "portable" } else { "installed" };
    line(format!("WoW Tools {} ({mode}), Windows {}", env!("CARGO_PKG_VERSION"), windows_version()));
    line(format!("Time: {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S")));
    line(format!("Language: {:?}", crate::i18n::current()));
    line(format!("Autostart: {}", schedule::logon_command().is_some()));

    line(String::new());
    line("== Game".into());
    let cfg = match cfg {
        Ok(cfg) => cfg,
        Err(e) => {
            line(format!("config error: {e:#}"));
            Config::default()
        }
    };
    let root = cfg.wow_path.clone().or_else(wow::detect_root);
    line(format!(
        "Folder: {} ({})",
        show(root.as_deref()),
        if cfg.wow_path.is_some() { "set" } else { "auto" }
    ));
    for fl in wow::all_flavors(root.as_deref()) {
        let on = cfg.flavors.is_empty() || cfg.flavors.contains(&fl.name);
        let version = fl.version.as_deref().unwrap_or("?");
        line(format!(
            "  {} {} {version} [{}] {}",
            if on { "+" } else { "-" },
            fl.name,
            fl.title(),
            fl.dir.display()
        ));
    }
    let games: Vec<String> = wow::running_games().into_iter().map(|g| g.name).collect();
    line(format!("Running: {}", if games.is_empty() { "-".into() } else { games.join(", ") }));

    line(String::new());
    line("== Addon updates".into());
    line(format!(
        "Auto: {}, {}; skip in game: {}; ElvUI: {}",
        cfg.update_addons,
        cfg.update_text(),
        cfg.update_skip_in_game,
        cfg.update_elvui
    ));
    let last = updater::last_run().map_or("-".into(), |t| t.to_string());
    line(format!("Last run: {last}"));
    let installed = AddonApp::installed();
    let chosen = updater::pick(cfg.update_app, &installed);
    line(format!("Chosen: {} (configured: {:?})", chosen.map_or("-", |a| a.title()), cfg.update_app));
    for app in installed {
        let exe = app.exe();
        let running = exe
            .as_deref()
            .and_then(|p| p.file_name())
            .is_some_and(|n| updater::running(&n.to_string_lossy()));
        line(format!("  {} {}{}", app.title(), show(exe.as_deref()), if running { " (running)" } else { "" }));
        match app.describe() {
            Ok(lines) => lines.into_iter().for_each(|l| line(format!("    {l}"))),
            Err(e) => line(format!("    {e:#}")),
        }
    }
    line("History:".into());
    for e in updater::history().iter().take(10) {
        let how = if e.manual { "manual" } else { "auto" };
        let ok = if e.ok { "ok" } else { "FAILED" };
        line(format!(
            "  {} {} {how} {ok}: {}",
            e.time.format("%Y-%m-%d %H:%M"),
            e.app,
            e.text.replace('\n', " | ")
        ));
    }

    line(String::new());
    line(format!("== {}", config_path.display()));
    line(fs::read_to_string(config_path).unwrap_or_else(|e| format!("({e})")).trim_end().to_string());

    let log = config::default_log_path();
    line(String::new());
    line(format!("== {} (end)", log.display()));
    line(log_tail(&log).unwrap_or_else(|e| format!("({e})")).trim_end().to_string());

    hide_home(&out)
}

/// The last whole lines within `LOG_TAIL_BYTES` of the log.
fn log_tail(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    let from = len.saturating_sub(LOG_TAIL_BYTES);
    file.seek(SeekFrom::Start(from))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    // Cut a line that starts before the tail.
    Ok(match (from > 0, text.find('\n')) {
        (true, Some(i)) => text[i + 1..].to_string(),
        _ => text.into_owned(),
    })
}

fn show(path: Option<&Path>) -> String {
    path.map_or("-".into(), |p| p.display().to_string())
}

/// The user folder (it holds the Windows user name) as `%USERPROFILE%`.
fn hide_home(text: &str) -> String {
    match std::env::var("USERPROFILE") {
        Ok(home) if home.len() > 3 => replace_ignore_case(text, &home, "%USERPROFILE%"),
        _ => text.to_string(),
    }
}

fn replace_ignore_case(text: &str, what: &str, with: &str) -> String {
    let lower = text.to_lowercase();
    let what = what.to_lowercase();
    // `to_lowercase` can change byte lengths (rare letters); then the indexes would not match.
    if lower.len() != text.len() {
        return text.replace(&what, with);
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(i) = lower[at..].find(&what) {
        out.push_str(&text[at..at + i]);
        out.push_str(with);
        at += i + what.len();
    }
    out.push_str(&text[at..]);
    out
}

#[cfg(windows)]
fn windows_version() -> String {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;
    let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion") else {
        return "?".into();
    };
    let build: String = key.get_value("CurrentBuild").unwrap_or_default();
    let display: String = key.get_value("DisplayVersion").unwrap_or_default();
    format!("{display} build {build}")
}

#[cfg(not(windows))]
fn windows_version() -> String {
    "-".into()
}

/// Puts `text` on the clipboard.
#[cfg(windows)]
pub fn copy_to_clipboard(hwnd: windows_sys::Win32::Foundation::HWND, text: &str) -> anyhow::Result<()> {
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            anyhow::bail!(std::io::Error::last_os_error());
        }
        let result = (|| {
            EmptyClipboard();
            let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
            if mem.is_null() {
                anyhow::bail!(std::io::Error::last_os_error());
            }
            let dst = GlobalLock(mem) as *mut u16;
            if dst.is_null() {
                GlobalFree(mem);
                anyhow::bail!(std::io::Error::last_os_error());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
            GlobalUnlock(mem);
            // The clipboard owns the memory once this succeeds.
            if SetClipboardData(CF_UNICODETEXT, mem).is_null() {
                GlobalFree(mem);
                anyhow::bail!(std::io::Error::last_os_error());
            }
            Ok(())
        })();
        CloseClipboard();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_folder_is_hidden() {
        let text = r"backup_dir = 'C:\Users\Bob\Documents\x' and c:\users\bob\AppData";
        assert_eq!(
            replace_ignore_case(text, r"C:\Users\Bob", "%USERPROFILE%"),
            r"backup_dir = '%USERPROFILE%\Documents\x' and %USERPROFILE%\AppData"
        );
    }

    #[test]
    fn log_tail_starts_at_a_whole_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.txt");
        let long: String = (0..10_000).map(|i| format!("line {i}\n")).collect();
        fs::write(&path, &long).unwrap();
        let tail = log_tail(&path).unwrap();
        assert!(tail.starts_with("line "));
        assert!(tail.ends_with("line 9999\n"));
        assert!(tail.len() as u64 <= LOG_TAIL_BYTES);
    }

    /// The report of this machine (read-only): `cargo test live_report -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_report() {
        println!("{}", report(&config::default_config_path()));
    }
}
