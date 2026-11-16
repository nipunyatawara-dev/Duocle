use crate::{
    effect::Size,
    focus::{Display, Rect, Tile},
};
use image::RgbImage;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::*;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;

pub const SETTINGS_APP_ID: &str = "app.duocle.desktop";

pub struct CaptureJob {
    pub display: Display,
    pub size: Size,
    pub bounds: Rect,
}

pub type Frame = (u64, Size, Option<RgbImage>);
pub type CaptureResult = (u64, Vec<Frame>);

pub struct OverlayFrame<'a> {
    pub bounds: Rect,
    pub tiles: &'a [Tile],
    pub geometry_changed: bool,
    pub progress: f32,
    pub size: Size,
    pub pixels: Option<&'a [u8]>,
    pub opacity: f32,
    #[cfg(target_os = "macos")]
    pub native_blur: f32,
    #[cfg(target_os = "macos")]
    pub chrome: bool,
}
