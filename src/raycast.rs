//! The raycaster's world, apart from the hardware (the `game` feature;
//! see `crate::game`, which draws it): the map, the player and how they
//! move, and casting a ray across the map for each column of the view.
//!
//! Positions are in map cells (a cell is 1 by 1, `x` east, `y` south, as
//! the map's rows are written), angles in radians, clockwise from east
//! (so that turning right, as seen from above with south down, adds).
//!
//! In `f32`: the Cortex-M4F does it in hardware, a division included,
//! where fixed point needs a 64-bit software division for each ray's step
//! (see `game`'s commit for the measurement on the watch).
//!
//! Rays step from cell edge to cell edge (DDA, as in Wolfenstein 3D), so
//! a ray costs one step per cell crossed, and only ever lands on a wall's
//! face. The distance to that face is measured along the view's direction,
//! not the ray's, so that a flat wall looks flat ("perpendicular"
//! distance, with no fish-eye).

use core::f32::consts::TAU;
use core::ops::Range;

/// The map's width and height, in cells.
pub const MAP_SIZE: usize = 16;

/// What's in each cell, by row (`[y][x]`): 0 for nothing, otherwise a
/// wall, of the kind numbered (which `game` gives a look).
pub type Map = [[u8; MAP_SIZE]; MAP_SIZE];

/// The kinds of wall in [`MAP`]: 1 to this.
pub const WALL_KINDS: u8 = 4;

/// The one map. Closed all round: [`wall`] says so of anywhere off it too,
/// but a ray that left would cost steps all the way to [`MAX_STEPS`].
pub const MAP: Map = {
    const ROWS: [&[u8; MAP_SIZE]; MAP_SIZE] = [
        b"1111111111111111",
        b"1..............1",
        b"1..22..........1",
        b"1..2.....3333..1",
        b"1..2.....3..3..1",
        b"1........3..3..1",
        b"1........3.33..1",
        b"1..............1",
        b"1....4.4.4.....1",
        b"1..............1",
        b"1....4.4.4..2221",
        b"1...........2..1",
        b"1..33.......2..1",
        b"1..33..........1",
        b"1..............1",
        b"1111111111111111",
    ];
    let mut map = [[0; MAP_SIZE]; MAP_SIZE];
    let mut y = 0;
    while y < MAP_SIZE {
        let mut x = 0;
        while x < MAP_SIZE {
            map[y][x] = match ROWS[y][x] {
                b'.' => 0,
                c => {
                    assert!(c >= b'1' && c - b'0' <= WALL_KINDS);
                    c - b'0'
                }
            };
            x += 1;
        }
        y += 1;
    }
    map
};

/// The wall in cell (`x`, `y`), or 0 if it's open; anywhere off the map
/// is wall.
pub fn wall(map: &Map, x: i32, y: i32) -> u8 {
    match (usize::try_from(x), usize::try_from(y)) {
        (Ok(x), Ok(y)) if x < MAP_SIZE && y < MAP_SIZE => map[y][x],
        _ => 1,
    }
}

/// How far the player reaches, as a square this far each way from where
/// they stand: no nearer a wall than this.
pub const RADIUS: f32 = 0.25;
/// Cells a second.
pub const WALK_SPEED: f32 = 2.5;
/// Radians a second.
pub const TURN_SPEED: f32 = 2.0;
/// The most time one step may take, in seconds: a longer gap between
/// frames (a pause, a busy moment) moves no further than this, so that
/// one step is always short of a cell, and can't pass through a wall.
pub const MAX_STEP: f32 = 0.1;
const _: () = assert!(WALK_SPEED * MAX_STEP < 1.0 - 2.0 * RADIUS);
const _: () = assert!(TURN_SPEED * MAX_STEP < TAU);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    pub x: f32,
    pub y: f32,
    /// Which way they face (see the module's docs), in `0..TAU`.
    pub angle: f32,
}

impl Player {
    /// Where the game begins: in the north-west corner, facing east.
    pub const START: Self = Self {
        x: 1.5,
        y: 1.5,
        angle: 0.0,
    };

    /// The way they face, as a unit vector.
    pub fn facing(&self) -> (f32, f32) {
        cos_sin(self.angle)
    }
}

/// The cosine and sine of `angle`, in `0..TAU`, to within a millionth or
/// so. Not libm's: `sinf` and `cosf` reduce any angle at all, in double
/// precision, which the M4F only does in software (5.5 KB of it).
pub fn cos_sin(angle: f32) -> (f32, f32) {
    use core::f32::consts::FRAC_PI_2;
    // To within an eighth of a turn of the nearest quarter, where the
    // series converge quickly.
    let quarter = (angle / FRAC_PI_2 + 0.5) as i32;
    let x = angle - quarter as f32 * FRAC_PI_2;
    let x2 = x * x;
    let sin = x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0)));
    let cos = 1.0 - x2 / 2.0 * (1.0 - x2 / 12.0 * (1.0 - x2 / 30.0 * (1.0 - x2 / 56.0)));
    match quarter & 3 {
        0 => (cos, sin),
        1 => (-sin, cos),
        2 => (-cos, -sin),
        _ => (sin, -cos),
    }
}

/// What the player asks for in a step: each from -1 to 1, for how fast
/// (touch only ever asks -1, 0 or 1; tilt anything between).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    /// Left (-1) or right (1).
    pub turn: f32,
    /// Back (-1) or forward (1).
    pub walk: f32,
}

impl Input {
    /// From where a finger is on the panel (pixels, 0 to `panel` across
    /// and down), in thirds each way: the left and right thirds turn that
    /// way, the top and bottom ones walk forward and back, and the middle
    /// does neither. So a corner does both, and nothing at all is the
    /// middle, or no finger.
    pub fn from_touch(at: Option<(u8, u8)>, panel: u16) -> Self {
        let Some((x, y)) = at else {
            return Self::default();
        };
        let third = |at: u8| match u16::from(at) * 3 / panel {
            0 => -1.0,
            1 => 0.0,
            _ => 1.0,
        };
        Self {
            turn: third(x),
            walk: -third(y),
        }
    }

    /// From how the wrist is tilted, `now`, from how it was when the game
    /// started, `neutral` (accelerations in the watch's axes, 1024 per g:
    /// x to 3 o'clock, y to 12, z out of the screen, pointing up, away
    /// from gravity). Rolling the 3 o'clock edge down turns right, and
    /// up, left; tipping 12 o'clock away, down, walks forward, and
    /// towards, back. Nothing within [`TILT_DEAD`] g of neutral, full
    /// speed from [`TILT_FULL`]. `None` for a reading far from 1 g: the
    /// arm swinging, which says nothing of the tilt.
    pub fn from_tilt(now: [i16; 3], neutral: [i16; 3]) -> Option<Self> {
        const G: f32 = 1024.0;
        let [x, y, z] = now.map(|a| f32::from(a) / G);
        let magnitude2 = x * x + y * y + z * z;
        if !(0.7 * 0.7..=1.3 * 1.3).contains(&magnitude2) {
            return None;
        }
        let [nx, ny, _] = neutral.map(|a| f32::from(a) / G);
        let speed = |tilt: f32| {
            let past = (tilt.abs() - TILT_DEAD) / (TILT_FULL - TILT_DEAD);
            past.clamp(0.0, 1.0).copysign(tilt)
        };
        // Rolled right, "up" leans to 9 o'clock: x falls. Tipped away, it
        // leans to 6: y falls.
        Some(Self {
            turn: speed(nx - x),
            walk: speed(ny - y),
        })
    }
}

/// How far from neutral, in g, a tilt starts to count (about 6°), and
/// goes full speed (about 24°): see [`Input::from_tilt`].
pub const TILT_DEAD: f32 = 0.1;
pub const TILT_FULL: f32 = 0.4;

/// How the player steers, as picked on the game's title.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Controls {
    /// By where a finger's held ([`Input::from_touch`]).
    #[default]
    Touch,
    /// By the wrist's tilt ([`Input::from_tilt`]).
    Tilt,
    /// Turning by tilt, walking by touch: the top half of the screen
    /// forward, the bottom half back.
    Hybrid,
}

impl Controls {
    /// The next, round and round.
    pub fn next(self) -> Self {
        match self {
            Self::Touch => Self::Tilt,
            Self::Tilt => Self::Hybrid,
            Self::Hybrid => Self::Touch,
        }
    }

    /// Whether it needs the tilt.
    pub fn tilts(self) -> bool {
        self != Self::Touch
    }

    /// What a step asks for, from the finger (as for
    /// [`Input::from_touch`]) and the tilt ([`Input::from_tilt`]'s, if
    /// there's one).
    pub fn input(self, finger: Option<(u8, u8)>, panel: u16, tilt: Option<Input>) -> Input {
        let tilt = tilt.unwrap_or_default();
        match self {
            Self::Touch => Input::from_touch(finger, panel),
            Self::Tilt => tilt,
            Self::Hybrid => Input {
                turn: tilt.turn,
                walk: match finger {
                    Some((_, y)) if u16::from(y) * 2 < panel => 1.0,
                    Some(_) => -1.0,
                    None => 0.0,
                },
            },
        }
    }
}

/// Moves `player` as `input` asks, over `seconds` (up to [`MAX_STEP`]).
/// Walls stop them, and solid `things` ([`Kind::solid`]), but only along
/// the way they're in: walking into one at an angle slides along it.
/// Whether something stopped them, of most of the way they asked to go:
/// a bump.
pub fn step(map: &Map, things: &[Thing], player: &mut Player, input: Input, seconds: f32) -> bool {
    let seconds = seconds.clamp(0.0, MAX_STEP);
    // A step turns less than a whole turn (see `MAX_STEP`), so one
    // wrap brings it back into `0..TAU`; and once more if that rounded up
    // to `TAU` itself (an angle just below 0).
    let mut angle = player.angle + input.turn.clamp(-1.0, 1.0) * TURN_SPEED * seconds;
    if angle < 0.0 {
        angle += TAU;
    }
    if angle >= TAU {
        angle -= TAU;
    }
    player.angle = angle;
    if input.walk == 0.0 {
        return false;
    }
    let (fx, fy) = player.facing();
    let distance = input.walk.clamp(-1.0, 1.0) * WALK_SPEED * seconds;
    let (dx, dy) = (fx * distance, fy * distance);
    // Each way on its own: what one blocks, the other may still go.
    let mut moved = 0.0;
    if !blocked(map, things, player.x + dx, player.y) {
        player.x += dx;
        moved += dx * dx;
    }
    if !blocked(map, things, player.x, player.y + dy) {
        player.y += dy;
        moved += dy * dy;
    }
    moved < distance * distance / 4.0
}

/// Whether a player at (`x`, `y`) would be in a wall, or a solid thing:
/// any of their square's corners in a wall, or their square overlapping
/// the thing's.
fn blocked(map: &Map, things: &[Thing], x: f32, y: f32) -> bool {
    let cell = |v: f32| libm::floorf(v) as i32;
    let walls = [
        (-RADIUS, -RADIUS),
        (RADIUS, -RADIUS),
        (-RADIUS, RADIUS),
        (RADIUS, RADIUS),
    ]
    .iter()
    .any(|&(ox, oy)| wall(map, cell(x + ox), cell(y + oy)) != 0);
    let reach = RADIUS + PROP_RADIUS;
    walls
        || things
            .iter()
            .any(|t| t.kind.solid() && (t.x - x).abs() < reach && (t.y - y).abs() < reach)
}

// ==============================================================================
// Things: gems to find, and props in the way
// ==============================================================================

/// What a thing in the world is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// To find: walked into, it's taken ([`pick`]).
    Gem,
    Barrel,
    Pillar,
}

impl Kind {
    /// Whether it's in the way ([`step`]): all but gems.
    pub const fn solid(self) -> bool {
        !matches!(self, Self::Gem)
    }

    /// How tall it stands on the floor, as a fraction of a wall: and as
    /// wide, its picture being square.
    pub const fn height(self) -> f32 {
        match self {
            Self::Gem => 0.35,
            Self::Barrel => 0.6,
            Self::Pillar => 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thing {
    /// Where its middle is.
    pub x: f32,
    pub y: f32,
    pub kind: Kind,
}

/// How far a solid thing reaches, as a square this far each way from its
/// middle (see [`blocked`]).
pub const PROP_RADIUS: f32 = 0.3;
/// How near a gem's middle the player's must come to take it.
pub const PICK_RADIUS: f32 = 0.45;

/// Everything in [`MAP`], each in an open cell's middle.
pub const THINGS: [Thing; 12] = {
    const fn at(x: usize, y: usize, kind: Kind) -> Thing {
        assert!(MAP[y][x] == 0, "a thing in a wall");
        Thing {
            x: x as f32 + 0.5,
            y: y as f32 + 0.5,
            kind,
        }
    }
    [
        at(13, 1, Kind::Gem),
        at(10, 4, Kind::Gem),
        at(4, 4, Kind::Gem),
        at(14, 7, Kind::Gem),
        at(6, 9, Kind::Gem),
        at(13, 11, Kind::Gem),
        at(8, 13, Kind::Gem),
        at(1, 14, Kind::Gem),
        at(7, 5, Kind::Barrel),
        at(11, 13, Kind::Barrel),
        at(3, 7, Kind::Pillar),
        at(13, 5, Kind::Pillar),
    ]
};

/// Which of [`THINGS`] have been taken, a bit each (only gems are).
pub type Taken = u16;
const _: () = assert!(THINGS.len() <= Taken::BITS as usize);

/// The gems among `things`, and how many of them are taken.
pub fn gems(things: &[Thing], taken: Taken) -> (u8, u8) {
    let mut found = 0;
    let mut all = 0;
    for (i, thing) in things.iter().enumerate() {
        if thing.kind == Kind::Gem {
            all += 1;
            found += u8::from(taken >> i & 1 != 0);
        }
    }
    (found, all)
}

/// Takes any gem not yet taken that `player` is within [`PICK_RADIUS`] of;
/// whether one was.
pub fn pick(things: &[Thing], taken: &mut Taken, player: &Player) -> bool {
    let mut picked = false;
    for (i, thing) in things.iter().enumerate() {
        let (dx, dy) = (thing.x - player.x, thing.y - player.y);
        if thing.kind == Kind::Gem
            && *taken >> i & 1 == 0
            && dx * dx + dy * dy < PICK_RADIUS * PICK_RADIUS
        {
            *taken |= 1 << i;
            picked = true;
        }
    }
    picked
}

/// The player's view, for a frame's rays: where they stand, the way they
/// face, and the camera's plane, at right angles to that (to the right),
/// as long as the view is wide, at a distance of one.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    x: f32,
    y: f32,
    facing: (f32, f32),
    plane: (f32, f32),
}

impl Camera {
    /// For a view `width` columns of pixels by `height` rows, of walls one
    /// cell wide as wide as they're tall: half as wide as tall (a wall
    /// a cell away fills the height), in the plane's terms.
    pub fn new(player: &Player, width: usize, height: usize) -> Self {
        let (fx, fy) = player.facing();
        let half_width = width as f32 / (2 * height) as f32;
        Self {
            x: player.x,
            y: player.y,
            facing: (fx, fy),
            plane: (-fy * half_width, fx * half_width),
        }
    }

    /// Where a point in the world is in the view, if it's ahead: across it
    /// as for [`cast`] (-1 at the left edge, 1 at the right, beyond them
    /// outside it), and how far ahead, as walls' distances are.
    pub fn project(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let (dx, dy) = (x - self.x, y - self.y);
        // The point as so much of the plane and of the way faced: the
        // inverse of their matrix.
        let det = self.plane.0 * self.facing.1 - self.facing.0 * self.plane.1;
        let across = (self.facing.1 * dx - self.facing.0 * dy) / det;
        let ahead = (self.plane.0 * dy - self.plane.1 * dx) / det;
        (ahead >= NEAREST).then(|| (across / ahead, ahead))
    }
}

/// Which way a ray crossed into the wall it hit: through the cell's west
/// or east edge (a line of constant `x`), or north or south.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    EastWest,
    NorthSouth,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Along the camera's direction (see the module's docs).
    pub distance: f32,
    /// The kind of wall (see [`MAP`]), 1 or more.
    pub wall: u8,
    pub side: Side,
    /// Where across the wall's face it hit, from 0 to 1, from the left
    /// as the player sees it (for textures).
    pub along: f32,
}

/// The most steps a ray takes before giving up: more than crossing the
/// map corner to corner takes.
pub const MAX_STEPS: usize = 2 * MAP_SIZE + 2;
/// Nearer than this, a wall is as near as this: the player can't be
/// nearer than [`RADIUS`] anyway, but a ray grazing a corner can be.
pub const NEAREST: f32 = 1.0 / 64.0;

/// Casts the ray through the view's column at `across`, from -1 (its left
/// edge) to 1 (its right), to the wall it hits.
pub fn cast(map: &Map, camera: &Camera, across: f32) -> Hit {
    let ray = (
        camera.facing.0 + camera.plane.0 * across,
        camera.facing.1 + camera.plane.1 * across,
    );
    let mut cell = (libm::floorf(camera.x) as i32, libm::floorf(camera.y) as i32);
    // How far along the ray it takes to cross a cell's width, and its
    // height: never for an axis it runs along (a large number, rather than
    // infinity, which times zero would make NaN).
    let delta = |r: f32| if r == 0.0 { 1e30 } else { (1.0 / r).abs() };
    let delta = (delta(ray.0), delta(ray.1));
    // Which way it steps, and how far along it's the next edge each way.
    let (step_x, mut next_x) = if ray.0 < 0.0 {
        (-1, (camera.x - cell.0 as f32) * delta.0)
    } else {
        (1, (cell.0 as f32 + 1.0 - camera.x) * delta.0)
    };
    let (step_y, mut next_y) = if ray.1 < 0.0 {
        (-1, (camera.y - cell.1 as f32) * delta.1)
    } else {
        (1, (cell.1 as f32 + 1.0 - camera.y) * delta.1)
    };
    let mut side = Side::EastWest;
    let mut kind = 0;
    for _ in 0..MAX_STEPS {
        if next_x < next_y {
            next_x += delta.0;
            cell.0 += step_x;
            side = Side::EastWest;
        } else {
            next_y += delta.1;
            cell.1 += step_y;
            side = Side::NorthSouth;
        }
        kind = wall(map, cell.0, cell.1);
        if kind != 0 {
            break;
        }
    }
    // Off the end of `MAX_STEPS` (a map not closed), as if there were
    // wall where it got to.
    let kind = kind.max(1);
    // The edge just crossed: one delta back from the next.
    let distance = match side {
        Side::EastWest => next_x - delta.0,
        Side::NorthSouth => next_y - delta.1,
    }
    .max(NEAREST);
    // Where on the face: the other coordinate where it hit, flipped so
    // that it runs left to right as seen (a face seen looking east runs
    // north to south, so `y` rises; looking west, it falls).
    let hit_at = match side {
        Side::EastWest => camera.y + distance * ray.1,
        Side::NorthSouth => camera.x + distance * ray.0,
    };
    let mut along = hit_at - libm::floorf(hit_at);
    let flipped = match side {
        Side::EastWest => step_x < 0,
        Side::NorthSouth => step_y > 0,
    };
    if flipped {
        along = 1.0 - along;
    }
    Hit {
        distance,
        wall: kind,
        side,
        along,
    }
}

/// The column's position across the view for ray [`cast`]: the middle
/// of column `x` of `width`, from -1 to 1.
pub fn across(x: usize, width: usize) -> f32 {
    (2 * x + 1) as f32 / width as f32 - 1.0
}

/// The rows of a view `height` tall that a wall `distance` away covers:
/// centred, as tall as the view at a distance of one, and clipped to it.
pub fn wall_rows(distance: f32, height: usize) -> Range<usize> {
    let tall = height as f32 / distance;
    if tall >= height as f32 {
        return 0..height;
    }
    // Rounded to whole rows, the same way at the top and bottom, so that
    // the wall stays centred.
    let gap = ((height as f32 - tall) / 2.0 + 0.5) as usize;
    gap..height - gap
}
