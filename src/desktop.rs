use crate::{
    automation::{self, Command, Paths},
    config::{AppInfo, Mode, Reveal, Settings, Shortcuts, parse_color},
    effect::{self, Size},
    focus::{self, Coverage, Desktop, Mask},
    gesture::{Gestures, Pointer},
    platform::{self, CaptureJob, CaptureResult, Overlay, OverlayFrame, SETTINGS_APP_ID, System},
};
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use image::RgbImage;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::File,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tao::{
    dpi::LogicalSize,
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder},
};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};
use wry::{WebView, WebViewBuilder};

enum Message {
    Ui(String),
    TrayMenu,
    Menu(MenuEvent),
    HotKey(GlobalHotKeyEvent),
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum UiAction {
    Ready,
    Toggle,
    Mode { value: Mode },
    Patch { key: String, value: Value },
    Reset,
    Peek,
    Login { value: bool },
    Capture,
    Close,
    Quit,
}

struct SettingsWindow {
    _webview: WebView,
    window: Window,
    #[cfg(target_os = "windows")]
    hidden_for_overview: bool,
}

struct Tray {
    icon: TrayIcon,
    toggle: MenuItem,
    ignore: MenuItem,
    ignore_app: Option<AppInfo>,
}

#[derive(Default)]
struct Screen {
    overlay: Option<Overlay>,
    coverage: Option<Coverage>,
    mask_progress: f32,
    texture: Vec<u8>,
    texture_from: Option<Vec<u8>>,
    texture_to: Vec<u8>,
    painted_revision: u64,
    opacity: f32,
    image: Option<RgbImage>,
    fresh_image: bool,
}

impl Screen {
    fn hide(&mut self) {
        if let Some(overlay) = &mut self.overlay {
            overlay.hide();
        }
        self.coverage = None;
        self.opacity = 0.0;
    }
}

struct App {
    paths: Paths,
    _lock: File,
    settings: Settings,
    window: Option<SettingsWindow>,
    tray: Option<Tray>,
    hotkeys: GlobalHotKeyManager,
    bindings: Vec<(HotKey, &'static str)>,
    proxy: EventLoopProxy<Message>,
    system: System,
    desktop: Desktop,
    gestures: Gestures,
    monitors: HashMap<u64, Screen>,
    masks: Vec<(u64, Mask)>,
    start: Instant,
    next_snapshot: Instant,
    next_request: Instant,
    next_capture: Instant,
    next_capture_check: Instant,
    capture_rx: Option<mpsc::Receiver<CaptureResult>>,
    capture_generation: u64,
    captured: bool,
    can_capture: bool,
    launched_at_login: bool,
    revision: u64,
    opacity: f32,
    target_visible: bool,
    wants_focus: bool,
    planned_revision: u64,
    reveal_origin: Pointer,
    last_frame: Instant,
    last_pointer: Instant,
    pointer: Pointer,
    paused: bool,
    exit: bool,
    ui_state: String,
    notice: Option<String>,
}

pub fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    if let Some(argument) = args.first()
        && argument != "--background"
        && !argument.starts_with("duocle://")
    {
        // The distributable uses the GUI subsystem, but terminal invocations
        // of that executable can still attach to the caller's console.
        unsafe { windows_sys::Win32::System::Console::AttachConsole(u32::MAX) };
    }

    let paths = Paths::new()?;
    let is_uri = args.first().is_some_and(|arg| arg.starts_with("duocle://"));
    let command = if is_uri {
        Some(automation::parse_url(&args[0])?)
    } else {
        automation::parse_cli(&args)?
    };
    if !is_uri {
        if let Some(command) = command {
            let value = paths.invoke(command)?;
            if !value.is_null() {
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            return Ok(());
        }
        if args
            .first()
            .is_some_and(|arg| matches!(arg.as_str(), "-h" | "--help"))
        {
            return Ok(());
        }
    }
    let Some(lock) = paths.lock()? else {
        if args.first().is_none_or(|arg| arg != "--background") {
            paths.invoke(command.unwrap_or(Command::Settings))?;
        }
        return Ok(());
    };

    let (settings, notice) = match Settings::load(&paths.settings) {
        Ok(settings) => (settings, None),
        Err(error) => (
            Settings::default(),
            Some(format!("Duocle could not read its saved settings. {error}")),
        ),
    };

    let event_loop = EventLoopBuilder::with_user_event().build();
    #[cfg(target_os = "macos")]
    let mut event_loop = event_loop;
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
        event_loop.set_activate_ignoring_other_apps(false);
    }
    let proxy = event_loop.create_proxy();
    let now = Instant::now();
    let mut app = App {
        paths,
        _lock: lock,
        settings,
        window: None,
        tray: None,
        hotkeys: GlobalHotKeyManager::new().context("initialize global shortcuts")?,
        bindings: Vec::new(),
        proxy: proxy.clone(),
        system: System::default(),
        desktop: Desktop::default(),
        gestures: Gestures::default(),
        monitors: HashMap::new(),
        masks: Vec::new(),
        start: now,
        next_snapshot: now,
        next_request: now,
        next_capture: now,
        next_capture_check: now,
        capture_rx: None,
        capture_generation: 0,
        captured: false,
        can_capture: false,
        launched_at_login: false,
        revision: 1,
        opacity: 0.0,
        target_visible: false,
        wants_focus: false,
        planned_revision: 0,
        reveal_origin: Pointer::default(),
        last_frame: now,
        last_pointer: now,
        pointer: Pointer::default(),
        paused: false,
        exit: false,
        ui_state: String::new(),
        notice,
    };

    let sender = proxy.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            let _ = sender.send_event(Message::TrayMenu);
        }
    }));
    let sender = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = sender.send_event(Message::Menu(event));
    }));
    GlobalHotKeyEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(Message::HotKey(event));
    }));

    let show =
        !is_uri && !args.iter().any(|arg| arg == "--background") && !app.paths.settings.exists();
    let mut startup = command;
    event_loop.run(move |event, target, control| {
        match event {
            Event::NewEvents(StartCause::Init) => {
                if let Err(error) = app.initialize(target, show, startup.take()) {
                    app.report(error);
                    if app.tray.is_none() {
                        let _ = app.show(target, true);
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if let Some(window) = &app.window {
                    if app.tray.is_none() {
                        app.exit = true;
                    } else {
                        window.window.set_visible(false);
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::Focused(_),
                ..
            } => {
                app.next_snapshot = Instant::now();
            }
            #[cfg(windows)]
            Event::WindowEvent {
                event: WindowEvent::ThemeChanged(_),
                ..
            } => {
                if let Some(window) = &app.window {
                    platform::prepare_titlebar(&window.window);
                }
            }
            Event::MainEventsCleared => app.tick(target),
            Event::UserEvent(message) => {
                if let Err(error) = app.message(target, message) {
                    app.report(error);
                }
            }
            #[cfg(target_os = "macos")]
            Event::Reopen { .. } => {
                let result = app.show(target, true);
                if let Err(error) = result {
                    app.report(error);
                }
            }
            #[cfg(target_os = "macos")]
            Event::Opened { urls } => {
                for url in urls {
                    let result = automation::parse_url(url.as_str())
                        .and_then(|command| app.execute(target, command));
                    if let Err(error) = result {
                        app.report(error);
                    }
                }
            }
            _ => {}
        }
        if app.exit {
            for screen in app.monitors.values_mut() {
                if let Some(overlay) = &mut screen.overlay {
                    overlay.hide();
                }
            }
            *control = ControlFlow::Exit;
        } else {
            let active = app.settings.enabled || app.settings.shake || app.opacity > 0.0;
            *control = ControlFlow::WaitUntil(
                app.last_frame + Duration::from_millis(if active { 16 } else { 85 }),
            );
        }
    });
}

impl App {
    fn initialize(
        &mut self,
        target: &EventLoopWindowTarget<Message>,
        show: bool,
        command: Option<Command>,
    ) -> Result<()> {
        self.can_capture = platform::can_capture();
        self.launched_at_login = platform::login_enabled();
        let menu = Menu::new();
        let toggle = MenuItem::with_id("toggle", "Enable focus", true, None);
        let ignore = MenuItem::with_id("ignore", "Ignore front app", false, None);
        menu.append_items(&[
            &toggle,
            &MenuItem::with_id("mode", "Switch focus mode", true, None),
            &ignore,
            &MenuItem::with_id("peek", "Peek for two seconds", true, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("settings", "Duocle settings…", true, None),
            &MenuItem::with_id("quit", "Quit Duocle", true, None),
        ])?;
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(false)
            .with_icon_as_template(cfg!(target_os = "macos"))
            .with_tooltip("Duocle")
            .with_icon(tray_image(self.settings.enabled)?)
            .build()?;
        self.tray = Some(Tray {
            icon,
            toggle,
            ignore,
            ignore_app: None,
        });
        if let Err(error) = self.register(&self.settings.shortcuts.clone()) {
            self.notice = Some(format!("A global shortcut is unavailable. {error}"));
        }
        self.update_tray();
        if show {
            self.show(target, true)?;
        }
        if let Some(command) = command {
            self.execute(target, command)?;
        }
        Ok(())
    }

    fn show(&mut self, target: &EventLoopWindowTarget<Message>, visible: bool) -> Result<()> {
        if visible && self.window.is_none() {
            self.snapshot_desktop();
            let builder = WindowBuilder::new()
                .with_title("Duocle")
                .with_visible(false)
                .with_inner_size(LogicalSize::new(780.0, 720.0))
                .with_min_inner_size(LogicalSize::new(500.0, 440.0));
            #[cfg(target_os = "macos")]
            let builder = {
                use tao::platform::macos::WindowBuilderExtMacOS;
                builder
                    .with_titlebar_transparent(true)
                    .with_title_hidden(true)
                    .with_fullsize_content_view(true)
                    .with_movable_by_window_background(true)
            };
            let window = builder.build(target)?;
            let titlebar_inset = platform::prepare_titlebar(&window);

            let proxy = self.proxy.clone();
            let webview = WebViewBuilder::new()
                .with_html(settings_html(titlebar_inset))
                .with_incognito(true)
                .with_navigation_handler(|url| url == "about:blank" || url == "about:srcdoc")
                .with_ipc_handler(move |request| {
                    let _ = proxy.send_event(Message::Ui(request.into_body()));
                })
                .build(&window)
                .context("open the Duocle settings web view")?;
            self.window = Some(SettingsWindow {
                _webview: webview,
                window,
                #[cfg(target_os = "windows")]
                hidden_for_overview: false,
            });
            self.ui_state.clear();
        }
        if let Some(window) = &mut self.window {
            #[cfg(target_os = "windows")]
            {
                window.hidden_for_overview = visible && self.desktop.overview;
                if window.hidden_for_overview {
                    window.window.set_visible(false);
                    return Ok(());
                }
            }
            window.window.set_visible(visible);
            if visible {
                window.window.set_focus();
            }
        }
        self.publish();
        Ok(())
    }

    fn report(&mut self, error: anyhow::Error) {
        eprintln!("Duocle: {error:#}");
        self.notice = Some(format!("{error:#}"));
        self.publish();
    }

    fn message(&mut self, target: &EventLoopWindowTarget<Message>, message: Message) -> Result<()> {
        let command = match message {
            Message::Ui(payload) => {
                match serde_json::from_str::<UiAction>(&payload)? {
                    UiAction::Ready => self.ui_state.clear(),
                    UiAction::Toggle => {
                        self.execute(target, Command::Toggle)?;
                    }
                    UiAction::Mode { value } => {
                        self.execute(target, Command::Mode(value))?;
                    }
                    UiAction::Patch { key, value } => {
                        self.patch(&key, value)?;
                    }
                    UiAction::Reset => self.commit(Settings::default())?,
                    UiAction::Peek => self.gestures.peek(self.millis()),
                    UiAction::Login { value } => {
                        platform::set_login(value)?;
                        self.launched_at_login = platform::login_enabled();
                    }
                    UiAction::Capture => platform::request_capture(),
                    UiAction::Close => self.show(target, false)?,
                    UiAction::Quit => {
                        self.execute(target, Command::Quit)?;
                    }
                }
                self.publish();
                return Ok(());
            }
            Message::TrayMenu => {
                let app = front_app(&self.read_desktop());
                if let Some(tray) = &mut self.tray {
                    tray.ignore.set_enabled(app.is_some());
                    let label = match &app {
                        Some(app) if self.settings.is_ignored(&app.id) => {
                            format!("Include {}", app.name)
                        }
                        Some(app) => format!("Ignore {}", app.name),
                        None => "Ignore front app".into(),
                    };
                    tray.ignore.set_text(label);
                    tray.ignore_app = app;
                    tray.icon.show_menu();
                }
                return Ok(());
            }
            Message::Menu(event) => match event.id.as_ref() {
                "toggle" => Some(Command::Toggle),
                "mode" => return self.switch_mode(),
                "ignore" => {
                    if let Some(app) = self.tray.as_mut().and_then(|tray| tray.ignore_app.take()) {
                        self.toggle_ignore(app)?;
                    }
                    return Ok(());
                }
                "peek" => Some(Command::Peek),
                "settings" => Some(Command::Settings),
                "quit" => Some(Command::Quit),
                _ => None,
            },
            Message::HotKey(event) if event.state == HotKeyState::Pressed => {
                match self
                    .bindings
                    .iter()
                    .find(|(key, _)| key.id() == event.id)
                    .map(|entry| entry.1)
                {
                    Some("toggle") => Some(Command::Toggle),
                    Some("mode") => return self.switch_mode(),
                    Some("ignore") => Some(Command::Ignore),
                    Some("peek") => Some(Command::Peek),
                    Some("settings") => {
                        let visible = self
                            .window
                            .as_ref()
                            .is_none_or(|window| !window.window.is_visible());
                        return self.show(target, visible);
                    }
                    _ => None,
                }
            }
            Message::HotKey(_) => None,
        };
        if let Some(command) = command {
            self.execute(target, command)?;
        }
        Ok(())
    }

    fn execute(
        &mut self,
        target: &EventLoopWindowTarget<Message>,
        command: Command,
    ) -> Result<Value> {
        match command {
            Command::Status => return Ok(self.status()),
            Command::On => self.set_enabled(true)?,
            Command::Off => self.set_enabled(false)?,
            Command::Toggle => self.set_enabled(!self.settings.enabled)?,
            Command::Peek => self.gestures.peek(self.millis()),
            Command::Settings => self.show(target, true)?,
            Command::Quit => self.exit = true,
            Command::Mode(mode) => self.patch("mode", json!(mode))?,
            Command::Ignore => {
                let app = front_app(&self.read_desktop())
                    .context("There is no front application to ignore.")?;
                self.toggle_ignore(app)?;
            }
            Command::Set(key, value) => self.patch(&key, value)?,
        }
        Ok(Value::Null)
    }

    fn set_enabled(&mut self, enabled: bool) -> Result<()> {
        self.patch("enabled", json!(enabled))
    }

    fn switch_mode(&mut self) -> Result<()> {
        let mode = if self.settings.mode == Mode::Deep {
            Mode::Ambient
        } else {
            Mode::Deep
        };
        self.patch("mode", json!(mode))
    }

    fn toggle_ignore(&mut self, app: AppInfo) -> Result<()> {
        let mut settings = self.settings.clone();
        if settings.is_ignored(&app.id) {
            settings.ignored.retain(|ignored| ignored.id != app.id);
        } else {
            settings.ignored.push(app);
        }
        self.commit(settings)
    }

    fn register(&mut self, shortcuts: &Shortcuts) -> Result<()> {
        let mut proposed = Vec::new();
        for (action, accelerator) in [
            ("toggle", &shortcuts.toggle),
            ("mode", &shortcuts.mode),
            ("ignore", &shortcuts.ignore),
            ("settings", &shortcuts.settings),
            ("peek", &shortcuts.peek),
        ] {
            if accelerator.trim().is_empty() {
                continue;
            }
            let key: HotKey = accelerator
                .parse()
                .map_err(|_| anyhow!("Invalid shortcut for {action}: {accelerator}"))?;
            if key.mods.is_empty() {
                bail!("Use at least one modifier for the {action} shortcut.");
            }
            if proposed.iter().any(|(previous, _)| previous == &key) {
                bail!("Two actions use the same shortcut.");
            }
            proposed.push((key, action));
        }
        let previous = std::mem::take(&mut self.bindings);
        for (key, _) in &previous {
            let _ = self.hotkeys.unregister(*key);
        }
        for (key, action) in proposed {
            if let Err(error) = self.hotkeys.register(key) {
                for (key, _) in self.bindings.drain(..) {
                    let _ = self.hotkeys.unregister(key);
                }
                for (key, action) in previous {
                    if self.hotkeys.register(key).is_ok() {
                        self.bindings.push((key, action));
                    }
                }
                return Err(anyhow!("Could not register the {action} shortcut. {error}"));
            }
            self.bindings.push((key, action));
        }
        Ok(())
    }

    fn patch(&mut self, key: &str, value: Value) -> Result<()> {
        let mut document = serde_json::to_value(&self.settings)?;
        let object = document
            .as_object_mut()
            .context("settings are not an object")?;
        if !object.contains_key(key) {
            bail!("Unknown setting {key}");
        }
        if key == "tint" && value.as_str().and_then(parse_color).is_none() {
            bail!("Tint must use a six-digit hex color, such as #233550.");
        }
        object.insert(key.into(), value);
        let updated =
            serde_json::from_value(document).with_context(|| format!("Invalid value for {key}"))?;
        self.commit(updated)
    }

    fn commit(&mut self, mut updated: Settings) -> Result<()> {
        updated.normalize();
        if updated == self.settings && self.paths.settings.exists() {
            return Ok(());
        }
        let old_shortcuts = self.settings.shortcuts.clone();
        let new_shortcuts = updated.shortcuts != old_shortcuts;
        if new_shortcuts {
            self.register(&updated.shortcuts)?;
        }
        if let Err(error) = updated.save(&self.paths.settings) {
            if new_shortcuts {
                let _ = self.register(&old_shortcuts);
            }
            return Err(error.context("Could not save Duocle settings"));
        }
        #[cfg(target_os = "macos")]
        if updated.dim_chrome != self.settings.dim_chrome {
            self.capture_generation += 1;
            for screen in self.monitors.values_mut() {
                screen.fresh_image |= screen.image.take().is_some();
            }
        }
        self.settings = updated;
        self.notice = None;
        self.revision += 1;
        self.next_snapshot = Instant::now();
        self.next_capture = Instant::now();
        self.update_tray();
        self.publish();
        Ok(())
    }

    fn update_tray(&self) {
        if let Some(tray) = &self.tray {
            tray.toggle.set_text(if self.settings.enabled {
                "Disable focus"
            } else {
                "Enable focus"
            });
            let _ = tray.icon.set_tooltip(Some(if self.settings.enabled {
                "Duocle · Focus on"
            } else {
                "Duocle · Focus off"
            }));
            if let Ok(icon) = tray_image(self.settings.enabled) {
                #[cfg(target_os = "macos")]
                let _ = tray.icon.set_icon_with_as_template(Some(icon), true);
                #[cfg(target_os = "windows")]
                let _ = tray.icon.set_icon(Some(icon));
            }
        }
    }

    fn read_desktop(&mut self) -> Desktop {
        let settings_id = self
            .window
            .as_ref()
            .filter(|window| window.window.is_visible())
            .map(|window| platform::settings_window_id(&window.window));
        self.system.snapshot(settings_id)
    }

    fn snapshot_desktop(&mut self) -> bool {
        let next = self.read_desktop();
        let changed = next.displays != self.desktop.displays
            || next.windows != self.desktop.windows
            || next.foreground != self.desktop.foreground
            || next.foreground_app != self.desktop.foreground_app
            || next.overview != self.desktop.overview;
        self.desktop = next;
        changed
    }

    fn foreground_name(&self) -> Option<&str> {
        let id = self.desktop.foreground_app.as_deref()?;
        if id == SETTINGS_APP_ID {
            Some("Duocle")
        } else {
            self.desktop
                .apps
                .iter()
                .find(|app| app.id == id)
                .map(|app| app.name.as_str())
        }
    }

    fn status(&self) -> Value {
        json!({
            "settings": self.settings,
            "apps": self.desktop.apps,
            "foreground": self.desktop.foreground_app,
            "foreground_name": self.foreground_name(),
            "paused": self.paused || self.desktop.overview,
            "focused_window": self.desktop.foreground.and_then(|id| {
                self.desktop.windows.iter().find(|window| window.id == id)
            }).map(|window| {
                let rect = window.bounds;
                json!({"x": rect.x, "y": rect.y, "width": rect.w, "height": rect.h})
            }),
            "capture_allowed": self.can_capture,
            "capturing": self.captured,
            "login": self.launched_at_login,
            "platform": std::env::consts::OS,
            "notice": self.notice,
        })
    }

    fn publish(&mut self) {
        let Some(window) = &self.window else { return };
        if !window.window.is_visible() {
            return;
        }
        let state = self.status().to_string();
        if self.ui_state != state {
            let script = format!("if (window.duocleState) window.duocleState({state});");
            let _ = window._webview.evaluate_script(&script);
            self.ui_state = state;
        }
    }

    fn millis(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn tick(&mut self, target: &EventLoopWindowTarget<Message>) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        if now >= self.next_request {
            self.next_request = now + Duration::from_millis(125);
            for (reply, command) in self.paths.pending() {
                let result = self.execute(target, command);
                Paths::respond(&reply, result);
            }
        }

        let show_settings = self
            .window
            .as_ref()
            .is_some_and(|window| window.window.is_visible());
        if now >= self.next_capture_check && (show_settings || self.settings.enabled) {
            self.next_capture_check = now + Duration::from_secs(2);
            let allowed = platform::can_capture();
            if self.can_capture != allowed {
                self.can_capture = allowed;
                self.revision += 1;
            }
        }
        let previous_foreground = self.desktop.foreground;
        let mut layout_changed = false;
        if now >= self.next_snapshot {
            let scan_interval = if self.settings.enabled {
                if self.pointer.down { 16 } else { 45 }
            } else if show_settings || self.desktop.overview {
                500
            } else {
                850
            };
            self.next_snapshot = now + Duration::from_millis(scan_interval);
            layout_changed = self.snapshot_desktop();
            self.desktop.apps.sort_by_key(|app| app.name.to_lowercase());
            self.desktop.apps.dedup_by(|a, b| a.id == b.id);
            #[cfg(target_os = "windows")]
            if let Some(window) = &mut self.window {
                if self.desktop.overview && window.window.is_visible() {
                    window.hidden_for_overview = true;
                    window.window.set_visible(false);
                } else if !self.desktop.overview && std::mem::take(&mut window.hidden_for_overview)
                {
                    platform::show_settings_without_activating(&window.window);
                }
            }
            self.monitors.retain(|id, _| {
                self.desktop
                    .displays
                    .iter()
                    .any(|display| display.id == *id)
            });
        }
        if now.duration_since(self.last_pointer) >= Duration::from_millis(15) {
            self.last_pointer = now;
            self.pointer = self.system.pointer();
            let gesture = self.gestures.update(
                self.millis(),
                self.pointer,
                &self.desktop.displays,
                &self.settings,
            );
            self.paused = gesture.paused;
            if gesture.toggle
                && let Err(error) = self.set_enabled(!self.settings.enabled)
            {
                self.report(error);
            }
        }

        let desired = self.settings.enabled && !self.paused;
        let changed = std::mem::replace(&mut self.wants_focus, desired) != desired;
        let animate_geometry = !layout_changed
            || previous_foreground != self.desktop.foreground
            || self.planned_revision != self.revision;
        if layout_changed || changed || self.planned_revision != self.revision {
            self.planned_revision = self.revision;
            let masks = if desired {
                focus::plan(&self.desktop, &self.settings)
            } else {
                Vec::new()
            };
            let visible = !masks.is_empty();
            if visible != self.target_visible {
                self.target_visible = visible;
                self.reveal_origin = self.pointer;
            }
            if visible && masks != self.masks {
                let changed_capture_area = self.masks.len() != masks.len()
                    || masks.iter().any(|(id, mask)| {
                        self.masks
                            .iter()
                            .find(|(current, _)| current == id)
                            .is_none_or(|(_, old)| old.bounds != mask.bounds)
                    });
                if changed_capture_area {
                    self.capture_generation += 1;
                    for (id, mask) in &masks {
                        if self
                            .masks
                            .iter()
                            .find(|(current, _)| current == id)
                            .is_none_or(|(_, old)| old.bounds != mask.bounds)
                            && let Some(screen) = self.monitors.get_mut(id)
                        {
                            screen.image = None;
                            screen.fresh_image = true;
                        }
                    }
                }
                self.masks = masks;
            }
        }
        let visible = self.target_visible;
        let step = elapsed * 1000.0 / f32::from(self.settings.transition.max(1));
        self.opacity = if self.desktop.overview {
            0.0
        } else if self.settings.transition == 0 {
            if visible { 1.0 } else { 0.0 }
        } else if visible {
            (self.opacity + step).min(1.0)
        } else {
            (self.opacity - step).max(0.0)
        };
        self.refresh_capture(now, visible);
        if self.opacity == 0.0 {
            self.masks.clear();
        }
        self.present(now, animate_geometry);
        self.publish();
    }

    fn refresh_capture(&mut self, now: Instant, visible: bool) {
        let capture = visible && self.can_capture && self.settings.uses_capture();
        if let Some(receiver) = &self.capture_rx {
            match receiver.try_recv() {
                Ok((generation, frames)) => {
                    self.capture_rx = None;
                    if capture && generation == self.capture_generation {
                        for (id, size, frame) in frames {
                            if self.masks.iter().any(|(display, mask)| {
                                *display == id && Size::for_mask(mask) == size
                            }) {
                                let screen = self.monitors.entry(id).or_default();
                                screen.image = frame;
                                screen.fresh_image = true;
                            }
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => self.capture_rx = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        self.captured = capture && self.monitors.values().any(|screen| screen.image.is_some());
        if !capture {
            if self.opacity == 0.0 || !self.settings.uses_capture() || !self.can_capture {
                for screen in self.monitors.values_mut() {
                    screen.fresh_image |= screen.image.take().is_some();
                }
            }
            return;
        }
        if now >= self.next_capture && self.capture_rx.is_none() {
            let jobs: Vec<_> = self
                .masks
                .iter()
                .filter_map(|(id, mask)| {
                    let display = self
                        .desktop
                        .displays
                        .iter()
                        .find(|display| display.id == *id)?
                        .clone();
                    Some(CaptureJob {
                        display,
                        size: Size::for_mask(mask),
                        bounds: mask.bounds,
                        #[cfg(target_os = "macos")]
                        chrome: self.settings.dim_chrome,
                    })
                })
                .collect();
            let (sender, receiver) = mpsc::channel();
            let generation = self.capture_generation;
            self.capture_rx = Some(receiver);
            self.next_capture = now + Duration::from_millis(115);
            thread::spawn(move || {
                let _ = sender.send((generation, platform::capture(jobs)));
            });
        }
    }

    fn present(&mut self, now: Instant, animate_geometry: bool) {
        let opacity = focus::ease(self.opacity);
        let duration = Duration::from_millis(u64::from(self.settings.transition));
        for (id, _) in &self.masks {
            self.monitors.entry(*id).or_default();
        }
        for (id, screen) in &mut self.monitors {
            let mask = self
                .masks
                .iter()
                .find(|(live, _)| live == id)
                .map(|(_, mask)| mask);
            let empty;
            let mask = match mask {
                Some(mask) => mask,
                None if opacity > 0.0 => {
                    let Some(coverage) = &screen.coverage else {
                        continue;
                    };
                    empty = Mask::transparent(coverage.target.bounds);
                    &empty
                }
                None => {
                    screen.hide();
                    continue;
                }
            };
            let (mask_changed, geometry_changed) = match screen.coverage.as_mut() {
                Some(coverage) => {
                    let mask_changed = coverage.target != *mask;
                    let duration = if mask_changed && !animate_geometry {
                        // Follow the same focused window as it moves or resizes.
                        Duration::ZERO
                    } else {
                        duration
                    };
                    (mask_changed, coverage.update(mask, now, duration))
                }
                None => {
                    screen.coverage = Some(Coverage::new(mask.clone(), now));
                    (true, true)
                }
            };
            let coverage = screen.coverage.as_ref().unwrap();
            if coverage.tiles.is_empty() {
                screen.hide();
                continue;
            }
            let progress = coverage.progress(now);
            let progress_changed = screen.mask_progress != progress;
            let size = Size::for_mask(mask);
            let cursor_reveal = self.settings.reveal == Reveal::Cursor && opacity < 1.0;
            let changed_opacity = screen.opacity != opacity;
            let repaint = mask_changed
                || screen.painted_revision != self.revision
                || screen.fresh_image
                || (self.settings.reveal == Reveal::Cursor && changed_opacity);
            if !geometry_changed && !repaint && !progress_changed && !changed_opacity {
                continue;
            }
            let reveal = cursor_reveal.then(|| {
                let pointer = self.reveal_origin;
                let bounds = mask.bounds;
                let dx = (pointer.x - bounds.x)
                    .abs()
                    .max((pointer.x - bounds.right()).abs());
                let dy = (pointer.y - bounds.y)
                    .abs()
                    .max((pointer.y - bounds.bottom()).abs());
                let max_radius = (dx as f32).hypot(dy as f32);
                (
                    pointer.x,
                    pointer.y,
                    (1.0 - opacity) * (max_radius + 36.0) - 36.0,
                )
            });
            let image = screen.image.as_ref();
            if repaint {
                let pixels = effect::render(mask, &self.settings, size, image, reveal);
                if mask_changed {
                    screen.texture_from = (self.settings.mode == Mode::Ambient
                        && progress < 1.0
                        && pixels.len() == screen.texture.len())
                    .then(|| screen.texture.clone());
                }
                screen.texture_to = pixels;
            }
            let blending = screen.texture_from.is_some();
            let pixels_changed = if let Some(from) = &screen.texture_from
                && progress < 1.0
                && from.len() == screen.texture_to.len()
            {
                screen.texture.resize(from.len(), 0);
                let weight = (progress * 256.0).round() as u16;
                for ((pixel, old), new) in
                    screen.texture.iter_mut().zip(from).zip(&screen.texture_to)
                {
                    *pixel = ((u16::from(*old) * (256 - weight) + u16::from(*new) * weight + 128)
                        >> 8) as u8;
                }
                true
            } else {
                screen.texture_from = None;
                if repaint || blending {
                    screen.texture.clone_from(&screen.texture_to);
                }
                repaint || blending
            };
            #[cfg(target_os = "macos")]
            let native_blur_radius = if image.is_none() {
                effect::mac_blur_radius(&self.settings)
            } else {
                0.0
            };
            let overlay = match &mut screen.overlay {
                Some(overlay) => overlay,
                None => match Overlay::new() {
                    Ok(overlay) => screen.overlay.insert(overlay),
                    Err(error) => {
                        screen.coverage = None;
                        self.notice =
                            Some(format!("Could not create a desktop overlay. {error:#}"));
                        continue;
                    }
                },
            };
            let frame = OverlayFrame {
                bounds: mask.bounds,
                tiles: &coverage.tiles,
                geometry_changed,
                progress,
                size,
                pixels: pixels_changed.then_some(&screen.texture),
                opacity: if cursor_reveal { 1.0 } else { opacity },
                #[cfg(target_os = "macos")]
                native_blur_radius,
                #[cfg(target_os = "macos")]
                native_blur_opacity: if cursor_reveal { opacity } else { 1.0 },
                #[cfg(target_os = "macos")]
                chrome: self.settings.dim_chrome,
            };
            match overlay.present(frame) {
                Ok(()) => {
                    screen.opacity = opacity;
                    screen.mask_progress = progress;
                    screen.painted_revision = self.revision;
                    screen.fresh_image = false;
                }
                Err(error) => {
                    screen.hide();
                    self.notice = Some(format!("Could not update a desktop overlay. {error:#}"));
                }
            }
        }
    }
}

fn front_app(desktop: &Desktop) -> Option<AppInfo> {
    let id = desktop
        .foreground_app
        .as_deref()
        .filter(|id| *id != SETTINGS_APP_ID && !desktop.overview)?;
    Some(
        desktop
            .apps
            .iter()
            .find(|app| app.id == id)
            .cloned()
            .unwrap_or_else(|| AppInfo {
                id: id.into(),
                name: id.into(),
            }),
    )
}

fn settings_html(titlebar_inset: u32) -> String {
    let styles = include_str!("../ui/material.css")
        .replace("DUOCLE_TITLEBAR_INSET", &format!("{titlebar_inset}px"))
        .replace(
            "DUOCLE_ROBOTO_FONT",
            &STANDARD.encode(include_bytes!("../ui/fonts/roboto-latin.woff2")),
        )
        .replace(
            "DUOCLE_SYMBOL_FONT",
            &STANDARD.encode(include_bytes!("../ui/fonts/material-symbols.woff2")),
        );
    include_str!("../ui/index.html").replace("/* DUOCLE_STYLE */", &styles)
}

fn tray_image(active: bool) -> Result<Icon> {
    let size = 32;
    let mut rgba = Vec::with_capacity(size * size * 4);
    let ink = if cfg!(target_os = "macos") {
        [0, 0, 0]
    } else if active {
        [108, 231, 203]
    } else {
        [187, 199, 220]
    };
    for row in 0..size {
        for column in 0..size {
            let x = column as f32 + 0.5;
            let y = row as f32 + 0.5;
            let outer = ((x - 14.0) / 1.14).hypot(y - 16.0);
            let inner = ((x - 20.0) / 1.10).hypot(y - 16.0);
            let dot = (x - 17.0).hypot(y - 16.0);
            let edge = (outer - 9.1).abs().min((inner - 5.5).abs());
            let alpha = if active && dot <= 1.8 {
                1.0
            } else {
                (1.9 - edge).clamp(0.0, 1.0)
            };
            rgba.extend([ink[0], ink[1], ink[2], (alpha * 255.0).round() as u8]);
        }
    }
    Icon::from_rgba(rgba, size as u32, size as u32).map_err(Into::into)
}
