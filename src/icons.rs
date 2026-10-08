//! The apps' own icons, as shown on getartcraft.com (and, for apps the site
//! doesn't list yet, in their repositories). Built-in apps' icons are
//! embedded so they show offline; apps found later are downloaded to
//! `<data>\icons` when discovered.

use eframe::egui;
use std::collections::HashMap;
use std::path::Path;

fn embedded(id: &str) -> Option<&'static [u8]> {
    Some(match id {
        "photocraft" => include_bytes!("../assets/app-icons/photocraft.png"),
        "lightcraft" => include_bytes!("../assets/app-icons/lightcraft.png"),
        "vectorcraft" => include_bytes!("../assets/app-icons/vectorcraft.png"),
        "designcraft" => include_bytes!("../assets/app-icons/designcraft.png"),
        "filmcraft" => include_bytes!("../assets/app-icons/filmcraft.png"),
        "effectcraft" => include_bytes!("../assets/app-icons/effectcraft.png"),
        "soundcraft" => include_bytes!("../assets/app-icons/soundcraft.png"),
        "pdfcraft" => include_bytes!("../assets/app-icons/pdfcraft.png"),
        "wordcraft" => include_bytes!("../assets/app-icons/wordcraft.png"),
        "gridcraft" => include_bytes!("../assets/app-icons/gridcraft.png"),
        "deckcraft" => include_bytes!("../assets/app-icons/deckcraft.png"),
        "cadcraft" => include_bytes!("../assets/app-icons/cadcraft.png"),
        _ => return None,
    })
}

/// Where a discovered app's icon is saved.
pub fn downloaded_path(data_dir: &Path, id: &str) -> std::path::PathBuf {
    data_dir.join("icons").join(format!("{id}.png"))
}

#[derive(Default)]
pub struct IconCache {
    textures: HashMap<String, egui::TextureHandle>,
}

impl IconCache {
    pub fn get(&mut self, ctx: &egui::Context, id: &str, data_dir: &Path) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.textures.get(id) {
            return Some(texture.clone());
        }
        let bytes = match embedded(id) {
            Some(bytes) => bytes.to_vec(),
            None => std::fs::read(downloaded_path(data_dir, id)).ok()?,
        };
        let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?.to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let texture = ctx.load_texture(
            format!("app-icon-{id}"),
            egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
            egui::TextureOptions::LINEAR,
        );
        self.textures.insert(id.to_owned(), texture.clone());
        Some(texture)
    }
}
