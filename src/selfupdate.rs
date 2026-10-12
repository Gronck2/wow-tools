//! Updates of WoW Tools itself from the GitHub releases of `CARGO_PKG_REPOSITORY`.
//!
//! `check` asks the GitHub API for the latest published release (drafts and pre-releases are
//! not returned) and keeps the answer in `data\self-update.json`, so the tray checks once a day
//! whatever its restarts. `download` fetches the release's exe and refuses it unless its SHA-256
//! matches the one GitHub reports for the asset (or, for older releases, the `SHA-256: ...` line
//! the release workflow writes into the notes). `install` swaps it in: a running exe can be
//! renamed, so the current one becomes `<exe>.old` (removed by the next start) and the new one
//! takes its name; the caller then starts it and exits. HTTP goes through WinHTTP, which uses
//! the system proxy and certificates.

use anyhow::{Context, Result, bail};
use chrono::{Local, NaiveDateTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// How often the tray asks GitHub.
pub const CHECK_EVERY: chrono::Duration = chrono::Duration::hours(24);

pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// "Gronck2/wow-tools".
fn repo() -> &'static str {
    env!("CARGO_PKG_REPOSITORY").trim_start_matches("https://github.com/").trim_end_matches('/')
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Release {
    /// "0.2.0", without the "v".
    pub version: String,
    /// The release page.
    pub page: String,
    pub asset: Option<Asset>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: Option<String>,
}

/// What `data\self-update.json` remembers between starts.
#[derive(Serialize, Deserialize, Default, Debug)]
struct Saved {
    checked: Option<NaiveDateTime>,
    latest: Option<Release>,
    /// The version a balloon was shown for, so it is shown once.
    notified: Option<String>,
    /// The version being installed: the new exe greets the user once it runs.
    installing: Option<String>,
}

fn saved_path() -> PathBuf {
    crate::config::data_dir().join("self-update.json")
}

fn load() -> Saved {
    fs::read_to_string(saved_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(s: &Saved) {
    let _ = fs::create_dir_all(crate::config::data_dir());
    if let Ok(text) = serde_json::to_string_pretty(s) {
        let _ = fs::write(saved_path(), text);
    }
}

/// "v1.2.3-beta" -> [1, 2, 3].
fn numbers(v: &str) -> Vec<u64> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let v = v.split(['-', '+']).next().unwrap_or_default();
    v.split('.').map(|n| n.parse().unwrap_or(0)).collect()
}

/// `candidate` is a later version than `than`.
pub fn newer(candidate: &str, than: &str) -> bool {
    let (mut a, mut b) = (numbers(candidate), numbers(than));
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a > b
}

/// When GitHub was last asked.
pub fn last_check() -> Option<NaiveDateTime> {
    load().checked
}

/// A check is due: never checked, or longer ago than `CHECK_EVERY`.
pub fn due(now: NaiveDateTime) -> bool {
    load().checked.is_none_or(|t| now - t >= CHECK_EVERY || t > now)
}

/// The release newer than this exe, from the last check.
pub fn available() -> Option<Release> {
    load().latest.filter(|r| newer(&r.version, current()))
}

/// Asks GitHub; returns the newer release, if there is one.
pub fn check() -> Result<Option<Release>> {
    let latest = fetch_latest()?;
    let mut s = load();
    s.checked = Some(Local::now().naive_local());
    s.latest = latest;
    save(&s);
    Ok(s.latest.filter(|r| newer(&r.version, current())))
}

/// True once per version: whether to tell the user about `release`.
pub fn take_notify(release: &Release) -> bool {
    let mut s = load();
    if s.notified.as_deref() == Some(release.version.as_str()) {
        return false;
    }
    s.notified = Some(release.version.clone());
    save(&s);
    true
}

/// The version this exe was just updated to, once (for a "updated" balloon on the first start).
pub fn take_installed() -> Option<String> {
    let mut s = load();
    let v = s.installing.take()?;
    save(&s);
    (v == current()).then_some(v)
}

fn fetch_latest() -> Result<Option<Release>> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo());
    let (status, body) = http::get(&url, Some("application/vnd.github+json"))?;
    // No published release yet (or the repository is not public).
    if status == 404 {
        return Ok(None);
    }
    if status != 200 {
        bail!(tr!("self_update.http", status = status));
    }
    parse_release(&body).map(Some)
}

fn parse_release(body: &[u8]) -> Result<Release> {
    #[derive(Deserialize)]
    struct ApiRelease {
        tag_name: String,
        html_url: String,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        assets: Vec<ApiAsset>,
    }
    #[derive(Deserialize)]
    struct ApiAsset {
        name: String,
        browser_download_url: String,
        size: u64,
        #[serde(default)]
        digest: Option<String>,
    }
    let r: ApiRelease = serde_json::from_slice(body).context("GitHub API")?;
    let notes_hash = r.body.as_deref().and_then(hash_in_notes);
    let asset = r.assets.into_iter().find(|a| a.name.to_ascii_lowercase().ends_with(".exe")).map(|a| Asset {
        sha256: a
            .digest
            .as_deref()
            .and_then(|d| d.strip_prefix("sha256:"))
            .map(str::to_ascii_lowercase)
            .or(notes_hash),
        name: a.name,
        url: a.browser_download_url,
        size: a.size,
    });
    Ok(Release {
        version: r.tag_name.trim_start_matches(['v', 'V']).to_string(),
        page: r.html_url,
        asset,
    })
}

/// The 64 hex digits after "SHA-256" in the release notes.
fn hash_in_notes(notes: &str) -> Option<String> {
    let at = notes.find("SHA-256")?;
    let rest = &notes[at + 7..];
    let start = rest.find(|c: char| c.is_ascii_hexdigit())?;
    let hex: String = rest[start..].chars().take_while(char::is_ascii_hexdigit).collect();
    (hex.len() == 64).then(|| hex.to_ascii_lowercase())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Downloads the release's exe into the data folder and checks it.
pub fn download(release: &Release) -> Result<PathBuf> {
    let Some(asset) = &release.asset else {
        bail!(tr!("self_update.no_asset", version = &release.version, url = &release.page));
    };
    let Some(expected) = &asset.sha256 else {
        bail!(tr!("self_update.no_hash", url = &release.page));
    };
    let (status, bytes) = http::get(&asset.url, None)?;
    if status != 200 {
        bail!(tr!("self_update.http", status = status));
    }
    verify(&bytes, asset.size, expected)?;
    let dir = crate::config::data_dir().join("update");
    fs::create_dir_all(&dir)?;
    let path = dir.join(&asset.name);
    fs::write(&path, &bytes).with_context(|| tr!("error.write", path = path.display()))?;
    Ok(path)
}

fn verify(bytes: &[u8], size: u64, sha256: &str) -> Result<()> {
    if bytes.len() as u64 != size || sha256_hex(bytes) != sha256.to_ascii_lowercase() {
        bail!(tr!("self_update.bad_hash"));
    }
    Ok(())
}

/// Puts `new` in place of `exe` (normally the running one), keeping the old one as `<exe>.old`
/// until the next start.
pub fn install(new: &Path, exe: &Path) -> Result<()> {
    let old = old_path(exe);
    let _ = fs::remove_file(&old);
    let fail = |e: std::io::Error| anyhow::anyhow!(tr!("self_update.replace_failed", path = exe.display(), error = e));
    fs::rename(exe, &old).map_err(fail)?;
    if let Err(e) = fs::copy(new, exe) {
        // Put the working exe back.
        let _ = fs::remove_file(exe);
        let _ = fs::rename(&old, exe);
        return Err(fail(e));
    }
    let _ = fs::remove_file(new);
    Ok(())
}

/// Remembers that `version` was installed, for `take_installed` in the new exe.
pub fn mark_installed(version: &str) {
    let mut s = load();
    s.installing = Some(version.to_string());
    save(&s);
}

fn old_path(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

/// The path this exe was started from, read once: after `install` renames the running file,
/// asking Windows again may give the `.old` name.
pub fn exe_path() -> Result<PathBuf> {
    static EXE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    EXE.get_or_init(|| std::env::current_exe().ok())
        .clone()
        .with_context(|| tr!("autostart.no_exe"))
}

/// Removes what an update left behind: the previous exe and the download folder.
pub fn cleanup() {
    if let Ok(exe) = exe_path() {
        let _ = fs::remove_file(old_path(&exe));
    }
    let _ = fs::remove_dir_all(crate::config::data_dir().join("update"));
}

/// Waits (up to 30 s) for the process that started this one to exit, so the new tray does not
/// meet the old one's single-instance mutex.
pub fn wait_for_exit(pid: u32) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !h.is_null() {
            WaitForSingleObject(h, 30_000);
            CloseHandle(h);
        }
    }
    #[cfg(not(windows))]
    let _ = pid;
}

#[cfg(windows)]
mod http {
    use anyhow::{Result, bail};
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Networking::WinHttp::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    /// Closes a WinHTTP handle when dropped.
    struct Handle(*mut c_void);

    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }

    fn check(h: *mut c_void) -> Result<Handle> {
        if h.is_null() {
            bail!(tr!("self_update.network", error = std::io::Error::last_os_error()));
        }
        Ok(Handle(h))
    }

    fn ok(r: windows_sys::core::BOOL) -> Result<()> {
        if r == 0 {
            bail!(tr!("self_update.network", error = std::io::Error::last_os_error()));
        }
        Ok(())
    }

    /// A GET over HTTPS (redirects followed); returns the status and the body.
    pub fn get(url: &str, accept: Option<&str>) -> Result<(u32, Vec<u8>)> {
        let Some(rest) = url.strip_prefix("https://") else {
            bail!("https only: {url}")
        };
        let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let path = if path.is_empty() { "/" } else { path };
        let agent = wide(&format!("wow-tools/{}", super::current()));
        unsafe {
            let session = check(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0))?;
            ok(WinHttpSetTimeouts(session.0, 15_000, 15_000, 30_000, 60_000))?;
            let connect = check(WinHttpConnect(session.0, wide(host).as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0))?;
            let request = check(WinHttpOpenRequest(
                connect.0,
                wide("GET").as_ptr(),
                wide(path).as_ptr(),
                null(),
                null(),
                null(),
                WINHTTP_FLAG_SECURE,
            ))?;
            let headers = accept.map(|a| wide(&format!("Accept: {a}\r\n")));
            let (hp, hl) = headers.as_ref().map_or((null(), 0), |h| (h.as_ptr(), u32::MAX));
            ok(WinHttpSendRequest(request.0, hp, hl, null(), 0, 0, 0))?;
            ok(WinHttpReceiveResponse(request.0, null_mut()))?;
            let mut status = 0u32;
            let mut len = size_of::<u32>() as u32;
            ok(WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                &mut status as *mut u32 as *mut c_void,
                &mut len,
                null_mut(),
            ))?;
            let mut body = Vec::new();
            loop {
                let mut available = 0u32;
                ok(WinHttpQueryDataAvailable(request.0, &mut available))?;
                if available == 0 {
                    break;
                }
                let start = body.len();
                body.resize(start + available as usize, 0);
                let mut read = 0u32;
                ok(WinHttpReadData(request.0, body[start..].as_mut_ptr() as *mut c_void, available, &mut read))?;
                body.truncate(start + read as usize);
                if read == 0 {
                    break;
                }
            }
            Ok((status, body))
        }
    }
}

#[cfg(not(windows))]
mod http {
    pub fn get(_: &str, _: Option<&str>) -> anyhow::Result<(u32, Vec<u8>)> {
        anyhow::bail!(tr!("autostart.windows_only"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("v0.10.0", "0.9.9"));
        assert!(newer("1.0", "0.99.99"));
        assert!(newer("0.1.1", "0.1"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.1.1"));
        assert!(!newer("v0.1.0-beta", "0.1.0"));
    }

    #[test]
    fn release_is_read_from_the_api_answer() {
        let hash = "c07d4d8858272999a19587729a084604df28a87f0143aef9c3d7d089ab3342ca";
        let json = format!(
            r#"{{"tag_name":"v0.2.0","html_url":"https://github.com/x/y/releases/tag/v0.2.0",
            "body":"SHA-256: `{}`\n\nnotes","assets":[
            {{"name":"notes.txt","browser_download_url":"https://e/notes.txt","size":3}},
            {{"name":"WoW-Tools-0.2.0.exe","browser_download_url":"https://e/a.exe","size":42}}]}}"#,
            hash.to_uppercase()
        );
        let r = parse_release(json.as_bytes()).unwrap();
        assert_eq!(r.version, "0.2.0");
        let a = r.asset.unwrap();
        assert_eq!((a.name.as_str(), a.url.as_str(), a.size), ("WoW-Tools-0.2.0.exe", "https://e/a.exe", 42));
        assert_eq!(a.sha256.as_deref(), Some(hash));

        // GitHub's own digest wins over the notes.
        let json = r#"{"tag_name":"v1.0.0","html_url":"h","body":null,"assets":[
            {"name":"a.exe","browser_download_url":"u","size":1,"digest":"sha256:ABCD"}]}"#;
        assert_eq!(parse_release(json.as_bytes()).unwrap().asset.unwrap().sha256.as_deref(), Some("abcd"));
    }

    /// The real GitHub API and a real download with its digest, on a public repository.
    /// `cargo test --release -- --ignored live_github`
    #[test]
    #[ignore]
    fn live_github() {
        let (status, body) = http::get(
            "https://api.github.com/repos/BurntSushi/ripgrep/releases/latest",
            Some("application/vnd.github+json"),
        )
        .unwrap();
        assert_eq!(status, 200);
        let release = parse_release(&body).unwrap();
        assert!(newer(&release.version, "0.1"), "{release:?}");
        // ripgrep ships archives, not an exe: download its smallest file with a digest.
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let asset = json["assets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["digest"].is_string())
            .min_by_key(|a| a["size"].as_u64())
            .expect("an asset with a digest");
        let sha = asset["digest"].as_str().unwrap().trim_start_matches("sha256:");
        let (status, bytes) = http::get(asset["browser_download_url"].as_str().unwrap(), None).unwrap();
        assert_eq!(status, 200);
        verify(&bytes, asset["size"].as_u64().unwrap(), sha).unwrap();
        let (status, _) = http::get("https://api.github.com/repos/Gronck2/no-such-repo-here/releases/latest", None).unwrap();
        assert_eq!(status, 404);
    }

    #[test]
    fn download_is_checked_by_size_and_hash() {
        let data = b"new exe";
        let hash = sha256_hex(data);
        assert!(verify(data, 7, &hash).is_ok());
        assert!(verify(data, 7, &hash.to_uppercase()).is_ok());
        assert!(verify(data, 8, &hash).is_err());
        assert!(verify(b"new exf", 7, &hash).is_err());
    }

    #[test]
    fn install_swaps_the_exe_and_keeps_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("wow-tools.exe");
        let new = dir.path().join("new.exe");
        fs::write(&exe, "old").unwrap();
        fs::write(&new, "new").unwrap();
        install(&new, &exe).unwrap();
        assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(fs::read_to_string(old_path(&exe)).unwrap(), "old");
        assert!(!new.exists());
    }
}
