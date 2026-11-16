use super::{CaptureJob, Frame, OverlayFrame, SETTINGS_APP_ID};
use crate::{
    config::AppInfo,
    effect::{Size, image_from_bgra},
    focus::{Desktop, Display, Rect, Window},
    gesture::Pointer,
};
use anyhow::{Context, Result};
use std::{collections::HashMap, io, os::windows::ffi::OsStrExt, path::Path, ptr, slice};
use tao::{
    platform::windows::WindowExtWindows,
    window::{Theme, Window as TaoWindow},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT, RECT, SIZE,
        WPARAM,
    },
    Graphics::{
        Dwm::{
            DWMWA_CAPTION_COLOR, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DWMWA_TEXT_COLOR,
            DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
            DWMWCP_ROUNDSMALL, DwmGetWindowAttribute, DwmSetWindowAttribute,
        },
        Gdi::*,
    },
    System::{
        LibraryLoader::GetModuleHandleW,
        Registry::*,
        Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW},
    },
    UI::{HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
use windows_sys::w;

fn null<T>() -> *mut T {
    ptr::null_mut()
}

fn rect(value: RECT) -> Rect {
    Rect::new(
        value.left,
        value.top,
        value.right - value.left,
        value.bottom - value.top,
    )
}

fn pid(hwnd: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid
}

#[derive(Default)]
pub struct System {
    last_external: Option<u64>,
}

unsafe extern "system" fn collect_monitors(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    context: LPARAM,
) -> i32 {
    let displays = unsafe { &mut *(context as *mut Vec<Display>) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
        displays.push(Display {
            id: monitor as u64,
            bounds: rect(info.rcMonitor),
            work: rect(info.rcWork),
            stage: None,
        });
    }
    1
}

unsafe extern "system" fn collect_windows(hwnd: HWND, context: LPARAM) -> i32 {
    let windows = unsafe { &mut *(context as *mut Vec<HWND>) };
    if unsafe { IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 } {
        windows.push(hwnd);
    }
    1
}

fn class(hwnd: HWND) -> String {
    let mut name = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32) };
    String::from_utf16_lossy(&name[..len.max(0) as usize])
}

fn application(hwnd: HWND, pid: u32) -> AppInfo {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if !handle.is_null() {
        let mut path = [0u16; 1024];
        let mut len = path.len() as u32;
        let success = unsafe {
            let result = QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut len) != 0;
            CloseHandle(handle);
            result
        };
        if success {
            let path = String::from_utf16_lossy(&path[..len as usize]);
            let path = Path::new(&path);
            let id = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            let name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if !id.is_empty() {
                return AppInfo { id, name };
            }
        }
    }
    let name = class(hwnd);
    AppInfo {
        id: format!("class:{}", name.to_lowercase()),
        name,
    }
}

unsafe extern "system" fn find_app_child(hwnd: HWND, context: LPARAM) -> i32 {
    if class(hwnd) == "Windows.UI.Core.CoreWindow" {
        unsafe { *(context as *mut HWND) = hwnd };
        return 0;
    }
    1
}

fn application_window(hwnd: HWND) -> HWND {
    if class(hwnd) == "ApplicationFrameWindow" {
        let mut child: HWND = null();
        unsafe {
            EnumChildWindows(
                hwnd,
                Some(find_app_child),
                (&mut child as *mut HWND) as LPARAM,
            )
        };
        if !child.is_null() {
            return child;
        }
    }
    hwnd
}

fn window_bounds(hwnd: HWND) -> Option<Rect> {
    let mut bounds = RECT::default();
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            (&mut bounds as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
    };
    if result < 0 && unsafe { GetWindowRect(hwnd, &mut bounds) } == 0 {
        return None;
    }
    let bounds = rect(bounds);
    (bounds.w >= 24 && bounds.h >= 18).then_some(bounds)
}

impl System {
    pub fn snapshot(&mut self, settings_window: Option<u64>) -> Desktop {
        let mut displays = Vec::new();
        let mut handles = Vec::new();
        unsafe {
            EnumDisplayMonitors(
                null(),
                ptr::null(),
                Some(collect_monitors),
                (&mut displays as *mut Vec<Display>) as LPARAM,
            );
            EnumWindows(
                Some(collect_windows),
                (&mut handles as *mut Vec<HWND>) as LPARAM,
            );
        }
        let own_pid = std::process::id();
        let foreground = unsafe { GetForegroundWindow() };
        let overview = match class(foreground).as_str() {
            "MultitaskingViewFrame" | "TaskSwitcherWnd" => true,
            "XamlExplorerHostIslandWindow" => pid(foreground) == pid(unsafe { GetShellWindow() }),
            _ => false,
        };
        // Keep the last application while the shell owns the foreground.
        if !overview {
            if foreground.is_null() {
                self.last_external = None;
            } else if pid(foreground) != own_pid {
                self.last_external = Some(foreground as u64);
            }
        }

        let mut by_pid: HashMap<u32, AppInfo> = HashMap::new();
        let mut windows = Vec::new();
        for handle in handles {
            let name = class(handle);
            if matches!(
                name.as_str(),
                "Shell_TrayWnd"
                    | "Shell_SecondaryTrayWnd"
                    | "Progman"
                    | "WorkerW"
                    | "DuocleOverlay"
            ) {
                continue;
            }
            let extended = unsafe { GetWindowLongPtrW(handle, GWL_EXSTYLE) } as u32;
            if extended & WS_EX_NOACTIVATE != 0 && self.last_external != Some(handle as u64) {
                continue;
            }
            let mut cloaked = 0u32;
            unsafe {
                DwmGetWindowAttribute(
                    handle,
                    DWMWA_CLOAKED as u32,
                    (&mut cloaked as *mut u32).cast(),
                    size_of::<u32>() as u32,
                );
            }
            if cloaked != 0 {
                continue;
            }
            let Some(bounds) = window_bounds(handle) else {
                continue;
            };
            let app_handle = application_window(handle);
            let pid = pid(app_handle);
            let app = if pid == own_pid {
                if settings_window != Some(handle as u64) {
                    continue;
                }
                SETTINGS_APP_ID.to_owned()
            } else {
                by_pid
                    .entry(pid)
                    .or_insert_with(|| application(app_handle, pid))
                    .id
                    .clone()
            };
            let focusable = extended & WS_EX_TOOLWINDOW == 0 && bounds.w >= 60 && bounds.h >= 45;
            windows.push(Window {
                id: handle as u64,
                app,
                bounds,
                focusable,
                corner_radius: corner_radius(handle, focusable),
            });
        }
        let foreground = settings_window
            .filter(|id| *id == foreground as u64)
            .or(self.last_external)
            .filter(|id| windows.iter().any(|window| window.id == *id));
        let foreground_app = foreground.and_then(|id| {
            windows
                .iter()
                .find(|window| window.id == id)
                .map(|window| window.app.clone())
        });
        let apps = by_pid.into_values().collect();
        Desktop {
            displays,
            windows,
            foreground,
            foreground_app,
            overview,
            apps,
        }
    }

    pub fn pointer(&self) -> Pointer {
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point) };
        let key = |code| unsafe { GetAsyncKeyState(i32::from(code)) < 0 };
        Pointer {
            x: point.x,
            y: point.y,
            shift: key(VK_SHIFT),
            control: key(VK_CONTROL),
            alt: key(VK_MENU),
            super_key: key(VK_LWIN) || key(VK_RWIN),
            down: key(VK_LBUTTON) || key(VK_RBUTTON) || key(VK_MBUTTON),
        }
    }
}

fn corner_radius(window: HWND, focusable: bool) -> i32 {
    let mut preference = 0i32;
    let dpi = unsafe {
        if IsZoomed(window) != 0 {
            return 0;
        }
        if DwmGetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&mut preference as *mut i32).cast(),
            size_of::<i32>() as u32,
        ) < 0
            || preference == DWMWCP_DONOTROUND
        {
            return 0;
        }
        GetDpiForWindow(window).max(96)
    };
    let points = if preference == DWMWCP_ROUNDSMALL || !focusable {
        4
    } else {
        8
    };
    ((points * dpi + 48) / 96) as i32
}

pub fn settings_window_id(window: &TaoWindow) -> u64 {
    window.hwnd() as u64
}

pub fn show_settings_without_activating(window: &TaoWindow) {
    unsafe { ShowWindow(window.hwnd() as HWND, SW_SHOWNOACTIVATE) };
}

pub fn prepare_titlebar(window: &TaoWindow) -> u32 {
    let hwnd = window.hwnd() as HWND;
    let dark = window.theme() == Theme::Dark;
    // Match the Material surface and on-surface roles from ui/material.css.
    let (surface, ink) = if dark {
        ([0x0f, 0x15, 0x11], [0xde, 0xe4, 0xde])
    } else {
        ([0xf5, 0xfb, 0xf5], [0x17, 0x1d, 0x19])
    };
    let color = |[r, g, b]: [u8; 3]| u32::from_le_bytes([r, g, b, 0]);
    for (attribute, value) in [
        (DWMWA_USE_IMMERSIVE_DARK_MODE, u32::from(dark)),
        (DWMWA_CAPTION_COLOR, color(surface)),
        (DWMWA_TEXT_COLOR, color(ink)),
    ] {
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                attribute as u32,
                (&value as *const u32).cast(),
                size_of::<u32>() as u32,
            );
        }
    }
    0
}

const RUN_KEY: windows_sys::core::PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_NAME: windows_sys::core::PCWSTR = w!("Duocle");

pub fn can_capture() -> bool {
    true
}

pub fn request_capture() {}

pub fn login_enabled() -> bool {
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            RUN_NAME,
            RRF_RT_REG_SZ,
            null(),
            null(),
            &mut 0,
        ) == ERROR_SUCCESS
    }
}

pub fn set_login(enabled: bool) -> Result<()> {
    unsafe {
        if !enabled {
            let status = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_NAME);
            return if matches!(status, ERROR_SUCCESS | ERROR_FILE_NOT_FOUND) {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(status as i32).into())
            };
        }
        let exe = std::env::current_exe()?;
        let mut command: Vec<u16> = "\"".encode_utf16().collect();
        command.extend(exe.as_os_str().encode_wide());
        command.extend("\" --background".encode_utf16());
        command.push(0);
        let mut key = null();
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            0,
            ptr::null(),
            0,
            KEY_SET_VALUE,
            ptr::null(),
            &mut key,
            null(),
        );
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32).into());
        }
        let status = RegSetValueExW(
            key,
            RUN_NAME,
            0,
            REG_SZ,
            command.as_ptr().cast(),
            (command.len() * 2) as u32,
        );
        RegCloseKey(key);
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32).into());
        }
    }
    Ok(())
}

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    pixels: *mut u8,
    bytes: usize,
}

impl Surface {
    fn new(w: i32, h: i32) -> Result<Self> {
        let bytes = (w as usize)
            .checked_mul(h as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .context("invalid overlay dimensions")?;
        unsafe {
            let dc = CreateCompatibleDC(null());
            if dc.is_null() {
                return Err(io::Error::last_os_error().into());
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = null();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null(), 0);
            if bitmap.is_null() || pixels.is_null() {
                DeleteDC(dc);
                return Err(io::Error::last_os_error().into());
            }
            let previous = SelectObject(dc, bitmap);
            if previous.is_null() || previous as isize == -1 {
                DeleteObject(bitmap);
                DeleteDC(dc);
                return Err(io::Error::last_os_error().into());
            }
            Ok(Self {
                dc,
                bitmap,
                previous,
                pixels: pixels.cast(),
                bytes,
            })
        }
    }

    fn pixels(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.pixels, self.bytes) }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

pub fn capture(jobs: Vec<CaptureJob>) -> Vec<Frame> {
    let screen = unsafe { GetDC(null()) };
    let frames = jobs
        .into_iter()
        .map(
            |CaptureJob {
                 display,
                 size,
                 bounds,
             }| {
                let image = if screen.is_null() {
                    None
                } else {
                    Surface::new(size.width as i32, size.height as i32)
                        .ok()
                        .and_then(|mut surface| unsafe {
                            SetStretchBltMode(surface.dc, HALFTONE);
                            SetBrushOrgEx(surface.dc, 0, 0, null());
                            // SRCCOPY omits WS_EX_LAYERED windows, including Duocle's
                            // overlays. CAPTUREBLT would feed the effect back into itself.
                            if StretchBlt(
                                surface.dc,
                                0,
                                0,
                                size.width as i32,
                                size.height as i32,
                                screen,
                                bounds.x,
                                bounds.y,
                                bounds.w,
                                bounds.h,
                                SRCCOPY,
                            ) == 0
                            {
                                return None;
                            }
                            GdiFlush();
                            image_from_bgra(size, surface.pixels())
                        })
                };
                (display.id, size, image)
            },
        )
        .collect();
    if !screen.is_null() {
        unsafe { ReleaseDC(null(), screen) };
    }
    frames
}

pub struct Overlay {
    hwnd: HWND,
    surface: Option<Surface>,
    bounds: Option<Rect>,
    background: Vec<u8>,
    visible: bool,
}

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

impl Overlay {
    pub fn new() -> Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(ptr::null());
            let name = w!("DuocleOverlay");
            let class = WNDCLASSW {
                lpfnWndProc: Some(overlay_proc),
                hInstance: instance,
                lpszClassName: name,
                ..Default::default()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TOPMOST
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW,
                name,
                w!("Duocle overlay"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null(),
                null(),
                instance,
                ptr::null(),
            );
            if hwnd.is_null() {
                return Err(io::Error::last_os_error().into());
            }
            // Also keep the overlay out of screen recording APIs that support
            // display affinity. GDI capture uses the layered-window behavior.
            SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
            Ok(Self {
                hwnd,
                surface: None,
                bounds: None,
                background: Vec::new(),
                visible: false,
            })
        }
    }

    pub fn present(&mut self, frame: OverlayFrame<'_>) -> Result<()> {
        let bounds = frame.bounds;
        let resized = self
            .bounds
            .is_none_or(|old| old.w != bounds.w || old.h != bounds.h);
        if resized {
            let surface = Surface::new(bounds.w, bounds.h)?;
            self.background.resize(surface.bytes, 0);
            self.surface = Some(surface);
        }
        self.bounds = Some(bounds);
        let surface = self.surface.as_mut().context("overlay has no surface")?;
        let width = bounds.w as usize;
        if let Some(pixels) = frame.pixels {
            expand(
                frame.size,
                pixels,
                width,
                bounds.h as usize,
                &mut self.background,
            );
        }
        let repaint = resized || frame.geometry_changed || frame.pixels.is_some();
        let destination = surface.pixels();
        if repaint {
            destination.fill(0);
        }
        for tile in frame.tiles {
            if !repaint && !tile.is_animating() {
                continue;
            }
            let rect = tile.rect;
            let alpha = (tile.opacity(frame.progress) * 256.0).round() as u16;
            let x = (rect.x - bounds.x) as usize;
            for y in (rect.y - bounds.y) as usize..(rect.bottom() - bounds.y) as usize {
                let from = (y * width + x) * 4;
                let to = from + rect.w as usize * 4;
                let row = &mut destination[from..to];
                if alpha == 0 {
                    row.fill(0);
                } else if alpha == 256 {
                    row.copy_from_slice(&self.background[from..to]);
                } else {
                    for (output, input) in row.iter_mut().zip(&self.background[from..to]) {
                        *output = ((u16::from(*input) * alpha + 128) >> 8) as u8;
                    }
                }
            }
        }
        let point = POINT {
            x: bounds.x,
            y: bounds.y,
        };
        let extent = SIZE {
            cx: bounds.w,
            cy: bounds.h,
        };
        let start = POINT::default();
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: (frame.opacity * 255.0).round() as u8,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            if UpdateLayeredWindow(
                self.hwnd,
                null(),
                &point,
                &extent,
                surface.dc,
                &start,
                0,
                &blend,
                ULW_ALPHA,
            ) == 0
            {
                return Err(io::Error::last_os_error().into());
            }
            if !self.visible {
                ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                self.visible = true;
            }
        }
        Ok(())
    }

    pub fn hide(&mut self) {
        if self.visible {
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            self.visible = false;
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.hwnd) };
    }
}

fn expand(size: Size, source: &[u8], width: usize, height: usize, target: &mut [u8]) {
    let source_width = size.width as usize;
    let offsets: Vec<usize> = (0..width).map(|x| (x * source_width / width) * 4).collect();
    let mut cache = vec![0; width * 4];
    let mut previous = None;
    for (y, row) in target.chunks_exact_mut(width * 4).enumerate() {
        let from_y = y * size.height as usize / height;
        if previous != Some(from_y) {
            let src = from_y * source_width * 4;
            for (pixel, x) in cache.as_chunks_mut::<4>().0.iter_mut().zip(&offsets) {
                pixel.copy_from_slice(&source[src + x..src + x + 4]);
            }
            previous = Some(from_y);
        }
        row.copy_from_slice(&cache);
    }
}
