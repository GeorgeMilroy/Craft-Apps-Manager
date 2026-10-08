//! The app's logo: the ArtCraft "A" mark, taken from the ArtCraft app's own
//! icon. Used for the window, the tray and notifications. The .exe gets the
//! same .ico from `build.rs`, and the installer from `installer\`.

use image::{imageops, ImageFormat, RgbaImage};
use std::sync::OnceLock;

/// The multi-size Windows icon (16 to 256 px).
pub const ICO: &[u8] = include_bytes!("../assets/logo.ico");
const PNG: &[u8] = include_bytes!("../assets/logo.png");

const BADGE: [f32; 3] = [249.0, 115.0, 22.0];

fn source() -> &'static RgbaImage {
    static IMAGE: OnceLock<RgbaImage> = OnceLock::new();
    IMAGE.get_or_init(|| {
        image::load_from_memory_with_format(PNG, ImageFormat::Png)
            .expect("embedded logo is a valid PNG")
            .to_rgba8()
    })
}

/// Straight-alpha RGBA pixels of the logo at `size`×`size`. `badge` adds an
/// orange dot in the top-right corner, which the tray uses to flag updates.
pub fn rgba(size: u32, badge: bool) -> Vec<u8> {
    let mut image = imageops::resize(source(), size, size, imageops::FilterType::Lanczos3);
    if badge {
        add_badge(&mut image);
    }
    image.into_raw()
}

fn add_badge(image: &mut RgbaImage) {
    let s = image.width() as f32;
    let (cx, cy) = (0.79 * s, 0.21 * s);
    for (x, y, px) in image.enumerate_pixels_mut() {
        let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
        // Coverage of a circle of radius r (in units of the icon size), antialiased.
        let cover = |r: f32| (0.5 - (d - r * s)).clamp(0.0, 1.0);
        // Clear a ring around the dot so it stands apart from the logo, then draw the dot.
        let a = px[3] as f32 / 255.0 * (1.0 - cover(0.25));
        let dot = cover(0.19);
        let out_a = dot + a * (1.0 - dot);
        if out_a > 0.0 {
            for c in 0..3 {
                px[c] = ((BADGE[c] * dot + px[c] as f32 * a * (1.0 - dot)) / out_a).round() as u8;
            }
        }
        px[3] = (out_a * 255.0).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_with_and_without_badge() {
        let plain = rgba(32, false);
        let badged = rgba(32, true);
        assert_eq!(plain.len(), 32 * 32 * 4);
        // The badge's centre (about 25, 7) is solid orange; the plain logo is
        // all but clear there (resizing leaves a faint edge).
        let at = |px: &[u8]| {
            let i = (7 * 32 + 25) * 4;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        assert_eq!(at(&badged), [249, 115, 22, 255]);
        assert!(at(&plain)[3] < 16);
    }
}
