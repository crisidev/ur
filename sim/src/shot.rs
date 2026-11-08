//! Screenshots: the panel's 240×240 pixels as a PNG, as
//! `scripts/screenshot.py` takes them from a dev kit; or the whole watch.

use std::path::Path;

use crate::brightness::Level;
use crate::case::Case;
use crate::display::{PANEL, SIZE, rgb888};

/// The panel as RGB, row by row.
pub fn rgb() -> Vec<u8> {
    PANEL.with_borrow(|panel| {
        panel
            .pixels
            .iter()
            .flat_map(|&pixel| rgb888(pixel, panel.idle))
            .collect()
    })
}

pub fn save(path: &Path) -> Result<(), String> {
    write(path, SIZE, SIZE, &rgb())
}

/// The whole watch, as the window draws it (see [`crate::case`]), its
/// panel lit by `backlight`, at `zoom` pixels a canvas pixel.
pub fn save_watch(path: &Path, backlight: Level, zoom: usize) -> Result<(), String> {
    let case = Case::new(zoom);
    let mut buffer = vec![0; case.width * case.height];
    case.draw(&mut buffer, crate::case::light(backlight), 0.0);
    let rgb: Vec<u8> = buffer
        .iter()
        .flat_map(|&pixel| {
            let [_, r, g, b] = pixel.to_be_bytes();
            [r, g, b]
        })
        .collect();
    write(path, case.width, case.height, &rgb)
}

// ==============================================================================
// Comparing screenshots
// ==============================================================================
//
// Two places compare frames: `--devkit` (the simulator's against a dev
// kit's) and `--diff` (one run's screenshots against another's, as the
// tests do against the approved ones, and CI between a PR and its base).
// Both show a difference the same way, drawn here.

// Comparing screenshots (`--diff`, `--devkit`) is the command line's: the
// browser's build has no files to read.

/// An RGB image, 8 bits a channel, row by row.
#[cfg(not(target_arch = "wasm32"))]
#[derive(PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Image {
    /// The pixel at (`x`, `y`), or `None` outside the image.
    fn pixel(&self, x: usize, y: usize) -> Option<[u8; 3]> {
        (x < self.width && y < self.height).then(|| {
            let at = (y * self.width + x) * 3;
            [self.rgb[at], self.rgb[at + 1], self.rgb[at + 2]]
        })
    }
}

/// Reads a PNG as RGB, whatever its colour type: the simulator writes RGB,
/// but an image approved by hand (or by another tool) needn't be.
#[cfg(not(target_arch = "wasm32"))]
pub fn read(path: &Path) -> Result<Image, String> {
    let fail = |e: &dyn std::fmt::Display| format!("reading {}: {e}", path.display());
    let file = std::fs::File::open(path).map_err(|e| fail(&e))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| fail(&e))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| fail(&"too large"))?;
    let mut buffer = vec![0; size];
    let info = reader.next_frame(&mut buffer).map_err(|e| fail(&e))?;
    buffer.truncate(info.buffer_size());
    let (width, height) = (info.width as usize, info.height as usize);
    let rgb = match info.color_type {
        png::ColorType::Rgb => buffer,
        png::ColorType::Rgba => buffer
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|&[r, g, b, _]| [r, g, b])
            .collect(),
        png::ColorType::Grayscale => buffer.iter().flat_map(|&v| [v, v, v]).collect(),
        png::ColorType::GrayscaleAlpha => buffer
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[v, _]| [v, v, v])
            .collect(),
        // `normalize_to_color8` expands palettes.
        png::ColorType::Indexed => return Err(fail(&"an unexpanded palette")),
    };
    Ok(Image { width, height, rgb })
}

/// Writes `before`, `after`, and where they differ, side by side: the
/// difference is magenta where the pixels differ, and `after` dimmed
/// elsewhere, so the change stands out against what it's in. Returns how
/// many pixels differ.
///
/// The two needn't be the same size (a screenshot of the whole watch,
/// say, against one of the panel): each panel is as large as the larger,
/// and a pixel only one of them has differs.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_difference(path: &Path, before: &Image, after: &Image) -> Result<usize, String> {
    const GAP: usize = 8;
    const BACKGROUND: u8 = 0x80;
    let panel = before.width.max(after.width);
    let height = before.height.max(after.height);
    let width = 3 * panel + 2 * GAP;
    let mut rgb = vec![BACKGROUND; width * height * 3];
    let mut differ = 0;
    for y in 0..height {
        for x in 0..panel {
            let mut put = |index: usize, color: [u8; 3]| {
                let at = (y * width + index * (panel + GAP) + x) * 3;
                rgb[at..at + 3].copy_from_slice(&color);
            };
            let (old, new) = (before.pixel(x, y), after.pixel(x, y));
            if let Some(color) = old {
                put(0, color);
            }
            if let Some(color) = new {
                put(1, color);
            }
            match (old, new) {
                (Some(old), Some(new)) if old == new => put(2, new.map(|c| c / 3)),
                (None, None) => {}
                _ => {
                    differ += 1;
                    put(2, [255, 0, 255]);
                }
            }
        }
    }
    write(path, width, height, &rgb)?;
    Ok(differ)
}

fn write(path: &Path, width: usize, height: usize, rgb: &[u8]) -> Result<(), String> {
    let png =
        encode(width, height, rgb).map_err(|e| format!("encoding {}: {e}", path.display()))?;
    crate::platform::save(path, &png)
}

/// A PNG of `rgb`, in memory: the file is the platform's business (the
/// browser downloads it).
fn encode(width: usize, height: usize, rgb: &[u8]) -> Result<Vec<u8>, png::EncodingError> {
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgb)?;
    writer.finish()?;
    Ok(png)
}
