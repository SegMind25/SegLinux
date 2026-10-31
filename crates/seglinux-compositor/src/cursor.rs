//! The mouse pointer images from the XCursor theme (XCURSOR_THEME, the
//! session's Breeze or Adwaita): the arrow, and the named shapes the
//! compositor (resize edges, moving) and applications (cursor-shape) ask
//! for, at the screen's scale. Loading from Smithay's anvil (MIT).

use std::{collections::HashMap, io::Read, rc::Rc, time::Duration};

use smithay::input::pointer::CursorIcon;

use tracing::warn;
use xcursor::{
    CursorTheme,
    parser::{Image, parse_xcursor},
};

static FALLBACK_CURSOR_DATA: &[u8] = include_bytes!("../resources/cursor.rgba");

pub struct Cursor {
    theme: CursorTheme,
    size: u32,
    /// Each shape asked for so far (none: the theme lacks it).
    icons: HashMap<CursorIcon, Option<Rc<Vec<Image>>>>,
    /// The theme's arrow, else a built-in one.
    default: Rc<Vec<Image>>,
}

impl Cursor {
    pub fn load() -> Cursor {
        let name = std::env::var("XCURSOR_THEME")
            .ok()
            .unwrap_or_else(|| "breeze_cursors".into());
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(24);

        let theme = CursorTheme::load(&name);
        let default = ["default", "left_ptr"]
            .iter()
            .find_map(|n| load_icon(&theme, n).ok())
            .unwrap_or_else(|| {
                warn!("cursor theme `{name}` has no arrow; using the built-in one");
                vec![Image {
                    size: 32,
                    width: 64,
                    height: 64,
                    xhot: 1,
                    yhot: 1,
                    delay: 1,
                    pixels_rgba: Vec::from(FALLBACK_CURSOR_DATA),
                    pixels_argb: vec![], //unused
                }]
            });

        Cursor {
            theme,
            size,
            icons: HashMap::new(),
            default: Rc::new(default),
        }
    }

    /// The image of `icon` (by its name, then its other names, else the
    /// arrow) for a screen at `scale`, at `time` for animated ones, and
    /// which frame it is.
    pub fn image(&mut self, icon: CursorIcon, scale: u32, time: Duration) -> (usize, Image) {
        let theme = &self.theme;
        let images = self
            .icons
            .entry(icon)
            .or_insert_with(|| {
                std::iter::once(icon.name())
                    .chain(icon.alt_names().iter().copied())
                    .find_map(|n| load_icon(theme, n).ok())
                    .map(Rc::new)
            })
            .clone()
            .unwrap_or_else(|| self.default.clone());
        frame(time.as_millis() as u32, self.size * scale.max(1), &images)
    }
}

fn nearest_images(size: u32, images: &[Image]) -> impl Iterator<Item = &Image> {
    // Follow the nominal size of the cursor to choose the nearest
    let nearest_image = images
        .iter()
        .min_by_key(|image| (size as i32 - image.size as i32).abs())
        .unwrap();

    images.iter().filter(move |image| {
        image.width == nearest_image.width && image.height == nearest_image.height
    })
}

fn frame(mut millis: u32, size: u32, images: &[Image]) -> (usize, Image) {
    let total = nearest_images(size, images).fold(0, |acc, image| acc + image.delay);
    if total == 0 {
        return (0, nearest_images(size, images).next().unwrap().clone());
    }
    millis %= total;

    for (i, img) in nearest_images(size, images).enumerate() {
        if millis < img.delay {
            return (i, img.clone());
        }
        millis -= img.delay;
    }

    unreachable!()
}

#[derive(Debug)]
enum Error {
    NoDefaultCursor,
    File(std::io::Error),
    Parse,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoDefaultCursor => write!(f, "the theme has no default cursor"),
            Error::File(e) => write!(f, "cannot open the cursor file: {e}"),
            Error::Parse => write!(f, "cannot read the cursor file"),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::File(e)
    }
}

fn load_icon(theme: &CursorTheme, name: &str) -> Result<Vec<Image>, Error> {
    let icon_path = theme.load_icon(name).ok_or(Error::NoDefaultCursor)?;
    let mut cursor_file = std::fs::File::open(icon_path)?;
    let mut cursor_data = Vec::new();
    cursor_file.read_to_end(&mut cursor_data)?;
    parse_xcursor(&cursor_data).ok_or(Error::Parse)
}
