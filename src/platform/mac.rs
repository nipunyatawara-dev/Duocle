use super::{CaptureJob, Frame, OverlayFrame, SETTINGS_APP_ID};
use crate::{
    config::AppInfo,
    effect::{MAX_MAC_BLUR_RADIUS, Size, image_from_bgra},
    focus::{Desktop, Display, Rect, Window, mac_foreground},
    gesture::Pointer,
};
use anyhow::{Context, Result, anyhow};
use block2::RcBlock;
use objc2::{
    AnyThread, MainThreadMarker, MainThreadOnly, exception,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol},
};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventModifierFlags as Flags, NSImage,
    NSRunningApplication, NSScreen, NSTitlebarSeparatorStyle, NSView, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindow,
    NSWindowAnimationBehavior, NSWindowCollectionBehavior as Collection, NSWindowStyleMask,
    NSWorkspace,
};
use objc2_core_foundation::{CFData, CFRetained, CGRect};
use objc2_core_graphics::{
    CGBitmapInfo, CGColor, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage,
    CGImageAlphaInfo, CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess,
    CGWindowListCopyWindowInfo, CGWindowListOption,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSNumber, NSObjectNSKeyValueCoding, NSPoint, NSProcessInfo,
    NSRect, NSSize, NSString, NSUserDefaults,
};
use objc2_quartz_core::{CALayer, CATransaction};
use objc2_screen_capture_kit::{
    SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration,
};
use objc2_service_management::{SMAppService, SMAppServiceStatus};
use std::{collections::HashMap, panic::AssertUnwindSafe, ptr, sync::mpsc, time::Duration};
use tao::{platform::macos::WindowExtMacOS, window::Window as TaoWindow};

#[derive(Default)]
pub struct System {
    last_external_pid: Option<i32>,
}

fn screen_origin(mtm: MainThreadMarker) -> f64 {
    NSScreen::screens(mtm)
        .iter()
        .next()
        .map_or(0.0, |screen| screen.frame().size.height)
}

fn cg_rect(rect: Rect) -> CGRect {
    NSRect::new(
        NSPoint::new(f64::from(rect.x), f64::from(rect.y)),
        NSSize::new(f64::from(rect.w), f64::from(rect.h)),
    )
}

fn desktop_rect(frame: NSRect, origin: f64) -> Rect {
    Rect::new(
        frame.origin.x.round() as i32,
        (origin - frame.origin.y - frame.size.height).round() as i32,
        frame.size.width.round() as i32,
        frame.size.height.round() as i32,
    )
}

fn app_info(app: &NSRunningApplication) -> AppInfo {
    let name = app
        .localizedName()
        .map_or_else(|| "Application".into(), |name| name.to_string());
    let id = app
        .bundleIdentifier()
        .map(|id| id.to_string())
        .or_else(|| {
            app.executableURL()
                .and_then(|url| url.path())
                .map(|path| path.to_string())
        })
        .unwrap_or_else(|| name.clone());
    AppInfo { id, name }
}

impl System {
    pub fn snapshot(&mut self, settings_window: Option<u64>) -> Desktop {
        let mtm = MainThreadMarker::new().expect("desktop snapshots must run on the main thread");
        let origin = screen_origin(mtm);
        let defaults = NSUserDefaults::standardUserDefaults();
        let stage_enabled = defaults
            .persistentDomainForName(&NSString::from_str("com.apple.WindowManager"))
            .and_then(|prefs| prefs.objectForKey(&NSString::from_str("GloballyEnabled")))
            .and_then(|value| value.downcast::<NSNumber>().ok())
            .is_some_and(|value| value.boolValue());
        let stage_on_right = defaults
            .persistentDomainForName(&NSString::from_str("com.apple.dock"))
            .and_then(|prefs| prefs.objectForKey(&NSString::from_str("orientation")))
            .and_then(|value| value.downcast::<NSString>().ok())
            .is_some_and(|value| value.to_string() == "left");

        let displays = NSScreen::screens(mtm)
            .iter()
            .filter_map(|screen| {
                let id = screen
                    .deviceDescription()
                    .objectForKey(&NSString::from_str("NSScreenNumber"))?
                    .downcast::<NSNumber>()
                    .ok()?
                    .unsignedLongLongValue();
                let bounds = desktop_rect(screen.frame(), origin);
                let work = desktop_rect(screen.visibleFrame(), origin);
                let stage = stage_enabled.then(|| {
                    let width = (work.w / 8).clamp(100, 180).min(work.w);
                    let x = if stage_on_right {
                        work.right() - width
                    } else {
                        work.x
                    };
                    Rect::new(x, work.y, width, work.h)
                });
                Some(Display {
                    id,
                    bounds,
                    work,
                    stage,
                })
            })
            .collect();

        let workspace = NSWorkspace::sharedWorkspace();
        let own_pid = std::process::id() as i32;
        let active_pid = workspace
            .frontmostApplication()
            .map(|app| app.processIdentifier());
        if active_pid != Some(own_pid) {
            self.last_external_pid = active_pid;
        }
        let mut apps_by_pid = HashMap::new();
        let mut apps = Vec::new();
        for app in &workspace.runningApplications() {
            let pid = app.processIdentifier();
            if pid == own_pid {
                continue;
            }
            let info = app_info(&app);
            if app.activationPolicy() == objc2_app_kit::NSApplicationActivationPolicy::Regular {
                apps.push(info.clone());
            }
            apps_by_pid.insert(pid, info);
        }
        let external_app = self
            .last_external_pid
            .and_then(|pid| apps_by_pid.get(&pid))
            .map(|info| info.id.clone());
        let version = NSProcessInfo::processInfo().operatingSystemVersion();
        let corner_radius = if version.majorVersion >= 26 { 21 } else { 11 };
        let mut windows = Vec::new();
        if let Some(list) = CGWindowListCopyWindowInfo(
            CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
            0,
        ) {
            // The CoreGraphics window-list contract is an array of CF dictionaries,
            // toll-free bridged to their Foundation counterparts.
            let list = unsafe {
                &*(CFRetained::as_ptr(&list)
                    .as_ptr()
                    .cast::<NSArray<NSDictionary<NSString, AnyObject>>>())
            };
            let key = |name| NSString::from_str(name);
            let owner_key = key("kCGWindowOwnerPID");
            let layer_key = key("kCGWindowLayer");
            let alpha_key = key("kCGWindowAlpha");
            let number_key = key("kCGWindowNumber");
            let bounds_key = key("kCGWindowBounds");
            let rect_keys = [key("X"), key("Y"), key("Width"), key("Height")];
            for window in list {
                let number = |key: &NSString| window.objectForKey(key)?.downcast::<NSNumber>().ok();
                let Some(pid) = number(&owner_key).map(|value| value.intValue()) else {
                    continue;
                };
                let Some(id) = number(&number_key).map(|value| value.unsignedLongLongValue())
                else {
                    continue;
                };
                let app = if pid == own_pid && Some(id) == settings_window {
                    SETTINGS_APP_ID
                } else if let Some(app) = apps_by_pid.get(&pid) {
                    app.id.as_str()
                } else {
                    continue;
                };
                if matches!(
                    app,
                    "com.apple.dock" | "com.apple.WindowManager" | "com.apple.controlcenter"
                ) {
                    continue;
                }
                let layer = number(&layer_key).map_or(-1, |value| value.intValue());
                if !(0..=8).contains(&layer)
                    || number(&alpha_key).is_some_and(|value| value.doubleValue() < 0.1)
                {
                    continue;
                }
                let Some(bounds) = window.objectForKey(&bounds_key) else {
                    continue;
                };
                // kCGWindowBounds is a CGRect dictionary of four CFNumbers.
                let bounds = unsafe {
                    &*(Retained::as_ptr(&bounds).cast::<NSDictionary<NSString, NSNumber>>())
                };
                let numbers = rect_keys.each_ref().map(|key| {
                    bounds
                        .objectForKey(key)
                        .map_or(0, |value| value.doubleValue().round() as i32)
                });
                let rect = Rect::new(numbers[0], numbers[1], numbers[2], numbers[3]);
                if rect.w < 24 || rect.h < 18 {
                    continue;
                }
                let focusable = layer == 0 && rect.w >= 60 && rect.h >= 45;
                windows.push(Window {
                    id,
                    app: app.into(),
                    bounds: rect,
                    focusable,
                    corner_radius: if layer == 0 { corner_radius } else { 9 },
                });
            }
        }
        let foreground = if active_pid == Some(own_pid) {
            windows
                .iter()
                .find(|window| Some(window.id) == settings_window)
        } else {
            None
        }
        .or_else(|| {
            external_app
                .as_deref()
                .and_then(|app| mac_foreground(&windows, app))
        });
        let focused = foreground.map(|window| window.id);
        let foreground_app = foreground.map(|window| window.app.clone()).or(external_app);
        Desktop {
            displays,
            windows,
            foreground: focused,
            foreground_app,
            apps,
        }
    }

    pub fn pointer(&self) -> Pointer {
        let mtm = MainThreadMarker::new().expect("pointer polling must run on the main thread");
        let flags = NSEvent::modifierFlags_class();
        let point = NSEvent::mouseLocation();
        Pointer {
            x: point.x as i32,
            y: (screen_origin(mtm) - point.y) as i32,
            shift: flags.contains(Flags::Shift),
            control: flags.contains(Flags::Control),
            alt: flags.contains(Flags::Option),
            super_key: flags.contains(Flags::Command),
            down: NSEvent::pressedMouseButtons() != 0,
        }
    }
}

pub fn settings_window_id(window: &TaoWindow) -> u64 {
    // Tao retains this NSWindow until the Rust window is dropped.
    let native = unsafe { &*window.ns_window().cast::<NSWindow>() };
    native.windowNumber() as u64
}

pub fn prepare_titlebar(window: &TaoWindow) -> u32 {
    let native = unsafe { &*window.ns_window().cast::<NSWindow>() };
    native.setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::None);
    (native.frame().size.height - native.contentLayoutRect().size.height)
        .round()
        .clamp(28.0, 64.0) as u32
}

pub fn can_capture() -> bool {
    CGPreflightScreenCaptureAccess()
}

pub fn request_capture() {
    if !CGRequestScreenCaptureAccess() {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
            .spawn();
    }
}

pub fn login_enabled() -> bool {
    let status = unsafe { SMAppService::mainAppService().status() };
    matches!(
        status,
        SMAppServiceStatus::Enabled | SMAppServiceStatus::RequiresApproval
    )
}

pub fn set_login(enabled: bool) -> Result<()> {
    let service = unsafe { SMAppService::mainAppService() };
    if login_enabled() == enabled {
        return Ok(());
    }
    unsafe {
        if enabled {
            service.registerAndReturnError()
        } else {
            service.unregisterAndReturnError()
        }
    }
    .map_err(|error| anyhow!("{}", error.localizedDescription()))?;
    Ok(())
}

pub fn capture(jobs: Vec<CaptureJob>) -> Vec<Frame> {
    objc2::rc::autoreleasepool(|_| {
        let (send, receive) = mpsc::channel();
        let callback = RcBlock::new(move |content: *mut SCShareableContent, _: *mut NSError| {
            let content = unsafe { Retained::retain(content) };
            let pointer = content.map(Retained::into_raw).map(|ptr| ptr as usize);
            if let Err(mpsc::SendError(Some(address))) = send.send(pointer) {
                unsafe { drop(Retained::from_raw(address as *mut SCShareableContent)) };
            }
        });
        unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&callback) };
        let content = receive
            .recv_timeout(Duration::from_secs(2))
            .ok()
            .flatten()
            .and_then(|address| unsafe { Retained::from_raw(address as *mut SCShareableContent) });
        let Some(content) = content else {
            return jobs
                .into_iter()
                .map(|job| (job.display.id, job.size, None))
                .collect();
        };

        let screens = unsafe { content.displays() };
        let applications = unsafe { content.applications() };
        let own_pid = std::process::id() as i32;
        let dock = NSString::from_str("com.apple.dock");
        let (send, receive) = mpsc::channel();
        let mut slots = Vec::new();

        for job in &jobs {
            let id = job.display.id;
            let size = job.size;
            let Some(screen) = screens
                .iter()
                .find(|screen| unsafe { u64::from(screen.displayID()) == id })
            else {
                continue;
            };
            unsafe {
                let excluded = applications
                    .iter()
                    .filter(|app| {
                        app.processID() == own_pid
                            || (!job.chrome && app.bundleIdentifier().isEqualToString(&dock))
                    })
                    .collect::<Vec<_>>();
                let filter =
                    SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                        SCContentFilter::alloc(),
                        &screen,
                        &NSArray::from_retained_slice(&excluded),
                        &NSArray::new(),
                    );
                filter.setIncludeMenuBar(job.chrome);
                let configuration = SCStreamConfiguration::new();
                configuration.setWidth(size.width as usize);
                configuration.setHeight(size.height as usize);
                configuration.setShowsCursor(false);
                configuration.setSourceRect(cg_rect(Rect::new(
                    job.bounds.x - job.display.bounds.x,
                    job.bounds.y - job.display.bounds.y,
                    job.bounds.w,
                    job.bounds.h,
                )));
                let send = send.clone();
                let callback = RcBlock::new(move |img: *mut CGImage, _: *mut NSError| {
                    let frame = img.as_ref().and_then(|img| {
                        let provider = CGImage::data_provider(Some(img))?;
                        let data = CGDataProvider::data(Some(&provider))?;
                        let bytes = data.as_bytes_unchecked();
                        let stride = CGImage::bytes_per_row(Some(img));
                        let row_width = size.width as usize * 4;
                        if CGImage::width(Some(img)) != size.width as usize
                            || CGImage::height(Some(img)) != size.height as usize
                            || stride < row_width
                            || bytes.len() < stride * size.height as usize
                        {
                            return None;
                        }
                        if stride == row_width {
                            image_from_bgra(size, bytes)
                        } else {
                            let packed: Vec<u8> = bytes
                                .chunks_exact(stride)
                                .flat_map(|row| row[..row_width].iter().copied())
                                .collect();
                            image_from_bgra(size, &packed)
                        }
                    });
                    let _ = send.send((id, size, frame));
                });
                SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
                    &filter,
                    &configuration,
                    Some(&callback),
                );
            }
            slots.push(id);
        }
        drop(send);
        let mut frames = Vec::new();
        for _ in slots {
            match receive.recv_timeout(Duration::from_secs(2)) {
                Ok(frame) => frames.push(frame),
                Err(_) => break,
            }
        }
        frames
    })
}

pub struct Overlay {
    window: Retained<NSWindow>,
    backdrop: Retained<NSVisualEffectView>,
    texture_view: Retained<NSView>,
    texture: Retained<CALayer>,
    texture_mask: Retained<CALayer>,
    mask_tiles: Vec<Retained<CALayer>>,
    bounds: Option<Rect>,
    backdrop_radius: f32,
    level: isize,
    visible: bool,
}

impl Overlay {
    pub fn new() -> Result<Self> {
        let mtm = MainThreadMarker::new().context("overlay must be created on the main thread")?;
        let rect = cg_rect(Rect::new(0, 0, 1, 1));
        // This object is managed by Retained, independent of AppKit close semantics.
        let window = unsafe {
            let window = NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            );
            window.setReleasedWhenClosed(false);
            window
        };
        window.setOpaque(false);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setHasShadow(false);
        window.setIgnoresMouseEvents(true);
        window.setCollectionBehavior(
            Collection::CanJoinAllSpaces
                | Collection::FullScreenAuxiliary
                | Collection::Stationary
                | Collection::IgnoresCycle,
        );
        window.setAnimationBehavior(NSWindowAnimationBehavior::None);

        let base = NSView::initWithFrame(NSView::alloc(mtm), rect);
        let backdrop = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), rect);
        backdrop.setMaterial(NSVisualEffectMaterial::FullScreenUI);
        backdrop.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        backdrop.setState(NSVisualEffectState::Active);
        backdrop.setWantsLayer(true);
        backdrop.setHidden(true);
        base.addSubview(&backdrop);
        let texture_view = NSView::initWithFrame(NSView::alloc(mtm), rect);
        texture_view.setWantsLayer(true);
        let texture = CALayer::new();
        texture_view.setLayer(Some(&texture));
        base.addSubview(&texture_view);
        window.setContentView(Some(&base));
        let texture_mask = CALayer::new();
        unsafe { texture.setMask(Some(&texture_mask)) };
        Ok(Self {
            window,
            backdrop,
            texture_view,
            texture,
            texture_mask,
            mask_tiles: Vec::new(),
            bounds: None,
            backdrop_radius: 0.0,
            level: -1,
            visible: false,
        })
    }

    pub fn present(&mut self, frame: OverlayFrame<'_>) -> Result<()> {
        let level = if frame.chrome { 28 } else { 7 };
        if self.level != level {
            self.window.setLevel(level);
            self.level = level;
        }

        CATransaction::begin();
        CATransaction::setDisableActions(true);
        let result = self.update_layers(&frame);
        CATransaction::commit();
        result?;
        self.window.setAlphaValue(f64::from(frame.opacity));
        if !self.visible {
            self.window.orderFrontRegardless();
            self.visible = true;
        }
        Ok(())
    }

    fn update_layers(&mut self, frame: &OverlayFrame<'_>) -> Result<()> {
        let bounds = frame.bounds;
        let resized = self.bounds != Some(bounds);
        if resized {
            let mtm = MainThreadMarker::new().unwrap();
            let position = Rect::new(
                bounds.x,
                screen_origin(mtm) as i32 - bounds.bottom(),
                bounds.w,
                bounds.h,
            );
            let local = cg_rect(Rect::new(0, 0, bounds.w, bounds.h));
            self.window.setFrame_display(cg_rect(position), false);
            self.backdrop.setFrame(local);
            self.texture_view.setFrame(local);
            self.texture.setFrame(local);
            self.texture_mask.setFrame(local);
            self.bounds = Some(bounds);
        }

        let local = |rect: Rect| {
            cg_rect(Rect::new(
                rect.x - bounds.x,
                bounds.bottom() - rect.bottom(),
                rect.w,
                rect.h,
            ))
        };
        if frame.geometry_changed {
            for layer in self
                .mask_tiles
                .drain(frame.tiles.len().min(self.mask_tiles.len())..)
            {
                layer.removeFromSuperlayer();
            }
            let solid = CGColor::new_generic_rgb(0.0, 0.0, 0.0, 1.0);
            for (index, tile) in frame.tiles.iter().enumerate() {
                if index == self.mask_tiles.len() {
                    let layer = CALayer::new();
                    layer.setBackgroundColor(Some(&solid));
                    self.texture_mask.addSublayer(&layer);
                    self.mask_tiles.push(layer);
                }
                self.mask_tiles[index].setFrame(local(tile.rect));
            }
        }
        for (index, tile) in frame.tiles.iter().enumerate() {
            if frame.geometry_changed || tile.is_animating() {
                self.mask_tiles[index].setOpacity(tile.opacity(frame.progress));
            }
        }
        let radius = frame.native_blur_radius;
        if radius > 0.0 {
            // Mask a single full-display sample after blurring it, so window
            // cutouts and the boundaries between tiles cannot clip the blur.
            if self.backdrop_radius == 0.0
                || resized
                || frame.geometry_changed
                || frame.tiles.iter().any(|tile| tile.is_animating())
            {
                let mask = make_backdrop_mask(frame)?;
                self.backdrop.setMaskImage(Some(&mask));
            }
            if self.backdrop_radius == 0.0 {
                self.backdrop.setHidden(false);
                self.backdrop.layoutSubtreeIfNeeded();
                self.backdrop.displayIfNeeded();
            }
            let adjusted = self
                .backdrop
                .layer()
                .is_some_and(|layer| set_backdrop_radius(&layer, radius));
            let strength = if adjusted {
                1.0
            } else {
                // The system recipe remains available if AppKit changes its layers.
                1.0 - (1.0 - radius / MAX_MAC_BLUR_RADIUS).powi(6)
            };
            self.backdrop
                .setAlphaValue(f64::from(strength * frame.native_blur_opacity));
        } else if self.backdrop_radius > 0.0 {
            self.backdrop.setHidden(true);
            self.backdrop.setMaskImage(None);
        }
        self.backdrop_radius = radius;
        if let Some(pixels) = frame.pixels {
            let image = make_image(frame.size, pixels)?;
            // CALayer.contents explicitly accepts a retained CGImage.
            unsafe {
                let contents = &*(CFRetained::as_ptr(&image).as_ptr().cast::<AnyObject>());
                self.texture.setContents(Some(contents));
            }
        }
        Ok(())
    }

    pub fn hide(&mut self) {
        if self.visible {
            self.window.orderOut(None);
            self.visible = false;
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.window.close();
    }
}

fn set_backdrop_radius(layer: &CALayer, radius: f32) -> bool {
    if layer.class().name() == c"CABackdropLayer" {
        let gaussian = NSString::from_str("gaussianBlur");
        let has_filter = layer.filters().is_some_and(|filters| {
            filters.iter().any(|filter| {
                filter.class().name() == c"CAFilter"
                    && filter.downcast::<NSObject>().is_ok_and(|filter| {
                        filter
                            .description()
                            .downcast::<NSString>()
                            .is_ok_and(|name| name.isEqualToString(&gaussian))
                    })
            })
        });
        return has_filter
            && exception::catch(AssertUnwindSafe(|| unsafe {
                layer.setValue_forKeyPath(
                    Some(&NSNumber::new_f32(radius)),
                    &NSString::from_str("filters.gaussianBlur.inputRadius"),
                );
            }))
            .is_ok();
    }
    // The overlay and its layer tree are updated on AppKit's main thread.
    unsafe { layer.sublayers() }.is_some_and(|layers| {
        layers
            .iter()
            .any(|layer| set_backdrop_radius(&layer, radius))
    })
}

fn make_backdrop_mask(frame: &OverlayFrame<'_>) -> Result<Retained<NSImage>> {
    let bounds = frame.bounds;
    let stride = bounds.w as usize * 2;
    let mut pixels = vec![0; stride * bounds.h as usize];
    for tile in frame.tiles {
        let alpha = (tile.opacity(frame.progress) * 255.0).round() as u8;
        let x = (tile.rect.x - bounds.x) as usize * 2;
        let y = (tile.rect.y - bounds.y) as usize;
        for row in y..y + tile.rect.h as usize {
            let start = row * stride + x;
            let end = start + tile.rect.w as usize * 2;
            for pixel in pixels[start..end].as_chunks_mut::<2>().0 {
                pixel[1] = alpha;
            }
        }
    }
    let provider = CGDataProvider::with_cf_data(Some(&CFData::from_bytes(&pixels)))
        .context("create backdrop mask data provider")?;
    let space = CGColorSpace::new_device_gray().context("create gray color space")?;
    let image = unsafe {
        CGImage::new(
            bounds.w as usize,
            bounds.h as usize,
            8,
            16,
            stride,
            Some(&space),
            CGBitmapInfo(CGImageAlphaInfo::PremultipliedLast.0),
            Some(&provider),
            ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .context("create backdrop mask image")?;
    Ok(NSImage::initWithCGImage_size(
        NSImage::alloc(),
        &image,
        NSSize::new(f64::from(bounds.w), f64::from(bounds.h)),
    ))
}

fn make_image(size: Size, pixels: &[u8]) -> Result<CFRetained<CGImage>> {
    let data = CFData::from_bytes(pixels);
    let provider =
        CGDataProvider::with_cf_data(Some(&data)).context("create overlay data provider")?;
    let space = CGColorSpace::new_device_rgb().context("create RGB color space")?;
    let flags =
        CGBitmapInfo::ByteOrder32Host | CGBitmapInfo(CGImageAlphaInfo::PremultipliedFirst.0);
    unsafe {
        CGImage::new(
            size.width as usize,
            size.height as usize,
            8,
            32,
            size.width as usize * 4,
            Some(&space),
            flags,
            Some(&provider),
            ptr::null(),
            true,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .context("create overlay image")
}
