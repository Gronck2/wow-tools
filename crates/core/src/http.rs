//! HTTPS GET over WinHTTP (system proxy and certificates; no TLS crate). Shared by the self-update, ElvUI and the name tables.

use std::sync::OnceLock;

static USER_AGENT: OnceLock<String> = OnceLock::new();

/// The program sets its `wow-tools/<version>` once at start; without it requests say `wow-tools`.
pub fn set_user_agent(agent: &str) {
    let _ = USER_AGENT.set(agent.to_string());
}

#[cfg_attr(not(windows), allow(dead_code))]
fn user_agent() -> &'static str {
    USER_AGENT.get().map_or("wow-tools", String::as_str)
}

#[cfg(windows)]
pub use imp::get;
#[cfg(not(windows))]
pub use stub::get;

#[cfg(windows)]
mod imp {
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
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Handle(h))
    }

    fn ok(r: windows_sys::core::BOOL) -> Result<()> {
        if r == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }

    const MAX_BODY: usize = 128 << 20;

    /// A GET over HTTPS (redirects followed); returns the status and the body.
    pub fn get(url: &str, accept: Option<&str>) -> Result<(u32, Vec<u8>)> {
        let host = url.trim_start_matches("https://").split('/').next().unwrap_or_default();
        request(url, accept).map_err(|e| anyhow::anyhow!(tr!("self_update.network", host = host, error = e)))
    }

    fn request(url: &str, accept: Option<&str>) -> Result<(u32, Vec<u8>)> {
        let Some(rest) = url.strip_prefix("https://") else {
            bail!("https only: {url}")
        };
        let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let path = if path.is_empty() { "/" } else { path };
        let agent = wide(super::user_agent());
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
                // The biggest download is a ~55 MB name table (macro_names, ItemSparse); the exe is smaller.
                if start + available as usize > MAX_BODY {
                    bail!("response over {} MB", MAX_BODY >> 20);
                }
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
mod stub {
    pub fn get(_: &str, _: Option<&str>) -> anyhow::Result<(u32, Vec<u8>)> {
        anyhow::bail!(tr!("autostart.windows_only"))
    }
}
