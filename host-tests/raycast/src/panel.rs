//! A panel for the view to be sent to, as the watch's: what `game` sends
//! of each column (`game_view::view_column`) lands in it.

use crate::game_view::{self, Hud, RGB444, Readout, SIZE, view_column};
use crate::raycast::{Camera, Player};

pub struct Panel {
    /// RGB565 (or RGB444's 12 bits), by row.
    pub pixels: Vec<u16>,
    /// Where each column's wall was (`game`'s lent front buffer).
    walls: Vec<[u8; 2]>,
}

impl Panel {
    pub fn new() -> Self {
        Self {
            pixels: vec![0; SIZE * SIZE],
            walls: vec![[0; 2]; SIZE],
        }
    }

    /// Sends a frame, every pixel if `whole`; how many pixels went.
    pub fn frame(
        &mut self,
        player: &Player,
        whole: bool,
        time: Option<(u8, u8)>,
        readout: &Readout,
        changed: bool,
    ) -> usize {
        self.send(player, whole, time, readout, changed, changed)
    }

    /// Sends only what changed; how many pixels went.
    #[cfg(test)]
    pub fn frame_changing(
        &mut self,
        player: &Player,
        time: Option<(u8, u8)>,
        readout: &Readout,
        time_changed: bool,
        readout_changed: bool,
    ) -> usize {
        self.send(player, false, time, readout, time_changed, readout_changed)
    }

    fn send(
        &mut self,
        player: &Player,
        whole: bool,
        time: Option<(u8, u8)>,
        readout: &Readout,
        time_changed: bool,
        readout_changed: bool,
    ) -> usize {
        let hud = Hud {
            time,
            readout,
            time_changed: whole || time_changed,
            readout_changed: whole || readout_changed,
        };
        let camera = Camera::new(player, SIZE, SIZE);
        let mut column = [0u8; SIZE * 2];
        let mut sent = 0;
        for x in 0..SIZE {
            let rows = view_column(&camera, x, &mut column, &mut self.walls[x], whole, &hud);
            sent += rows.len();
            let colors: Vec<u16> = if RGB444 {
                assert!(
                    rows.start.is_multiple_of(2) && rows.len().is_multiple_of(2),
                    "{rows:?}"
                );
                column[..rows.len() * 3 / 2]
                    .chunks(3)
                    .flat_map(|b| {
                        let (b0, b1, b2) = (u16::from(b[0]), u16::from(b[1]), u16::from(b[2]));
                        [b0 << 4 | b1 >> 4, (b1 & 0xf) << 8 | b2]
                    })
                    .collect()
            } else {
                column[..rows.len() * 2]
                    .chunks(2)
                    .map(|b| u16::from_be_bytes([b[0], b[1]]))
                    .collect()
            };
            for (y, color) in rows.zip(colors) {
                self.pixels[y * SIZE + x] = color;
            }
        }
        sent
    }

    /// As a PPM image.
    pub fn ppm(&self) -> Vec<u8> {
        let mut out = format!("P6 {SIZE} {SIZE} 255\n").into_bytes();
        for &c in &self.pixels {
            let (r, g, b) = if game_view::RGB444 {
                ((c >> 8 & 0xf) * 17, (c >> 4 & 0xf) * 17, (c & 0xf) * 17)
            } else {
                (
                    (c >> 11) * 255 / 31,
                    (c >> 5 & 0x3f) * 255 / 63,
                    (c & 0x1f) * 255 / 31,
                )
            };
            out.extend([r as u8, g as u8, b as u8]);
        }
        out
    }
}
