//! How the raycaster's world looks (the `game` feature; see `crate::game`,
//! which sends it): its colours, the walls' textures, and how both darken
//! with distance, all tables made at compile time, in flash; and drawing
//! a column of the view ([`view_column`]), with the HUD over it ([`Hud`]),
//! from them. Apart from the hardware, to test on the host.
//!
//! Colours are in the panel's format ([`RGB444`]), as `game` sends them:
//! RGB565, or RGB444 in the low 12 bits.

use core::ops::Range;

use crate::font;
use crate::raycast::{self, Camera, MAP, Side};

/// Whether the game sends 12 bits a pixel rather than 16: a quarter less
/// over the bus, for a coarser picture (see `Display::set_format`). Off:
/// tried on the watch (2026-11-12), RGB565's smooth ceiling and floor
/// were worth more than RGB444's speed, which bands them.
pub const RGB444: bool = false;

/// A colour in the panel's format, from 8 bits a channel.
pub const fn rgb(r: u8, g: u8, b: u8) -> u16 {
    let (r, g, b) = (r as u16, g as u16, b as u16);
    if RGB444 {
        (r >> 4) << 8 | (g >> 4) << 4 | b >> 4
    } else {
        (r >> 3) << 11 | (g >> 2) << 5 | b >> 3
    }
}

/// Texels a side.
pub const TEXTURE: usize = 32;

/// How dark something this far away is, from 0 (as lit) to [`SHADES`] - 1.
pub const SHADES: usize = 8;
/// Shades a cell of distance: the darkest is from 7 / this cells on.
const SHADE_PER_CELL: f32 = 1.25;

/// The light left at a shade: from all of it to under a quarter.
const fn light(shade: usize) -> f32 {
    1.0 - shade as f32 / 9.0
}

/// The shade of something `distance` cells away (`darker` for a wall's
/// north and south faces, so that corners show).
pub fn shade(distance: f32, darker: bool) -> usize {
    let shade = (distance * SHADE_PER_CELL) as usize + usize::from(darker);
    shade.min(SHADES - 1)
}

/// The palette, 8 bits a channel: four colours to a kind of wall (see
/// `raycast::MAP`), in its texture's order.
const PALETTE: [[u8; 3]; 16] = [
    // Stone: mortar, and the stone dark to light.
    [44, 44, 50],
    [104, 104, 112],
    [128, 128, 138],
    [156, 156, 168],
    // Brick: mortar, and the bricks dark to light.
    [150, 140, 128],
    [112, 40, 30],
    [150, 58, 42],
    [184, 86, 62],
    // Metal: seams, the panels dark and light, and rivets.
    [24, 40, 64],
    [48, 86, 136],
    [64, 112, 172],
    [176, 204, 232],
    // Wood: the gaps between planks, and the grain dark to light.
    [52, 36, 20],
    [104, 72, 40],
    [134, 96, 56],
    [160, 120, 72],
];

/// Every colour of [`PALETTE`] at every shade: `[shade][colour]`.
pub static SHADED: [[u16; 16]; SHADES] = {
    let mut table = [[0; 16]; SHADES];
    let mut shade = 0;
    while shade < SHADES {
        let mut i = 0;
        while i < PALETTE.len() {
            let [r, g, b] = PALETTE[i];
            table[shade][i] = rgb(dim(r, shade), dim(g, shade), dim(b, shade));
            i += 1;
        }
        shade += 1;
    }
    table
};

const fn dim(channel: u8, shade: usize) -> u8 {
    (channel as f32 * light(shade)) as u8
}

/// The ceiling and floor, lit, and how the view's rows show them: the
/// rows nearest the middle are furthest away, so darkest, as walls are
/// at the same distance.
const CEILING: [u8; 3] = [56, 60, 72];
const FLOOR: [u8; 3] = [96, 84, 70];

/// The view's width and height, in pixels: the panel's.
pub const SIZE: usize = 240;
const HEIGHT: usize = SIZE;

/// The ceiling and floor, a colour a row: what's there wherever no wall
/// is. Smooth, unlike the walls' few shades: they're the same every
/// frame, so cost nothing to make.
pub static BACKGROUND: [u16; HEIGHT] = {
    let mut rows = [0; HEIGHT];
    let mut y = 0;
    while y < HEIGHT {
        // From the middle of the row to the middle of the view: a wall
        // `distance` away is `HEIGHT / distance` rows tall, so its foot
        // is half that from the middle.
        let from_middle = (y as f32 + 0.5 - HEIGHT as f32 / 2.0).abs();
        let distance = HEIGHT as f32 / (2.0 * from_middle);
        let shade = distance * SHADE_PER_CELL;
        let shade = if shade > (SHADES - 1) as f32 {
            (SHADES - 1) as f32
        } else {
            shade
        };
        let light = 1.0 - shade / 9.0;
        let [r, g, b] = if y < HEIGHT / 2 { CEILING } else { FLOOR };
        rows[y] = rgb(
            (r as f32 * light) as u8,
            (g as f32 * light) as u8,
            (b as f32 * light) as u8,
        );
        y += 1;
    }
    rows
};

/// A texture: [`TEXTURE`] columns, left to right as seen, of two bits a
/// texel, top first from the lowest bits, each its kind's colour (the
/// four in [`PALETTE`] from `4 * (kind - 1)`). In halves of 32 bits: the
/// M4 shifts a 64-bit number only in software, a call a pixel.
pub type Texture = [[u32; 2]; TEXTURE];

/// Which of its kind's colours column `texels` of a texture has at row
/// `v` (below [`TEXTURE`]).
#[inline(always)]
pub fn texel(texels: &[u32; 2], v: usize) -> usize {
    (texels[v / 16] >> (2 * (v % 16)) & 0b11) as usize
}

/// By kind of wall, from 1.
pub static TEXTURES: [Texture; 4] = [texture(0), texture(1), texture(2), texture(3)];

const fn texture(index: usize) -> Texture {
    let mut texture = [[0; 2]; TEXTURE];
    let mut u = 0;
    while u < TEXTURE {
        let mut v = 0;
        while v < TEXTURE {
            let texel = match index {
                0 => stone(u, v),
                1 => brick(u, v),
                2 => metal(u, v),
                _ => wood(u, v),
            };
            texture[u][v / 16] |= (texel as u32 & 0b11) << (2 * (v % 16));
            v += 1;
        }
        u += 1;
    }
    texture
}

/// Noise: a number from `x`, `y` and `seed`, its bits all over the place.
const fn hash(x: usize, y: usize, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77) ^ seed;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h
}

/// Blocks of stone 16 by 8, each row of them shifted, with mortar between,
/// and speckled.
const fn stone(u: usize, v: usize) -> u64 {
    let row = v / 8;
    let shifted = u + row * 7;
    if v.is_multiple_of(8) || shifted.is_multiple_of(16) {
        return 0;
    }
    match hash(u, v, 1) % 8 {
        0 | 1 => 1,
        2..=5 => 2,
        _ => 3,
    }
}

/// Bricks 16 by 8, every other row shifted half a brick, each a shade of
/// its own, lit along its top.
const fn brick(u: usize, v: usize) -> u64 {
    let row = v / 8;
    let shifted = u + row % 2 * 8;
    if v % 8 == 7 || shifted % 16 == 15 {
        return 0;
    }
    if v.is_multiple_of(8) {
        return 3;
    }
    let brick = hash(shifted / 16, row, 2);
    if hash(u, v, 3).is_multiple_of(7) {
        3 - (brick % 2) as u64
    } else {
        1 + (brick % 2) as u64
    }
}

/// Panels 16 square, with seams, rivets in their corners, and a lighter
/// band down each.
const fn metal(u: usize, v: usize) -> u64 {
    let (pu, pv) = (u % 16, v % 16);
    if pu == 0 || pv == 0 {
        return 0;
    }
    if (pu == 3 || pu == 13) && (pv == 3 || pv == 13) {
        return 3;
    }
    if pu >= 6 && pu <= 9 { 2 } else { 1 }
}

/// Planks 8 wide, upright, with gaps between, and grain along them.
const fn wood(u: usize, v: usize) -> u64 {
    if u.is_multiple_of(8) {
        return 0;
    }
    // Grain wanders: whether this texel's in a dark line, by the column
    // and a slow drift down the plank.
    let plank = u / 8;
    let drift = (hash(plank, v / 6, 4) % 3) as usize;
    if (u + drift).is_multiple_of(3) {
        1
    } else if hash(u, v, 5).is_multiple_of(5) {
        3
    } else {
        2
    }
}

/// Makes column `x` of the view into `column`, and says which rows: those
/// whose pixels changed since the last frame, but every one if `whole`,
/// and maybe more ([`rows_to_send`]). `wall` is where the wall in this
/// column was last frame, and becomes where it is now; `sent` is the rows
/// the column before sent, and becomes this one's.
pub fn view_column(
    camera: &Camera,
    x: usize,
    column: &mut [u8],
    wall: &mut [u8; 2],
    sent: &mut Range<usize>,
    whole: bool,
    hud: &Hud,
) -> Range<usize> {
    let hit = raycast::cast(&MAP, camera, raycast::across(x, SIZE));
    let rows = raycast::wall_rows(hit.distance, SIZE);
    let was = usize::from(wall[0])..usize::from(wall[1]);
    // The rows go up to `SIZE`, 240: they fit.
    *wall = [rows.start as u8, rows.end as u8];
    let mut changed = if whole { 0..SIZE } else { union(&was, &rows) };
    for hud in hud.changed(x) {
        changed = union(&changed, &hud);
    }
    let send = rows_to_send(changed, sent);
    *sent = send.clone();
    if send.is_empty() {
        return send;
    }
    let colors = &mut column.as_chunks_mut::<2>().0[..send.len()];
    // Ceiling and floor where there's no wall.
    let drawn = send.start.max(rows.start)..send.end.min(rows.end);
    let (above, below) = if drawn.is_empty() {
        (send.clone(), 0..0)
    } else {
        (send.start..drawn.start, drawn.end..send.end)
    };
    for y in above.chain(below) {
        colors[y - send.start] = BACKGROUND[y].to_ne_bytes();
    }
    // The wall: its texture's column, stretched over its height, and
    // shaded for its distance.
    if !drawn.is_empty() {
        let kind = usize::from(hit.wall - 1) % TEXTURES.len();
        let u = ((hit.along * TEXTURE as f32) as usize).min(TEXTURE - 1);
        let texels = TEXTURES[kind][u];
        let shades = &SHADED[shade(hit.distance, hit.side == Side::NorthSouth)][kind * 4..];
        // Down the texture in 16.16 fixed point, from the wall's top
        // (above the view when it's near), at the middle of each row.
        let tall = SIZE as f32 / hit.distance;
        let top = (SIZE as f32 - tall) / 2.0;
        let step = TEXTURE as f32 / tall;
        let mut v = ((drawn.start as f32 + 0.5 - top) * step * 65536.0) as u32;
        let step = (step * 65536.0) as u32;
        for color in &mut colors[drawn.start - send.start..drawn.end - send.start] {
            let row = ((v >> 16) as usize).min(TEXTURE - 1);
            *color = shades[texel(&texels, row)].to_ne_bytes();
            v += step;
        }
    }
    hud.draw(x, send.clone(), colors);
    pack(colors);
    send
}

/// Rows a window takes beyond what a column needs, and still serves it:
/// see [`rows_to_send`].
const SLACK: usize = 24;
/// Rows a new window takes beyond what its first column needs, each way.
const PAD: usize = 4;

/// The rows to send for a column whose `changed` rows must be, after a
/// column that sent `sent`. Sending more is never wrong, only slower: a
/// new window costs about what 80 more pixels would (see
/// `Display::stream_columns`), so a column sends the one before's rows,
/// for both to share a window, when that covers it with up to [`SLACK`]
/// to spare; or else what it needs, with [`PAD`] more each way, for the
/// columns after to fit in. In whole pairs of pixels, for RGB444's three
/// bytes to two. None for none.
pub fn rows_to_send(changed: Range<usize>, sent: &Range<usize>) -> Range<usize> {
    if changed.is_empty() {
        return changed;
    }
    if sent.start <= changed.start && changed.end <= sent.end && sent.len() - changed.len() <= SLACK
    {
        return sent.clone();
    }
    let mut send = changed.start.saturating_sub(PAD)..(changed.end + PAD).min(SIZE);
    if RGB444 {
        send = send.start & !1..((send.end + 1) & !1).min(SIZE);
    }
    send
}

/// The rows of both, and any between them.
fn union(a: &Range<usize>, b: &Range<usize>) -> Range<usize> {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.clone(),
        (_, true) => a.clone(),
        _ => a.start.min(b.start)..a.end.max(b.end),
    }
}

/// Packs a column's colours, as [`view_column`] made them (two bytes
/// each, native order), into the panel's bytes ([`RGB444`]), in place.
fn pack(colors: &mut [[u8; 2]]) {
    if !RGB444 {
        for color in colors {
            *color = u16::from_ne_bytes(*color).to_be_bytes();
        }
        return;
    }
    // Three bytes for every two pixels' four: writing never overtakes
    // reading.
    let pairs = colors.len() / 2;
    let bytes = colors.as_flattened_mut();
    for i in 0..pairs {
        let a = u16::from_ne_bytes([bytes[4 * i], bytes[4 * i + 1]]);
        let b = u16::from_ne_bytes([bytes[4 * i + 2], bytes[4 * i + 3]]);
        bytes[3 * i] = (a >> 4) as u8;
        bytes[3 * i + 1] = (a << 4) as u8 | (b >> 8) as u8;
        bytes[3 * i + 2] = b as u8;
    }
}

/// What's drawn over the view: the time, centred at the top, and, if
/// there's one (`game`'s `game-stats` feature), the [`Readout`] at the
/// bottom left.
pub struct Hud<'a> {
    pub time: Option<(u8, u8)>,
    pub readout: Option<&'a Readout>,
    /// Whether each changed since the last frame.
    pub time_changed: bool,
    pub readout_changed: bool,
}

impl Hud<'_> {
    /// The time, "12:34", in the small font at 3×.
    const TIME_SCALE: usize = 3;
    const TIME_X: usize = (SIZE - (5 * 4 - 1) * Self::TIME_SCALE) / 2;
    const TIME_Y: usize = 8;
    /// The columns and rows it covers, its shadow's too.
    const TIME: (Range<usize>, Range<usize>) = (
        Self::TIME_X..Self::TIME_X + (5 * 4 - 1) * Self::TIME_SCALE + 1,
        Self::TIME_Y..Self::TIME_Y + 5 * Self::TIME_SCALE + 1,
    );

    /// The rows of column `x` that changed since the last frame.
    fn changed(&self, x: usize) -> impl Iterator<Item = Range<usize>> {
        let time = self.time_changed && Self::TIME.0.contains(&x);
        let readout =
            self.readout.is_some() && self.readout_changed && Readout::AREA.0.contains(&x);
        let time = time.then_some(Self::TIME.1);
        time.into_iter().chain(readout.then_some(Readout::AREA.1))
    }

    /// Draws over `colors`, rows `rows` of column `x`.
    fn draw(&self, x: usize, rows: Range<usize>, colors: &mut [[u8; 2]]) {
        let white = rgb(255, 255, 255).to_ne_bytes();
        let black = rgb(0, 0, 0).to_ne_bytes();
        if Self::TIME.0.contains(&x) {
            let text = match self.time {
                Some((hour, minute)) => [
                    b'0' + hour / 10,
                    b'0' + hour % 10,
                    b':',
                    b'0' + minute / 10,
                    b'0' + minute % 10,
                ],
                None => *b"--:--",
            };
            // White, with a shadow down and to the right, to read over
            // anything.
            let lit = |x: usize, y: usize| {
                let dx = x.wrapping_sub(Self::TIME_X) / Self::TIME_SCALE;
                let dy = y.wrapping_sub(Self::TIME_Y) / Self::TIME_SCALE;
                glyph_lit(&text, dx, dy)
            };
            for y in Self::TIME.1.start.max(rows.start)..Self::TIME.1.end.min(rows.end) {
                let color = &mut colors[y - rows.start];
                if lit(x, y) {
                    *color = white;
                } else if lit(x.wrapping_sub(1), y.wrapping_sub(1)) {
                    *color = black;
                }
            }
        }
        if let Some(readout) = self.readout
            && Readout::AREA.0.contains(&x)
        {
            let dx = (x - Readout::X) / Readout::SCALE;
            for y in Readout::AREA.1.start.max(rows.start)..Readout::AREA.1.end.min(rows.end) {
                let dy = (y - Readout::Y) / Readout::SCALE;
                let line = readout.lines.get(dy / 6);
                let lit = line.is_some_and(|line| glyph_lit(line, dx, dy % 6));
                colors[y - rows.start] = if lit { white } else { black };
            }
        }
    }
}

/// Whether `text`, in the small font at 1× (each character three columns
/// and a gap, five rows), has a pixel at (`x`, `y`). Zeroes are spaces.
fn glyph_lit(text: &[u8], x: usize, y: usize) -> bool {
    let (i, column) = (x / 4, x % 4);
    match text.get(i) {
        Some(&c) if c != 0 && column < 3 && y < 5 => {
            font::small_glyph(char::from(c))[y] >> (2 - column) & 1 != 0
        }
        _ => false,
    }
}

/// The last second of `game`'s frames as text, drawn at the view's bottom left
/// (in the small font at 2×, white on black): frames a second, a frame's
/// milliseconds (average and most), the CPU's part of them (making the
/// columns), and the kilobytes sent.
#[derive(Default)]
pub struct Readout {
    lines: [[u8; Self::CHARS]; 3],
}

impl Readout {
    const CHARS: usize = 16;
    const SCALE: usize = 2;
    /// A line's height, in pixels: five rows, and a gap.
    const LINE: usize = 6 * Self::SCALE;
    const X: usize = 8;
    const Y: usize = SIZE - 8 - 3 * Self::LINE;
    /// The columns and rows it covers, the gap below its last line too.
    const AREA: (Range<usize>, Range<usize>) = (
        Self::X..Self::X + Self::CHARS * 4 * Self::SCALE,
        Self::Y..Self::Y + 3 * Self::LINE,
    );

    /// From frames a second in tenths, times in microseconds, and bytes.
    pub fn of(fps_x10: u32, frame_us: u32, frame_max: u32, fill_us: u32, bytes: u32) -> Self {
        let tenths = |line: &mut Line, n: u32| {
            line.number(n / 10);
            line.push(b'.');
            line.number(n % 10);
        };
        let mut lines = [Line::default(), Line::default(), Line::default()];
        tenths(&mut lines[0], fps_x10);
        lines[0].text(b" FPS");
        lines[1].text(b"MS ");
        tenths(&mut lines[1], frame_us / 100);
        lines[1].text(b" MAX ");
        tenths(&mut lines[1], frame_max / 100);
        lines[2].text(b"CPU ");
        tenths(&mut lines[2], fill_us / 100);
        lines[2].text(b" KB ");
        lines[2].number(bytes / 1000);
        Self {
            lines: lines.map(Line::done),
        }
    }
}

const _: () = assert!(Readout::AREA.0.end <= SIZE && Readout::AREA.1.end <= SIZE);

/// A line of [`Readout`] being written; what doesn't fit is dropped.
#[derive(Default)]
struct Line {
    bytes: [u8; Readout::CHARS],
    len: usize,
}

impl Line {
    fn push(&mut self, b: u8) {
        if let Some(slot) = self.bytes.get_mut(self.len) {
            *slot = b;
            self.len += 1;
        }
    }

    fn text(&mut self, text: &[u8]) {
        text.iter().for_each(|&b| self.push(b));
    }

    fn number(&mut self, mut n: u32) {
        let mut digits = [0u8; 10];
        let mut i = digits.len();
        loop {
            i -= 1;
            digits[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        self.text(&digits[i..]);
    }

    fn done(self) -> [u8; Readout::CHARS] {
        self.bytes
    }
}
