use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Ambient,
    #[default]
    Deep,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Displays {
    #[default]
    PerDisplay,
    Global,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    Off,
    #[default]
    None,
    Shift,
    Control,
    Alt,
    Super,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    #[default]
    Off,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reveal {
    #[default]
    Fade,
    Cursor,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Group {
    pub name: String,
    pub apps: Vec<AppInfo>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Shortcuts {
    pub toggle: String,
    pub mode: String,
    pub ignore: String,
    pub settings: String,
    pub peek: String,
}

impl Default for Shortcuts {
    fn default() -> Self {
        let modifiers = if cfg!(target_os = "macos") {
            "Ctrl+Alt+Super"
        } else {
            "Ctrl+Alt+Shift"
        };
        Self {
            toggle: format!("{modifiers}+F"),
            mode: format!("{modifiers}+M"),
            ignore: format!("{modifiers}+E"),
            settings: format!("{modifiers}+C"),
            peek: String::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub mode: Mode,
    pub blur: u8,
    pub tint: String,
    pub tint_strength: u8,
    pub grain: u8,
    pub monochrome: u8,
    pub transition: u16,
    pub reveal: Reveal,
    pub app_wide: bool,
    pub displays: Displays,
    pub ignored: Vec<AppInfo>,
    pub groups: Vec<Group>,
    pub shake: bool,
    pub shake_toggle: Modifier,
    pub shake_peek: Modifier,
    pub sensitivity: u8,
    pub corner: Corner,
    pub pause_dragging: bool,
    pub click_toggles: bool,
    pub cover_stage: bool,
    pub dim_chrome: bool,
    pub precise_effects: bool,
    pub shortcuts: Shortcuts,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: Mode::Deep,
            blur: 50,
            tint: "#121B29".into(),
            tint_strength: 25,
            grain: 5,
            monochrome: 0,
            transition: 240,
            reveal: Reveal::Fade,
            app_wide: false,
            displays: Displays::PerDisplay,
            ignored: if cfg!(target_os = "macos") {
                vec![AppInfo {
                    id: "com.apple.finder".into(),
                    name: "Finder".into(),
                }]
            } else {
                Vec::new()
            },
            groups: Vec::new(),
            shake: true,
            shake_toggle: Modifier::None,
            shake_peek: Modifier::Shift,
            sensitivity: 50,
            corner: Corner::Off,
            pause_dragging: true,
            click_toggles: false,
            cover_stage: false,
            dim_chrome: false,
            precise_effects: false,
            shortcuts: Shortcuts::default(),
        }
    }
}

impl Settings {
    pub fn normalize(&mut self) {
        self.blur = self.blur.min(100);
        self.tint_strength = self.tint_strength.min(100);
        self.grain = self.grain.min(100);
        self.monochrome = self.monochrome.min(100);
        self.transition = self.transition.min(1_000);
        self.sensitivity = self.sensitivity.min(100);
        if parse_color(&self.tint).is_none() {
            self.tint = Self::default().tint;
        }
        self.ignored.sort_by(|a, b| a.id.cmp(&b.id));
        self.ignored.dedup_by(|a, b| a.id == b.id);
        for group in &mut self.groups {
            group.apps.sort_by(|a, b| a.id.cmp(&b.id));
            group.apps.dedup_by(|a, b| a.id == b.id);
        }
        self.groups.retain(|group| group.apps.len() > 1);
    }

    pub fn is_ignored(&self, id: &str) -> bool {
        self.ignored.iter().any(|app| app.id == id)
    }

    pub fn uses_capture(&self) -> bool {
        (self.blur > 0 || self.monochrome > 0)
            && (!cfg!(target_os = "macos") || self.precise_effects)
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let mut settings: Self = match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(error.into()),
        };
        settings.normalize();
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        write_atomic(path, &serde_json::to_vec_pretty(self)?)
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Path has no parent"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.persist(path)?;
    Ok(())
}

pub fn parse_color(input: &str) -> Option<[u8; 3]> {
    let input = input.strip_prefix('#')?;
    if input.len() != 6 || !input.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&input[0..2], 16).ok()?,
        u8::from_str_radix(&input[2..4], 16).ok()?,
        u8::from_str_radix(&input[4..6], 16).ok()?,
    ])
}
