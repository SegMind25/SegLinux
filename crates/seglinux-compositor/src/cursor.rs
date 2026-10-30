//! The mouse pointer image from the XCursor theme (from Smithay's anvil, MIT).

use std::{io::Read, time::Duration};

use tracing::warn;
use xcursor::{
    CursorTheme,
    parser::{Image, parse_xcursor},
};

static FALLBACK_CURSOR_DATA: &[u8] = include_bytes!("../resources/cursor.rgba");

pub struct Cursor {
    icons: Vec<Image>,
    size: u32,
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
        let icons = load_icon(&theme)
            .map_err(|err| warn!("Unable to load xcursor: {}, using fallback cursor", err))
            .unwrap_or_else(|_| {
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

        Cursor { icons, size }
    }

    pub fn get_image(&self, scale: u32, time: Duration) -> Image {
        let size = self.size * scale;
        frame(time.as_millis() as u32, size, &self.icons)
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

fn frame(mut millis: u32, size: u32, images: &[Image]) -> Image {
    let total = nearest_images(size, images).fold(0, |acc, image| acc + image.delay);
    if total == 0 {
        return nearest_images(size, images).next().unwrap().clone();
    }
    millis %= total;

    for img in nearest_images(size, images) {
        if millis < img.delay {
            return img.clone();
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

fn load_icon(theme: &CursorTheme) -> Result<Vec<Image>, Error> {
    let icon_path = theme.load_icon("default").ok_or(Error::NoDefaultCursor)?;
    let mut cursor_file = std::fs::File::open(icon_path)?;
    let mut cursor_data = Vec::new();
    cursor_file.read_to_end(&mut cursor_data)?;
    parse_xcursor(&cursor_data).ok_or(Error::Parse)
}
