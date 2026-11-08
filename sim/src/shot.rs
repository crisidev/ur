//! Screenshots: the panel's 240×240 pixels as a PNG, as
//! `scripts/screenshot.py` takes them from a dev kit.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

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
    let fail = |e: &dyn std::fmt::Display| format!("writing {}: {e}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
    }
    let file = File::create(path).map_err(|e| fail(&e))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), SIZE as u32, SIZE as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    let mut writer = encoder.write_header().map_err(|e| fail(&e))?;
    writer.write_image_data(&rgb()).map_err(|e| fail(&e))?;
    writer.finish().map_err(|e| fail(&e))
}
