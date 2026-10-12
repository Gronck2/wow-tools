//! Running periodically without a console window:
//! - `interval`: a Task Scheduler task running `backup` every N minutes;
//! - `logon`: starts the tray (or headless `watch`) at sign-in (HKCU Run key, no admin needed).
//!
//! Both go through `conhost.exe --headless` so no console flashes.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

const TASK_NAME: &str = "WoW Tools";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "WoW Tools";
/// schtasks limit for the /TR length.
const MAX_TR_LEN: usize = 261;

fn command_line(config: Option<&Path>, args: &str) -> Result<String> {
    let exe = std::env::current_exe().with_context(|| tr!("autostart.no_exe"))?;
    let mut cmd = format!("conhost.exe --headless \"{}\" --log-to-file", exe.display());
    if let Some(cfg) = config {
        cmd.push_str(&format!(" --config \"{}\"", cfg.display()));
    }
    cmd.push(' ');
    cmd.push_str(args);
    Ok(cmd)
}

fn schtasks(args: &[&str]) -> Result<String> {
    let out = Command::new("schtasks").args(args).output().with_context(|| tr!("autostart.schtasks_failed"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        bail!("schtasks: {}", text.trim());
    }
    Ok(text)
}

pub fn install_interval(every_minutes: u64, config: Option<&Path>) -> Result<()> {
    if !(1..=1439).contains(&every_minutes) {
        bail!(tr!("autostart.bad_interval"));
    }
    let tr = command_line(config, "backup")?;
    if tr.len() > MAX_TR_LEN {
        bail!(tr!("autostart.too_long", n = MAX_TR_LEN));
    }
    let mo = every_minutes.to_string();
    schtasks(&["/Create", "/F", "/TN", TASK_NAME, "/SC", "MINUTE", "/MO", &mo, "/TR", &tr])?;
    Ok(())
}

pub fn remove_interval() -> Result<bool> {
    if !interval_installed() {
        return Ok(false);
    }
    schtasks(&["/Delete", "/F", "/TN", TASK_NAME])?;
    Ok(true)
}

pub fn interval_installed() -> bool {
    schtasks(&["/Query", "/TN", TASK_NAME]).is_ok()
}

#[cfg(windows)]
mod run_key {
    use super::*;
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};

    fn open(write: bool) -> Result<RegKey> {
        let flags = if write { KEY_READ | KEY_SET_VALUE } else { KEY_READ };
        Ok(RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, flags)?)
    }

    pub fn install(config: Option<&Path>, headless: bool) -> Result<()> {
        let args = if headless { "watch" } else { "tray" };
        open(true)?.set_value(RUN_VALUE, &command_line(config, args)?)?;
        Ok(())
    }

    pub fn remove() -> Result<bool> {
        let key = open(true)?;
        if key.get_value::<String, _>(RUN_VALUE).is_err() {
            return Ok(false);
        }
        key.delete_value(RUN_VALUE)?;
        Ok(true)
    }

    pub fn get() -> Option<String> {
        open(false).ok()?.get_value(RUN_VALUE).ok()
    }
}

#[cfg(not(windows))]
mod run_key {
    use super::*;
    pub fn install(_: Option<&Path>, _: bool) -> Result<()> {
        bail!(tr!("autostart.windows_only"))
    }
    pub fn remove() -> Result<bool> {
        Ok(false)
    }
    pub fn get() -> Option<String> {
        None
    }
}

pub use run_key::{get as logon_command, install as install_logon, remove as remove_logon};
