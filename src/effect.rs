use crate::{
    config::{Mode, Settings, parse_color},
    focus::{Mask, ease},
};
use image::{ImageBuffer, RgbImage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub fn for_mask(mask: &Mask) -> Self {
        let width = mask.bounds.w.clamp(1, 640) as u32;
        let height = (i64::from(width) * i64::from(mask.bounds.h) / i64::from(mask.bounds.w.max(1)))
            .clamp(1, 1_200) as u32;
        Self { width, height }
    }

    pub fn pixels(self) -> usize {
        self.width as usize * self.height as usize
    }
}

#[cfg(target_os = "macos")]
pub const MAX_MAC_BLUR_RADIUS: f32 = 125.0;

#[cfg(target_os = "macos")]
pub fn mac_blur_radius(settings: &Settings) -> f32 {
    let radius = f32::from(settings.blur) / 100.0 * MAX_MAC_BLUR_RADIUS;
    if settings.mode == Mode::Ambient {
        radius * 0.85
    } else {
        radius
    }
}

/// Renders premultiplied BGRA. Native masks handle the crisp edges of clear
/// windows independently of the resolution used for the filtered background.
pub fn render(
    mask: &Mask,
    settings: &Settings,
    size: Size,
    image: Option<&RgbImage>,
    reveal: Option<(i32, i32, f32)>,
) -> Vec<u8> {
    let image = image.filter(|image| image.dimensions() == (size.width, size.height));
    let softened = image.filter(|_| settings.blur > 0).map(|image| {
        #[cfg(target_os = "macos")]
        let sigma = mac_blur_radius(settings) * size.width as f32 / mask.bounds.w.max(1) as f32;
        #[cfg(not(target_os = "macos"))]
        let strength = f32::from(settings.blur) / 100.0;
        #[cfg(not(target_os = "macos"))]
        let sigma = 0.8 + 13.0 * strength * strength;
        image::imageops::fast_blur(image, sigma)
    });
    let continuous_blur = cfg!(target_os = "macos") && softened.is_some();
    let filtered = softened.as_ref().or(image);
    let [tr, tg, tb] = parse_color(&settings.tint)
        .unwrap_or([18, 27, 41])
        .map(f32::from);
    let tint = f32::from(settings.tint_strength) / 100.0;
    let gray = f32::from(settings.monochrome) / 100.0;
    let grain = f32::from(settings.grain) / 100.0 * 36.0;
    let x_step = mask.bounds.w as f32 / size.width as f32;
    let y_step = mask.bounds.h as f32 / size.height as f32;
    let falloff = (mask.bounds.w.min(mask.bounds.h) as f32 * 0.24).max(1.0);
    let mut pixels = Vec::with_capacity(size.pixels() * 4);

    for y in 0..size.height {
        for x in 0..size.width {
            let px = mask.bounds.x as f32 + (x as f32 + 0.5) * x_step;
            let py = mask.bounds.y as f32 + (y as f32 + 0.5) * y_step;
            let ambient = if settings.mode == Mode::Ambient {
                let distance = mask
                    .ambient
                    .iter()
                    .map(|rect| {
                        let dx = (rect.x as f32 - px).max(px - rect.right() as f32).max(0.0);
                        let dy = (rect.y as f32 - py).max(py - rect.bottom() as f32).max(0.0);
                        dx * dx + dy * dy
                    })
                    .fold(f32::INFINITY, f32::min)
                    .sqrt();
                0.30 + 0.58 * ease(distance / falloff)
            } else {
                1.0
            };
            let aperture = reveal.map_or(1.0, |(cx, cy, radius)| {
                ease((px - cx as f32).hypot(py - cy as f32) / 36.0 - radius / 36.0)
            });
            let random = (x.wrapping_mul(0x37d2_ee59) ^ y.wrapping_mul(0x9e37_79b9))
                .wrapping_mul(0x85eb_ca6b);
            let noise = ((random >> 24) as f32 / 255.0 - 0.5) * grain;
            let (red, green, blue, alpha) = if let Some(img) = filtered {
                // Ambient varies the tint over the blur. Blending the capture
                // with the live desktop would bring sharp text back into focus.
                let (tint, noise, opacity) = if continuous_blur {
                    (tint * ambient, noise * ambient, 1.0)
                } else {
                    (tint, noise, ambient)
                };
                let [r, g, b] = img.get_pixel(x, y).0.map(f32::from);
                let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                let mix = |color: f32, tint_color: f32| {
                    ((color * (1.0 - gray) + luminance * gray) * (1.0 - tint)
                        + tint_color * tint
                        + noise)
                        .clamp(0.0, 255.0)
                };
                (mix(r, tr), mix(g, tg), mix(b, tb), opacity * aperture)
            } else {
                // Grain is translucent over the live desktop, even with tint disabled.
                let noise_alpha = noise.abs() / 72.0;
                let alpha = tint + (1.0 - tint) * noise_alpha;
                let noise_color = if noise.is_sign_positive() { 255.0 } else { 0.0 };
                let mix = |color: f32| {
                    if alpha == 0.0 {
                        0.0
                    } else {
                        (color * tint + noise_color * (1.0 - tint) * noise_alpha) / alpha
                    }
                };
                (mix(tr), mix(tg), mix(tb), alpha * ambient * aperture)
            };
            pixels.extend([
                (blue * alpha).round() as u8,
                (green * alpha).round() as u8,
                (red * alpha).round() as u8,
                (255.0 * alpha).round() as u8,
            ]);
        }
    }
    pixels
}

pub fn image_from_bgra(size: Size, bgra: &[u8]) -> Option<RgbImage> {
    let rgb = bgra
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[2], p[1], p[0]])
        .collect();
    ImageBuffer::from_raw(size.width, size.height, rgb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::Rect;

    #[cfg(target_os = "macos")]
    #[test]
    fn ambient_blur_has_full_coverage_and_graduated_tint() {
        let mask = Mask {
            bounds: Rect::new(0, 0, 16, 16),
            clear: vec![],
            ambient: vec![Rect::new(0, 0, 8, 8)],
        };
        let settings = Settings {
            mode: Mode::Ambient,
            ..Settings::default()
        };
        let size = Size::for_mask(&mask);
        let desktop = RgbImage::from_pixel(size.width, size.height, image::Rgb([255, 255, 255]));
        let pixels = render(&mask, &settings, size, Some(&desktop), None);
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 255)
        );
        assert!(pixels[2] > pixels[pixels.len() - 2]);
    }

    #[test]
    fn tint_is_premultiplied_and_monochrome_removes_color() {
        let mask = Mask {
            bounds: Rect::new(0, 0, 1, 1),
            clear: vec![],
            ambient: vec![],
        };
        let size = Size::for_mask(&mask);
        let mut settings = Settings {
            blur: 0,
            tint: "#FF8000".into(),
            tint_strength: 50,
            grain: 0,
            ..Settings::default()
        };
        assert_eq!(
            render(&mask, &settings, size, None, None),
            [0, 64, 128, 128]
        );
        settings.tint_strength = 0;
        settings.monochrome = 100;
        let image = image_from_bgra(size, &[0, 0, 255, 255]).unwrap();
        assert_eq!(
            render(&mask, &settings, size, Some(&image), None),
            [54, 54, 54, 255]
        );
    }
}
