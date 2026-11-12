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
use crate::raycast::{self, Camera, Kind, MAP, Side, Taken, Thing};

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
/// `raycast::MAP`), in its texture's order; then three to a kind of
/// thing (see `raycast::Kind`), from [`SPRITE_COLORS`], for their
/// pictures' colours 1 to 3 (0 is none: what's behind shows).
const PALETTE: [[u8; 3]; COLORS] = [
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
    // A gem: dark, bright, and its glint.
    [16, 110, 84],
    [40, 200, 150],
    [210, 255, 236],
    // A barrel: dark wood, wood, and its hoops.
    [72, 42, 20],
    [128, 80, 40],
    [70, 70, 76],
    // A pillar: its stone in shadow, lit, and in the light.
    [84, 84, 92],
    [130, 130, 142],
    [182, 182, 194],
];
const COLORS: usize = 16 + 3 * 3;
/// Where [`PALETTE`]'s colours for things start.
const SPRITE_COLORS: usize = 16;

/// Every colour of [`PALETTE`] at every shade: `[shade][colour]`.
pub static SHADED: [[u16; COLORS]; SHADES] = {
    let mut table = [[0; COLORS]; SHADES];
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

/// Things' pictures, as [`Texture`]s, by `raycast::Kind`: a texel of 0 is
/// none, so what's behind shows.
pub static SPRITES: [Texture; 3] = [texture(4), texture(5), texture(6)];

/// Which of [`SPRITES`] (and of their colours) a thing has.
const fn sprite(kind: Kind) -> usize {
    match kind {
        Kind::Gem => 0,
        Kind::Barrel => 1,
        Kind::Pillar => 2,
    }
}

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
                3 => wood(u, v),
                4 => gem(u, v),
                5 => barrel(u, v),
                _ => pillar(u, v),
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

/// The pictures of things ([`SPRITES`]), in coordinates doubled about
/// the middle, so that it's between pixels: from -31 to 31.
const fn centred(t: usize) -> i32 {
    2 * t as i32 - 31
}

/// A gem: a diamond, lit from the top left, its edge dark.
const fn gem(u: usize, v: usize) -> u64 {
    let (x, y) = (centred(u), centred(v) + 2);
    let reach = x.abs() + y.abs() * 5 / 4;
    if reach > 26 {
        return 0;
    }
    if reach > 22 {
        return 1;
    }
    match (x < 0, y < 0) {
        (true, true) => 3,
        (false, false) => 1,
        _ => 2,
    }
}

/// A barrel: staves, darker at its sides, with two hoops, its top and
/// bottom rounded.
const fn barrel(u: usize, v: usize) -> u64 {
    let x = centred(u);
    let width = if v < 2 || v > 29 { 17 } else { 21 };
    if x.abs() > width {
        return 0;
    }
    if (v >= 6 && v <= 8) || (v >= 23 && v <= 25) {
        return 3;
    }
    if x.abs() > 15 || (x + 31) % 10 == 0 {
        1
    } else {
        2
    }
}

/// A pillar: a round shaft, lit from the left, between a wider capital
/// and base.
const fn pillar(u: usize, v: usize) -> u64 {
    let x = centred(u);
    let end = v < 4 || v > 27;
    if x.abs() > if end { 19 } else { 13 } {
        return 0;
    }
    if v == 4 || v == 27 {
        return 1;
    }
    if x < -5 {
        3
    } else if x > 6 {
        1
    } else {
        2
    }
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

/// A thing as the view shows it, for a frame ([`Sprites`]).
#[derive(Clone, Copy, Default)]
struct Sprite {
    /// Which of [`SPRITES`].
    picture: usize,
    /// Its middle across the view, its width and height, and its top, in
    /// pixels (as `f32`s: parts of a pixel count, for smooth movement).
    centre: f32,
    size: f32,
    top: f32,
    /// How far ahead, as walls' distances are.
    depth: f32,
}

impl Sprite {
    /// The rows of the view it covers.
    fn rows(&self) -> Range<usize> {
        let top = self.top.max(0.0);
        let bottom = (self.top + self.size).clamp(0.0, SIZE as f32);
        top as usize..(bottom + 0.999) as usize
    }
}

/// The things in view this frame, furthest first, as [`view_column`]
/// draws them: each over what's further away, and under what's nearer.
pub struct Sprites {
    list: [Sprite; raycast::THINGS.len()],
    len: usize,
}

impl Sprites {
    /// Those of `things` not taken, ahead of `camera`, and in the view.
    pub fn of(camera: &Camera, things: &[Thing], taken: Taken) -> Self {
        let mut sprites = Self {
            list: [Sprite::default(); raycast::THINGS.len()],
            len: 0,
        };
        let half = SIZE as f32 / 2.0;
        for (i, thing) in things.iter().enumerate() {
            if taken >> i & 1 != 0 || sprites.len == sprites.list.len() {
                continue;
            }
            let Some((across, depth)) = camera.project(thing.x, thing.y) else {
                continue;
            };
            // As a wall is: as tall as the view at a distance of one, but
            // standing on the floor, and only so much of a wall's height.
            let size = thing.kind.height() * SIZE as f32 / depth;
            let centre = half * (1.0 + across);
            if centre + size / 2.0 < 0.0 || centre - size / 2.0 > SIZE as f32 {
                continue;
            }
            let floor = half + half / depth;
            sprites.list[sprites.len] = Sprite {
                picture: sprite(thing.kind),
                centre,
                size,
                top: floor - size,
                depth,
            };
            sprites.len += 1;
        }
        sprites.list[..sprites.len].sort_unstable_by(|a, b| b.depth.total_cmp(&a.depth));
        sprites
    }

    /// Those in column `x` nearer than `distance` (its wall), furthest
    /// first, and which column of their picture.
    fn in_column(&self, x: usize, distance: f32) -> impl Iterator<Item = (&Sprite, usize)> {
        let middle = x as f32 + 0.5;
        self.list[..self.len].iter().filter_map(move |sprite| {
            let from_left = middle - (sprite.centre - sprite.size / 2.0);
            let inside = sprite.depth < distance && from_left >= 0.0 && from_left < sprite.size;
            inside.then(|| {
                let u = (from_left / sprite.size * TEXTURE as f32) as usize;
                (sprite, u.min(TEXTURE - 1))
            })
        })
    }
}

/// Makes column `x` of the view into `column`, and says which rows: those
/// whose pixels changed since the last frame, but every one if `whole`,
/// and maybe more ([`rows_to_send`]). `shown` is the rows that weren't
/// ceiling or floor in this column last frame (a wall, things in front of
/// it), and becomes this frame's; `sent` is the rows the column before
/// sent, and becomes this one's.
#[allow(clippy::too_many_arguments)]
pub fn view_column(
    camera: &Camera,
    sprites: &Sprites,
    x: usize,
    column: &mut [u8],
    shown: &mut [u8; 2],
    sent: &mut Range<usize>,
    whole: bool,
    hud: &Hud,
) -> Range<usize> {
    let hit = raycast::cast(&MAP, camera, raycast::across(x, SIZE));
    let rows = raycast::wall_rows(hit.distance, SIZE);
    let mut showing = rows.clone();
    for (sprite, _) in sprites.in_column(x, hit.distance) {
        showing = union(&showing, &sprite.rows());
    }
    let was = usize::from(shown[0])..usize::from(shown[1]);
    // The rows go up to `SIZE`, 240: they fit.
    *shown = [showing.start as u8, showing.end as u8];
    let mut changed = if whole {
        0..SIZE
    } else {
        union(&was, &showing)
    };
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
    // Things in front of it, the same way, furthest first, with what's
    // behind showing through their pictures' 0s.
    for (sprite, u) in sprites.in_column(x, hit.distance) {
        let texels = SPRITES[sprite.picture][u];
        let rows = sprite.rows();
        let drawn = send.start.max(rows.start)..send.end.min(rows.end);
        if texels == [0; 2] || drawn.is_empty() {
            continue;
        }
        let first = SPRITE_COLORS + 3 * sprite.picture;
        let shades = &SHADED[shade(sprite.depth, false)][first..first + 3];
        let step = TEXTURE as f32 / sprite.size;
        let mut v = ((drawn.start as f32 + 0.5 - sprite.top) * step * 65536.0) as u32;
        let step = (step * 65536.0) as u32;
        for color in &mut colors[drawn.start - send.start..drawn.end - send.start] {
            let row = ((v >> 16) as usize).min(TEXTURE - 1);
            if let Some(i) = texel(&texels, row).checked_sub(1) {
                *color = shades[i].to_ne_bytes();
            }
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

/// What's drawn over the view: the time, centred at the top; the gems
/// found, at the top left; and, if there's one (`game`'s `game-stats`
/// feature), the [`Readout`] at the bottom left.
pub struct Hud<'a> {
    pub time: Option<(u8, u8)>,
    /// Found, of all.
    pub gems: (u8, u8),
    pub readout: Option<&'a Readout>,
    /// Whether each changed since the last frame.
    pub time_changed: bool,
    pub gems_changed: bool,
    pub readout_changed: bool,
}

/// Text in the HUD that reads over anything: white, in the small font at
/// `scale`, with a black shadow down and to the right.
struct Label {
    x: usize,
    y: usize,
    scale: usize,
    /// Characters, at most.
    chars: usize,
}

impl Label {
    /// The columns and rows it covers, its shadow's too.
    const fn area(&self) -> (Range<usize>, Range<usize>) {
        (
            self.x..self.x + (self.chars * 4 - 1) * self.scale + 1,
            self.y..self.y + 5 * self.scale + 1,
        )
    }

    /// Draws `text` over `colors`, rows `rows` of column `x`.
    fn draw(&self, text: &[u8], x: usize, rows: Range<usize>, colors: &mut [[u8; 2]]) {
        let white = rgb(255, 255, 255).to_ne_bytes();
        let black = rgb(0, 0, 0).to_ne_bytes();
        let lit = |x: usize, y: usize| {
            let dx = x.wrapping_sub(self.x) / self.scale;
            let dy = y.wrapping_sub(self.y) / self.scale;
            glyph_lit(text, dx, dy)
        };
        let (columns, area) = self.area();
        if !columns.contains(&x) {
            return;
        }
        for y in area.start.max(rows.start)..area.end.min(rows.end) {
            let color = &mut colors[y - rows.start];
            if lit(x, y) {
                *color = white;
            } else if lit(x.wrapping_sub(1), y.wrapping_sub(1)) {
                *color = black;
            }
        }
    }
}

impl Hud<'_> {
    /// The time, "12:34", centred.
    const TIME: Label = Label {
        x: (SIZE - (5 * 4 - 1) * 3) / 2,
        y: 8,
        scale: 3,
        chars: 5,
    };
    /// The gems, "GEMS 3/8".
    const GEMS: Label = Label {
        x: 8,
        y: 10,
        scale: 2,
        chars: 8,
    };

    /// The rows of column `x` that changed since the last frame.
    fn changed(&self, x: usize) -> impl Iterator<Item = Range<usize>> {
        let changed = |changed: bool, (columns, rows): (Range<usize>, Range<usize>)| {
            (changed && columns.contains(&x)).then_some(rows)
        };
        let readout = self.readout.is_some() && self.readout_changed;
        changed(self.time_changed, Self::TIME.area())
            .into_iter()
            .chain(changed(self.gems_changed, Self::GEMS.area()))
            .chain(changed(readout, Readout::AREA))
    }

    /// Draws over `colors`, rows `rows` of column `x`.
    fn draw(&self, x: usize, rows: Range<usize>, colors: &mut [[u8; 2]]) {
        let digit = |n: u8| b'0' + n % 10;
        let time = match self.time {
            Some((hour, minute)) => [
                digit(hour / 10),
                digit(hour),
                b':',
                digit(minute / 10),
                digit(minute),
            ],
            None => *b"--:--",
        };
        Self::TIME.draw(&time, x, rows.clone(), colors);
        let (found, all) = self.gems;
        let gems = [b'G', b'E', b'M', b'S', b' ', digit(found), b'/', digit(all)];
        Self::GEMS.draw(&gems, x, rows.clone(), colors);
        if let Some(readout) = self.readout
            && Readout::AREA.0.contains(&x)
        {
            let white = rgb(255, 255, 255).to_ne_bytes();
            let black = rgb(0, 0, 0).to_ne_bytes();
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
