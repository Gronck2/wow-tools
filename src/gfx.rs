//! Drawing shared by the panel and the menu (Direct2D / DirectWrite, Windows 11 look):
//! the system theme, geometry in DIPs, text formats, popup windows and PNG snapshots.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows::Win32::Foundation::HWND as WinHwnd;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::HSTRING;
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Shell::{ABM_GETTASKBARPOS, APPBARDATA, SHAppBarMessage};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub type Result<T> = windows::core::Result<T>;

// ---------------------------------------------------------------- geometry

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
    pub fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.x,
            top: self.y,
            right: self.x + self.w,
            bottom: self.y + self.h,
        }
    }
    pub fn inset(&self, d: f32) -> R {
        R {
            x: self.x + d,
            y: self.y + d,
            w: self.w - 2.0 * d,
            h: self.h - 2.0 * d,
        }
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

// ---------------------------------------------------------------- theme

pub fn col(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub bg: D2D1_COLOR_F,
    pub card: D2D1_COLOR_F,
    pub card_hover: D2D1_COLOR_F,
    pub stroke: D2D1_COLOR_F,
    pub text: D2D1_COLOR_F,
    pub text2: D2D1_COLOR_F,
    pub accent: D2D1_COLOR_F,
    pub on_accent: D2D1_COLOR_F,
    pub ok: D2D1_COLOR_F,
    pub warn: D2D1_COLOR_F,
    pub error: D2D1_COLOR_F,
}

fn read_dword(key: &str, value: &str) -> Option<u32> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey(key).ok()?;
    key.get_value::<u32, _>(value).ok()
}

/// The system light/dark mode and accent color.
pub fn theme() -> Theme {
    let dark = read_dword(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "AppsUseLightTheme") == Some(0);
    theme_for(dark)
}

/// A light or dark theme with the system accent (`--ui-snapshot` forces either).
pub fn theme_for(dark: bool) -> Theme {
    // AccentColor is stored as 0xAABBGGRR.
    let a = read_dword(r"Software\Microsoft\Windows\DWM", "AccentColor").unwrap_or(0xFFD47800);
    let (mut r, mut g, mut b) = (a as u8, (a >> 8) as u8, (a >> 16) as u8);
    if dark {
        // A lighter accent reads better on dark backgrounds, like Windows does.
        let mix = |c: u8| (c as f32 + (255.0 - c as f32) * 0.35) as u8;
        (r, g, b) = (mix(r), mix(g), mix(b));
    }
    let accent = col(r, g, b, 1.0);
    // Like Windows 11: dark text on the light dark-mode accent, white on the light-mode one.
    let on_accent = if dark { col(0, 0, 0, 1.0) } else { col(255, 255, 255, 1.0) };
    if dark {
        Theme {
            dark,
            bg: col(0x20, 0x20, 0x20, 1.0),
            card: col(0x2B, 0x2B, 0x2B, 1.0),
            card_hover: col(0x33, 0x33, 0x33, 1.0),
            stroke: col(255, 255, 255, 0.08),
            text: col(255, 255, 255, 1.0),
            text2: col(255, 255, 255, 0.6),
            accent,
            on_accent,
            ok: col(0x6C, 0xCB, 0x5F, 1.0),
            warn: col(0xFC, 0xE1, 0x00, 1.0),
            error: col(0xFF, 0x99, 0xA4, 1.0),
        }
    } else {
        Theme {
            dark,
            bg: col(0xF3, 0xF3, 0xF3, 1.0),
            card: col(0xFF, 0xFF, 0xFF, 1.0),
            card_hover: col(0xF5, 0xF5, 0xF5, 1.0),
            stroke: col(0, 0, 0, 0.08),
            text: col(0x1A, 0x1A, 0x1A, 1.0),
            text2: col(0, 0, 0, 0.6),
            accent,
            on_accent,
            ok: col(0x0F, 0x7B, 0x0F, 1.0),
            warn: col(0x9D, 0x5D, 0x00, 1.0),
            error: col(0xC4, 0x2B, 0x1C, 1.0),
        }
    }
}

// ---------------------------------------------------------------- text

pub const UI_FONT: &str = "Segoe UI Variable Text";
pub const TITLE_FONT: &str = "Segoe UI Variable Display";
pub const ICON_FONT: &str = "Segoe Fluent Icons";

#[derive(Clone, Copy)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub fn dwrite() -> Result<IDWriteFactory> {
    unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }
}

/// Width of `s` set in `fmt`, in DIPs.
pub fn text_width(fmt: &IDWriteTextFormat, s: &str) -> f32 {
    let measure = || -> Result<f32> {
        let s: Vec<u16> = s.encode_utf16().collect();
        let layout = unsafe { dwrite()?.CreateTextLayout(&s, fmt, 10_000.0, 100.0)? };
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut m)? };
        Ok(m.widthIncludingTrailingWhitespace)
    };
    measure().unwrap_or(s.chars().count() as f32 * 7.0)
}

pub fn text_format(dw: &IDWriteFactory, family: &str, size: f32, bold: bool, align: Align) -> Result<IDWriteTextFormat> {
    let weight = if bold { DWRITE_FONT_WEIGHT_SEMI_BOLD } else { DWRITE_FONT_WEIGHT_NORMAL };
    let align = match align {
        Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
        Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
        Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
    };
    unsafe {
        let f = dw.CreateTextFormat(
            &HSTRING::from(family),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            &HSTRING::from("ru-RU"),
        )?;
        f.SetTextAlignment(align)?;
        f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        // Long lines end with "…" instead of being cut mid-letter.
        let sign = dw.CreateEllipsisTrimmingSign(&f)?;
        let trimming = DWRITE_TRIMMING {
            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
            delimiter: 0,
            delimiterCount: 0,
        };
        f.SetTrimming(&trimming, &sign)?;
        Ok(f)
    }
}

/// Like `text_format`, but for paragraphs: words wrap, the text starts at the top.
pub fn wrap_format(dw: &IDWriteFactory, size: f32) -> Result<IDWriteTextFormat> {
    let f = text_format(dw, UI_FONT, size, false, Align::Left)?;
    unsafe {
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
        f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
    }
    Ok(f)
}

/// The height `s` takes in `fmt` wrapped at `width`.
pub fn text_height(fmt: &IDWriteTextFormat, s: &str, width: f32) -> f32 {
    let measure = || -> Result<f32> {
        let s: Vec<u16> = s.encode_utf16().collect();
        let layout = unsafe { dwrite()?.CreateTextLayout(&s, fmt, width, 10_000.0)? };
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut m)? };
        Ok(m.height)
    };
    measure().unwrap_or(18.0 * s.lines().count().max(1) as f32)
}

/// Like `text_format`, but a long path loses its middle ("C:\Users\…\data\backups"), not its end.
pub fn path_format(dw: &IDWriteFactory, size: f32) -> Result<IDWriteTextFormat> {
    let f = text_format(dw, UI_FONT, size, false, Align::Left)?;
    unsafe {
        let sign = dw.CreateEllipsisTrimmingSign(&f)?;
        let trimming = DWRITE_TRIMMING {
            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
            delimiter: '\\' as u32,
            delimiterCount: 2,
        };
        f.SetTrimming(&trimming, &sign)?;
    }
    Ok(f)
}

// ---------------------------------------------------------------- painting

/// A render target plus one brush whose color is set per call.
pub struct Painter<'a> {
    pub rt: &'a ID2D1RenderTarget,
    pub brush: &'a ID2D1SolidColorBrush,
}

impl Painter<'_> {
    /// Draws an icon (only for `ui-snapshot`: it makes a WIC factory on every call).
    pub fn icon(&self, icon: HICON, r: R) {
        use windows::Win32::Graphics::Imaging::*;
        use windows::Win32::System::Com::*;
        use windows_sys::Win32::Graphics::Gdi::{
            BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC,
        };
        unsafe {
            // The icon's 32-bit pixels, top-down.
            let mut ii: ICONINFO = zeroed();
            GetIconInfo(icon, &mut ii);
            let mut bm: BITMAP = zeroed();
            GetObjectW(ii.hbmColor, size_of::<BITMAP>() as i32, &mut bm as *mut _ as _);
            let (w, h) = (bm.bmWidth, bm.bmHeight);
            let mut bi: BITMAPINFO = zeroed();
            bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w;
            bi.bmiHeader.biHeight = -h;
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB;
            let mut px = vec![0u8; (w * h * 4) as usize];
            let dc = GetDC(null_mut());
            GetDIBits(dc, ii.hbmColor, 0, h as u32, px.as_mut_ptr() as _, &mut bi, DIB_RGB_COLORS);
            ReleaseDC(null_mut(), dc);
            DeleteObject(ii.hbmColor);
            DeleteObject(ii.hbmMask);
            let Ok(wic) = CoCreateInstance::<_, IWICImagingFactory>(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) else {
                return;
            };
            let Ok(src) = wic.CreateBitmapFromMemory(w as u32, h as u32, &GUID_WICPixelFormat32bppBGRA, (w * 4) as u32, &px) else {
                return;
            };
            let Ok(conv) = wic.CreateFormatConverter() else { return };
            if conv
                .Initialize(
                    &src,
                    &GUID_WICPixelFormat32bppPBGRA,
                    WICBitmapDitherTypeNone,
                    None,
                    0.0,
                    WICBitmapPaletteTypeCustom,
                )
                .is_err()
            {
                return;
            }
            if let Ok(bmp) = self.rt.CreateBitmapFromWicBitmap(&conv, None) {
                self.rt
                    .DrawBitmap(&bmp, Some(&r.d2d()), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None);
            }
        }
    }

    pub fn clear(&self, c: D2D1_COLOR_F) {
        unsafe { self.rt.Clear(Some(&c)) }
    }

    pub fn fill(&self, r: R, radius: f32, c: D2D1_COLOR_F) {
        unsafe {
            self.brush.SetColor(&c);
            let rr = D2D1_ROUNDED_RECT {
                rect: r.d2d(),
                radiusX: radius,
                radiusY: radius,
            };
            self.rt.FillRoundedRectangle(&rr, self.brush);
        }
    }

    pub fn stroke(&self, r: R, radius: f32, c: D2D1_COLOR_F, width: f32) {
        unsafe {
            self.brush.SetColor(&c);
            let rr = D2D1_ROUNDED_RECT {
                rect: r.inset(width / 2.0).d2d(),
                radiusX: radius,
                radiusY: radius,
            };
            self.rt.DrawRoundedRectangle(&rr, self.brush, width, None);
        }
    }

    pub fn circle(&self, x: f32, y: f32, radius: f32, c: D2D1_COLOR_F) {
        unsafe {
            self.brush.SetColor(&c);
            let e = D2D1_ELLIPSE {
                point: windows_numerics::Vector2 { X: x, Y: y },
                radiusX: radius,
                radiusY: radius,
            };
            self.rt.FillEllipse(&e, self.brush);
        }
    }

    pub fn text(&self, s: &str, fmt: &IDWriteTextFormat, r: R, c: D2D1_COLOR_F) {
        unsafe {
            self.brush.SetColor(&c);
            let s: Vec<u16> = s.encode_utf16().collect();
            self.rt
                .DrawText(&s, fmt, &r.d2d(), self.brush, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
        }
    }

    /// Shifts everything drawn after it (scrolling).
    pub fn offset(&self, dx: f32, dy: f32) {
        unsafe { self.rt.SetTransform(&windows_numerics::Matrix3x2::translation(dx, dy)) }
    }

    /// A small on/off switch like the Windows 11 toggle.
    pub fn switch(&self, sw: R, on: bool, t: &Theme) {
        let radius = sw.h / 2.0;
        if on {
            self.fill(sw, radius, t.accent);
            self.circle(sw.x + sw.w - radius, sw.cy(), radius - 4.0, t.on_accent);
        } else {
            self.stroke(sw, radius, t.text2, 1.0);
            self.circle(sw.x + radius, sw.cy(), radius - 4.5, t.text2);
        }
    }
}

/// A popup window's Direct2D surface, created lazily for the current window size.
pub struct Surface {
    pub hwnd: HWND,
    pub scale: f32,
    d2d: ID2D1Factory,
    rt: Option<(ID2D1HwndRenderTarget, ID2D1SolidColorBrush)>,
}

impl Surface {
    pub fn new(hwnd: HWND, scale: f32) -> Result<Self> {
        let d2d = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        Ok(Self { hwnd, scale, d2d, rt: None })
    }

    /// Drops the render target, e.g. after the window was resized.
    pub fn reset(&mut self) {
        self.rt = None;
    }

    pub fn paint(&mut self, draw: impl FnOnce(&Painter)) {
        unsafe {
            if self.rt.is_none() {
                let mut rc: RECT = zeroed();
                GetClientRect(self.hwnd, &mut rc);
                let dpi = 96.0 * self.scale;
                let props = D2D1_RENDER_TARGET_PROPERTIES {
                    dpiX: dpi,
                    dpiY: dpi,
                    ..Default::default()
                };
                let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                    hwnd: WinHwnd(self.hwnd),
                    pixelSize: D2D_SIZE_U {
                        width: (rc.right - rc.left) as u32,
                        height: (rc.bottom - rc.top) as u32,
                    },
                    presentOptions: D2D1_PRESENT_OPTIONS_NONE,
                };
                let Ok(rt) = self.d2d.CreateHwndRenderTarget(&props, &hp) else { return };
                rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                let Ok(brush) = rt.CreateSolidColorBrush(&col(0, 0, 0, 1.0), None) else {
                    return;
                };
                self.rt = Some((rt, brush));
            }
            let (rt, brush) = self.rt.as_ref().unwrap();
            rt.BeginDraw();
            draw(&Painter { rt, brush });
            if rt.EndDraw(None, None).is_err() {
                self.rt = None;
            }
        }
    }
}

// ---------------------------------------------------------------- windows

/// Where the taskbar is docked: 0 left, 1 top, 2 right, 3 bottom.
pub fn taskbar_edge() -> u32 {
    unsafe {
        let mut abd: APPBARDATA = zeroed();
        abd.cbSize = size_of::<APPBARDATA>() as u32;
        if SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd) != 0 { abd.uEdge } else { 3 }
    }
}

/// Cursor position, the work area of its monitor and that monitor's scale.
pub fn cursor_monitor() -> (POINT, RECT, f32) {
    unsafe {
        let mut pt: POINT = zeroed();
        GetCursorPos(&mut pt);
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(monitor, &mut mi);
        let (mut dpi, mut dpi_y) = (96u32, 96u32);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y);
        (pt, mi.rcWork, dpi as f32 / 96.0)
    }
}

/// Creates a hidden-until-shown topmost popup with Windows 11 rounded corners.
/// Position and size are in pixels.
pub fn create_popup(class: &str, wndproc: WNDPROC, (x, y, w, h): (i32, i32, i32, i32), dark: bool, small_corners: bool) -> HWND {
    unsafe {
        let hinst = GetModuleHandleW(null());
        let class: Vec<u16> = class.encode_utf16().chain([0]).collect();
        let mut wc: WNDCLASSW = zeroed();
        wc.style = CS_DROPSHADOW;
        wc.lpfnWndProc = wndproc;
        wc.hInstance = hinst;
        wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc); // fails harmlessly when already registered
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            x,
            y,
            w,
            h,
            null_mut(),
            null_mut(),
            hinst,
            null(),
        );
        // DWMWA_WINDOW_CORNER_PREFERENCE: 2 = round, 3 = round small (menus).
        let round: u32 = if small_corners { 3 } else { 2 };
        DwmSetWindowAttribute(hwnd, 33, &round as *const u32 as _, 4);
        // DWMWA_USE_IMMERSIVE_DARK_MODE: dark border in dark mode.
        let dark = dark as u32;
        DwmSetWindowAttribute(hwnd, 20, &dark as *const u32 as _, 4);
        hwnd
    }
}

/// Creates a regular (taskbar, title bar) window of `w`×`h` DIPs of client area, centered on
/// the cursor's monitor and no taller than its work area, with a title bar in the theme's colors.
/// Returns the window and the monitor scale.
pub fn create_window(class: &str, title: &str, wndproc: WNDPROC, (w, h): (f32, f32), t: &Theme) -> (HWND, f32) {
    unsafe {
        let hinst = GetModuleHandleW(null());
        let class: Vec<u16> = class.encode_utf16().chain([0]).collect();
        let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
        let mut wc: WNDCLASSW = zeroed();
        wc.lpfnWndProc = wndproc;
        wc.hInstance = hinst;
        wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        let (_, work, scale) = cursor_monitor();
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: (w * scale).round() as i32,
            bottom: (h * scale).round() as i32,
        };
        AdjustWindowRectExForDpi(&mut rc, style, 0, 0, (96.0 * scale) as u32);
        let (ww, wh) = (rc.right - rc.left, (rc.bottom - rc.top).min(work.bottom - work.top));
        let x = work.left + ((work.right - work.left) - ww).max(0) / 2;
        let y = work.top + ((work.bottom - work.top) - wh).max(0) / 2;
        let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), style, x, y, ww, wh, null_mut(), null_mut(), hinst, null());
        let dark = t.dark as u32;
        DwmSetWindowAttribute(hwnd, 20, &dark as *const u32 as _, 4); // DWMWA_USE_IMMERSIVE_DARK_MODE
        let bg = t.bg;
        let caption: u32 = (bg.r * 255.0) as u32 | ((bg.g * 255.0) as u32) << 8 | ((bg.b * 255.0) as u32) << 16;
        DwmSetWindowAttribute(hwnd, 35, &caption as *const u32 as _, 4); // DWMWA_CAPTION_COLOR
        (hwnd, scale)
    }
}

/// Shows a window. Not `ShowWindow`: its first call in a process started hidden (autostart through
/// `conhost --headless`, `Start-Process -WindowStyle Hidden`) takes the start mode and keeps it hidden.
pub fn show(hwnd: HWND) {
    unsafe { SetWindowPos(hwnd, null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW) };
}

/// The system "Select folder" dialog; `None` when cancelled.
pub fn pick_folder(owner: HWND, title: &str, start: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    use windows::Win32::System::Com::*;
    use windows::Win32::UI::Shell::*;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let opts = dlg.GetOptions().ok()?;
        dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        dlg.SetTitle(&HSTRING::from(title)).ok()?;
        if let Some(item) = start
            .filter(|d| d.is_dir())
            .and_then(|d| SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(d.as_os_str()), None).ok())
        {
            let _ = dlg.SetFolder(&item);
        }
        dlg.Show(Some(WinHwnd(owner))).ok()?;
        let name = dlg.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as _));
        path.map(Into::into)
    }
}

// ---------------------------------------------------------------- snapshots

/// Draws into an offscreen bitmap of `w`×`h` DIPs and saves it as PNG.
pub fn render_png(path: &str, w: f32, h: f32, scale: f32, draw: impl FnOnce(&Painter)) -> Result<()> {
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::*;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let (pw, ph) = ((w * scale) as u32, (h * scale) as u32);
        let bitmap = wic.CreateBitmap(pw, ph, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
        let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0 * scale,
            dpiY: 96.0 * scale,
            ..Default::default()
        };
        let rt = d2d.CreateWicBitmapRenderTarget(&bitmap, &props)?;
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let brush = rt.CreateSolidColorBrush(&col(0, 0, 0, 1.0), None)?;
        rt.BeginDraw();
        draw(&Painter { rt: &rt, brush: &brush });
        rt.EndDraw(None, None)?;

        let stream = wic.CreateStream()?;
        stream.InitializeFromFilename(&HSTRING::from(path), windows::Win32::Foundation::GENERIC_WRITE.0)?;
        let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, null_mut())?;
        let frame = frame.unwrap();
        frame.Initialize(None)?;
        frame.SetSize(pw, ph)?;
        let mut format = GUID_WICPixelFormat32bppPBGRA;
        frame.SetPixelFormat(&mut format)?;
        frame.WriteSource(&bitmap, null())?;
        frame.Commit()?;
        encoder.Commit()?;
    }
    Ok(())
}
