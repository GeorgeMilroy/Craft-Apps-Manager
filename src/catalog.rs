//! The apps this manager knows about. The built-in list covers every Craft
//! app in the storytold organisation that publishes Windows builds; new ones
//! are picked up at runtime by `check::discover`.

use serde::{Deserialize, Serialize};

pub const OWNER: &str = "storytold";

/// `*craft` repositories that aren't offered for install. ArtCraft is left
/// out on purpose: its logo is this manager's logo.
pub const EXCLUDED: &[&str] = &["artcraft"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppInfo {
    /// GitHub repository name. Also the stem of the app's executable.
    pub id: String,
    /// Product name, as used by the MSI and the install folder.
    pub name: String,
    pub description: String,
    /// Short label, as on getartcraft.com ("Image editor").
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub colors: AppColors,
    #[serde(default)]
    pub homepage: Option<String>,
    /// Earlier product names, so older installs are still recognised.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Found by scanning the GitHub organisation rather than built in.
    #[serde(default)]
    pub discovered: bool,
}

/// An app's brand colours from getartcraft.com: the colour of its icon tile,
/// and the text colours the site uses for it on light and dark backgrounds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppColors {
    pub tile: [u8; 3],
    pub ink_light: [u8; 3],
    pub ink_dark: [u8; 3],
}

impl Default for AppColors {
    /// The site's accent blue.
    fn default() -> Self {
        AppColors::new(0x2d81ff, 0x1659c4, 0x74aaff)
    }
}

impl AppColors {
    const fn new(tile: u32, ink_light: u32, ink_dark: u32) -> Self {
        const fn rgb(c: u32) -> [u8; 3] {
            [(c >> 16) as u8, (c >> 8) as u8, c as u8]
        }
        AppColors { tile: rgb(tile), ink_light: rgb(ink_light), ink_dark: rgb(ink_dark) }
    }

    pub fn ink(&self, dark: bool) -> [u8; 3] {
        if dark {
            self.ink_dark
        } else {
            self.ink_light
        }
    }
}

impl AppInfo {
    pub fn repo_url(&self) -> String {
        format!("https://github.com/{OWNER}/{}", self.id)
    }

    /// The product name followed by any former names.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.name.as_str()).chain(self.aliases.iter().map(String::as_str))
    }

    /// An entry for a repository found at runtime.
    pub fn discovered(id: &str, name: &str, description: &str, homepage: Option<String>) -> Self {
        AppInfo {
            id: id.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            category: String::new(),
            colors: AppColors::default(),
            homepage: homepage.filter(|h| !h.is_empty()),
            aliases: Vec::new(),
            discovered: true,
        }
    }
}

pub fn builtin() -> Vec<AppInfo> {
    let app = |id: &str, name: &str, category: &str, colors: AppColors, description: &str, page: Option<&str>| AppInfo {
        id: id.to_owned(),
        name: name.to_owned(),
        description: description.to_owned(),
        category: category.to_owned(),
        colors,
        homepage: page.map(|p| format!("https://getartcraft.com{p}")),
        aliases: Vec::new(),
        discovered: false,
    };
    // Colours for the first seven apps are the site's; the rest come from their icons.
    let mut pdfcraft = app(
        "pdfcraft",
        "PdfCraft",
        "PDF editor",
        AppColors::new(0x12a58a, 0x0a7563, 0x3fd6b8),
        "PDF viewing and editing. A clean-room Adobe Acrobat alternative.",
        Some("/apps/pdfcraft"),
    );
    pdfcraft.aliases.push("PrintCraft".to_owned());

    vec![
        app(
            "photocraft",
            "PhotoCraft",
            "Image editor",
            AppColors::new(0x2f7bf5, 0x1a5bc4, 0x7fb0ff),
            "Image editing. A clean-room Adobe Photoshop alternative.",
            Some("/apps/photocraft"),
        ),
        app(
            "vectorcraft",
            "VectorCraft",
            "Vector illustration",
            AppColors::new(0xe8573f, 0xb83a24, 0xff8a70),
            "Vector illustration. A clean-room Adobe Illustrator alternative.",
            Some("/apps/vectorcraft"),
        ),
        app(
            "filmcraft",
            "FilmCraft",
            "Video editor",
            AppColors::new(0x8b5cf6, 0x6a3fd6, 0xb69cff),
            "Video editing. A clean-room Adobe Premiere Pro alternative.",
            Some("/apps/filmcraft"),
        ),
        app(
            "lightcraft",
            "LightCraft",
            "Photo library & raw developer",
            AppColors::new(0xf2a516, 0x8a5800, 0xffc14d),
            "Photo library and RAW development. A clean-room Adobe Lightroom alternative.",
            Some("/apps/lightcraft"),
        ),
        pdfcraft,
        app(
            "effectcraft",
            "EffectCraft",
            "Motion graphics & VFX",
            AppColors::new(0xe0368f, 0xb0206c, 0xff7ab8),
            "Motion graphics and visual effects. A clean-room Adobe After Effects alternative.",
            Some("/apps/effectcraft"),
        ),
        app(
            "designcraft",
            "DesignCraft",
            "Page layout",
            AppColors::new(0x7bb51c, 0x457008, 0xa6e04a),
            "Page layout and publishing. A clean-room Adobe InDesign alternative.",
            Some("/apps/designcraft"),
        ),
        app(
            "soundcraft",
            "SoundCraft",
            "Audio workstation",
            AppColors::new(0x14a9c4, 0x0b6f82, 0x5fd3e8),
            "Audio recording, editing and mixing. A clean-room Avid Pro Tools alternative.",
            None,
        ),
        app(
            "wordcraft",
            "WordCraft",
            "Word processor",
            AppColors::new(0x3b5bdb, 0x2a44b0, 0x8ea2ff),
            "Word processing. A clean-room Microsoft Word alternative.",
            None,
        ),
        app(
            "gridcraft",
            "GridCraft",
            "Spreadsheets",
            AppColors::new(0x1f9d55, 0x13703b, 0x5fd98f),
            "Spreadsheets. A clean-room Microsoft Excel alternative.",
            None,
        ),
        app(
            "deckcraft",
            "DeckCraft",
            "Presentations",
            AppColors::new(0xf26b1d, 0xb5470a, 0xff9a5c),
            "Presentations and slide shows. A clean-room Microsoft PowerPoint alternative.",
            None,
        ),
        app(
            "cadcraft",
            "CADCraft",
            "CAD & drafting",
            AppColors::new(0x14a2c5, 0x0b6c85, 0x5ccbe8),
            "Computer-aided design and drafting. A clean-room AutoCAD alternative.",
            None,
        ),
    ]
}
