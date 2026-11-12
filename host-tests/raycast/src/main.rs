//! Host tests for the firmware's `raycast` module, the `game` feature's
//! world: the map, moving through it, and casting rays across it; and
//! for `game_view`, how it looks. `cargo test` runs them.
//!
//! `cargo run --release -- view.ppm [x y angle]` draws the view from the
//! start (or there), as the watch would, to look at.

#[path = "../../../src/raycast.rs"]
#[allow(dead_code)]
mod raycast;

#[path = "../../../src/game_view.rs"]
#[allow(dead_code)]
mod game_view;

#[path = "../../../src/font.rs"]
#[allow(dead_code)]
mod font;

mod panel;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(out) = args.first() else {
        println!("run `cargo test`, or `cargo run -- view.ppm [x y angle]`");
        return;
    };
    let mut player = raycast::Player::START;
    if let [_, x, y, angle] = &args[..] {
        player = raycast::Player {
            x: x.parse().unwrap(),
            y: y.parse().unwrap(),
            angle: angle.parse().unwrap(),
        };
    }
    let mut panel = panel::Panel::new();
    panel.frame(&player, true, Some((12, 34)), None, true);
    std::fs::write(out, panel.ppm()).unwrap();
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

    use super::raycast::*;

    /// The game's view (`game::VIEW_W` by `VIEW_H`).
    const WIDTH: usize = 162;
    const HEIGHT: usize = 120;

    /// A map with nothing in it but its walls all round.
    fn empty() -> Map {
        let mut map = [[0; MAP_SIZE]; MAP_SIZE];
        map[0] = [1; MAP_SIZE];
        map[MAP_SIZE - 1] = [1; MAP_SIZE];
        for row in &mut map {
            row[0] = 1;
            row[MAP_SIZE - 1] = 1;
        }
        map
    }

    /// Xorshift, for reproducible randomness without a dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        /// In `0.0..1.0`.
        fn unit(&mut self) -> f32 {
            (self.next() >> 40) as f32 / (1u64 << 24) as f32
        }

        fn sign(&mut self) -> i8 {
            (self.next() % 3) as i8 - 1
        }
    }

    fn player(x: f32, y: f32, angle: f32) -> Player {
        Player { x, y, angle }
    }

    fn open(map: &Map, x: f32, y: f32) -> bool {
        wall(map, x.floor() as i32, y.floor() as i32) == 0
    }

    /// Whether `player` stands clear of every wall: every corner of their
    /// square in the open.
    fn clear(map: &Map, player: &Player) -> bool {
        [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
            .iter()
            .all(|(ox, oy)| open(map, player.x + ox * RADIUS, player.y + oy * RADIUS))
    }

    #[test]
    fn the_map_is_closed_all_round() {
        for i in 0..MAP_SIZE {
            for (x, y) in [(i, 0), (i, MAP_SIZE - 1), (0, i), (MAP_SIZE - 1, i)] {
                assert_ne!(MAP[y][x], 0, "({x}, {y}) is open");
            }
        }
        for (x, y) in [(-1, 5), (5, -1), (16, 5), (5, 16), (-100, i32::MAX)] {
            assert_eq!(wall(&MAP, x, y), 1);
        }
        assert!(MAP.iter().flatten().all(|&w| w <= WALL_KINDS));
    }

    /// Every open cell can be walked to from the start: no part of the map
    /// shut away, nor a passage too narrow for the player (a cell is wider
    /// than they are, so any open neighbour can be walked into).
    #[test]
    fn every_open_cell_can_be_reached() {
        let start = Player::START;
        assert!(clear(&MAP, &start));
        let mut seen = [[false; MAP_SIZE]; MAP_SIZE];
        let mut todo = vec![(start.x as usize, start.y as usize)];
        while let Some((x, y)) = todo.pop() {
            if MAP[y][x] != 0 || std::mem::replace(&mut seen[y][x], true) {
                continue;
            }
            todo.extend([(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]);
        }
        for y in 0..MAP_SIZE {
            for x in 0..MAP_SIZE {
                assert!(MAP[y][x] != 0 || seen[y][x], "({x}, {y}) can't be reached");
            }
        }
        const { assert!(RADIUS < 0.5, "and a cell is wide enough for the player") };
    }

    #[test]
    fn cosines_and_sines() {
        let mut worst: f32 = 0.0;
        for i in 0..=100_000 {
            // All the way round, and the very edge of it.
            let angle = (i as f32 * TAU / 100_000.0).min(TAU - f32::EPSILON * 4.0);
            let (cos, sin) = cos_sin(angle);
            worst = worst
                .max((cos - angle.cos()).abs())
                .max((sin - angle.sin()).abs());
        }
        assert!(worst < 2e-6, "off by {worst}");
        assert_eq!(cos_sin(0.0), (1.0, 0.0));
    }

    #[test]
    fn rays_straight_down_the_middle() {
        let camera = Camera::new(&Player::START, WIDTH, HEIGHT);
        // East along the top row, to the far wall's west face.
        let hit = cast(&MAP, &camera, 0.0);
        assert_eq!(hit.side, Side::EastWest);
        assert_eq!(hit.wall, 1);
        assert!((hit.distance - 13.5).abs() < 1e-4, "{hit:?}");
        assert!((hit.along - 0.5).abs() < 1e-4, "{hit:?}");
        // North, to the wall just there.
        let camera = Camera::new(&player(1.5, 1.5, 3.0 * FRAC_PI_2), WIDTH, HEIGHT);
        let hit = cast(&MAP, &camera, 0.0);
        assert_eq!(hit.side, Side::NorthSouth);
        assert!((hit.distance - 0.5).abs() < 1e-4, "{hit:?}");
    }

    /// Across a flat wall faced square on, every column is as far: no
    /// fish-eye.
    #[test]
    fn a_flat_wall_looks_flat() {
        let map = empty();
        for (angle, distance) in [
            (0.0, 6.5),
            (FRAC_PI_2, 6.4),
            (PI, 7.5),
            (3.0 * FRAC_PI_2, 7.6),
        ] {
            let camera = Camera::new(&player(8.5, 8.6, angle), WIDTH, HEIGHT);
            for x in 0..WIDTH {
                let hit = cast(&map, &camera, across(x, WIDTH));
                assert!(
                    (hit.distance - distance).abs() < 1e-3,
                    "facing {angle}, column {x}: {hit:?}"
                );
            }
        }
    }

    /// Where on a wall's face a ray lands runs left to right as the
    /// player sees it, whichever way they face (for textures not to be
    /// mirrored on half the walls).
    #[test]
    fn along_runs_left_to_right() {
        let map = empty();
        for angle in [0.0, FRAC_PI_2, PI, 3.0 * FRAC_PI_2] {
            let camera = Camera::new(&player(8.3, 8.4, angle), WIDTH, HEIGHT);
            let along: Vec<_> = (-5..=5)
                .map(|i| cast(&map, &camera, i as f32 / 100.0).along)
                .collect();
            assert!(
                along.windows(2).all(|w| w[0] < w[1]),
                "facing {angle}: {along:?}"
            );
        }
    }

    /// The edge-to-edge steps agree with marching along the ray in tiny
    /// steps, from all over the map, every way.
    #[test]
    fn rays_agree_with_marching() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut checked = 0;
        while checked < 2_000 {
            let (x, y) = (1.0 + rng.unit() * 14.0, 1.0 + rng.unit() * 14.0);
            let p = player(x, y, rng.unit() * TAU);
            if !clear(&MAP, &p) {
                continue;
            }
            checked += 1;
            let camera = Camera::new(&p, WIDTH, HEIGHT);
            let a = rng.unit() * 2.0 - 1.0;
            let hit = cast(&MAP, &camera, a);
            // The same ray: the way faced, and the plane's part, at right
            // angles, as wide as the view is to its height.
            let (fx, fy) = p.facing();
            let half = WIDTH as f32 / (2 * HEIGHT) as f32;
            let ray = (fx - fy * half * a, fy + fx * half * a);
            // In steps of a thousandth, counted, rather than added up
            // (which would gather rounding errors).
            let mut steps = 0;
            let at = |steps: i32| steps as f32 / 1000.0;
            while open(&MAP, x + at(steps) * ray.0, y + at(steps) * ray.1) {
                steps += 1;
            }
            let t = at(steps);
            // `t` along this ray is the distance along the way faced.
            assert!(
                (t - hit.distance).abs() < 2e-3,
                "from ({x}, {y}) at {}, across {a}: marched {t}, cast {hit:?}",
                p.angle
            );
            let (hx, hy) = (x + t * ray.0, y + t * ray.1);
            let marched = wall(&MAP, hx.floor() as i32, hy.floor() as i32);
            let corner = hit.along < 0.01 || hit.along > 0.99;
            assert!(
                corner || marched == hit.wall,
                "{hit:?}, marched into {marched}"
            );
            assert!((0.0..=1.0).contains(&hit.along));
        }
    }

    #[test]
    fn rays_never_end_inside_the_player() {
        let map = empty();
        // In a corner, every way, every column: walls as near as can be.
        for i in 0..64 {
            let p = player(1.0 + RADIUS, 1.0 + RADIUS, i as f32 * TAU / 64.0);
            let camera = Camera::new(&p, WIDTH, HEIGHT);
            for x in 0..WIDTH {
                let hit = cast(&map, &camera, across(x, WIDTH));
                assert!(hit.distance >= NEAREST && hit.distance.is_finite());
            }
        }
    }

    #[test]
    fn columns_span_the_view() {
        let half = 1.0 / WIDTH as f32;
        assert!((across(0, WIDTH) - (half - 1.0)).abs() < 1e-6);
        assert!((across(WIDTH - 1, WIDTH) - (1.0 - half)).abs() < 1e-6);
        assert!(across(WIDTH / 2, WIDTH) > 0.0 && across(WIDTH / 2 - 1, WIDTH) < 0.0);
    }

    #[test]
    fn walls_shrink_with_distance_and_stay_centred() {
        assert_eq!(wall_rows(1.0, HEIGHT), 0..HEIGHT);
        assert_eq!(wall_rows(NEAREST, HEIGHT), 0..HEIGHT);
        assert_eq!(wall_rows(2.0, HEIGHT), 30..90);
        assert_eq!(wall_rows(4.0, HEIGHT), 45..75);
        let mut last = 0..HEIGHT;
        for i in 1..1000 {
            let rows = wall_rows(1.0 + i as f32 / 10.0, HEIGHT);
            assert_eq!(rows.start, HEIGHT - rows.end, "centred");
            assert!(rows.len() <= last.len(), "never taller further away");
            last = rows;
        }
        assert!(last.len() <= 2);
    }

    #[test]
    fn the_screen_in_thirds() {
        let input = |x, y| Input::from_touch(Some((x, y)), 240);
        let i = |turn, walk| Input { turn, walk };
        assert_eq!(Input::from_touch(None, 240), Input::default());
        assert_eq!(input(120, 120), i(0, 0));
        assert_eq!(input(0, 0), i(-1, 1));
        assert_eq!(input(239, 239), i(1, -1));
        assert_eq!(input(79, 120), i(-1, 0));
        assert_eq!(input(80, 120), i(0, 0));
        assert_eq!(input(159, 120), i(0, 0));
        assert_eq!(input(160, 120), i(1, 0));
        assert_eq!(input(120, 79), i(0, 1));
        assert_eq!(input(120, 160), i(0, -1));
        // Off the panel (it can't, but): the far third.
        assert_eq!(input(255, 255), i(1, -1));
    }

    #[test]
    fn turning() {
        let mut p = Player::START;
        for _ in 0..20 {
            assert!(!step(&MAP, &mut p, Input { turn: 1, walk: 0 }, 0.05));
        }
        assert!((p.angle - TURN_SPEED).abs() < 1e-4, "{p:?}");
        assert_eq!((p.x, p.y), (Player::START.x, Player::START.y));
        // Left past east wraps round.
        let mut p = Player::START;
        step(&MAP, &mut p, Input { turn: -1, walk: 0 }, 0.05);
        assert!((p.angle - (TAU - TURN_SPEED * 0.05)).abs() < 1e-4, "{p:?}");
    }

    #[test]
    fn walking() {
        let mut p = Player::START;
        assert!(!step(&MAP, &mut p, Input { turn: 0, walk: 1 }, 0.1));
        assert!(
            (p.x - 1.5 - WALK_SPEED * 0.1).abs() < 1e-5 && p.y == 1.5,
            "{p:?}"
        );
        assert!(!step(&MAP, &mut p, Input { turn: 0, walk: -1 }, 0.1));
        assert!((p.x - 1.5).abs() < 1e-5, "{p:?}");
        // A long gap between frames moves no further than `MAX_STEP`.
        let mut far = Player::START;
        step(&MAP, &mut far, Input { turn: 1, walk: 1 }, 5.0);
        let mut most = Player::START;
        step(&MAP, &mut most, Input { turn: 1, walk: 1 }, MAX_STEP);
        assert_eq!(far, most);
        // And time going backwards (it can't, but) moves nothing.
        let mut p = Player::START;
        step(&MAP, &mut p, Input { turn: 1, walk: 1 }, -1.0);
        assert_eq!(p, Player::START);
    }

    #[test]
    fn walking_into_a_wall_bumps() {
        // North, into the top wall: nowhere to go.
        let mut p = player(1.5, 1.0 + RADIUS + 0.01, 3.0 * FRAC_PI_2);
        let before = p;
        assert!(step(&MAP, &mut p, Input { turn: 0, walk: 1 }, 0.1));
        assert!(
            (p.y - before.y).abs() < 0.02 && p.y >= 1.0 + RADIUS,
            "{p:?}"
        );
    }

    /// At an angle, a wall stops only the way into it: the player slides
    /// along, and that's no bump.
    #[test]
    fn walls_slide() {
        let mut p = player(5.5, 1.0 + RADIUS + 0.01, 3.0 * FRAC_PI_2 + FRAC_PI_4);
        let before = p;
        for _ in 0..5 {
            assert!(!step(&MAP, &mut p, Input { turn: 0, walk: 1 }, 0.1));
        }
        assert!(p.x > before.x + 0.8, "slid east: {p:?}");
        assert!(p.y >= 1.0 + RADIUS && p.y <= before.y, "{p:?}");
    }

    /// However the player is steered, for however long between frames,
    /// they never end up in a wall, never jump, and always face a way in
    /// `0..TAU`.
    #[test]
    fn never_through_walls() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        let mut p = Player::START;
        let mut held = Input::default();
        for i in 0..200_000 {
            // Inputs held for a while, as a finger would.
            if i % 20 == 0 {
                held = Input {
                    turn: rng.sign(),
                    walk: rng.sign(),
                };
            }
            let seconds = match rng.next() % 10 {
                0 => rng.unit() * 2.0,
                _ => rng.unit() * 0.06,
            };
            let before = p;
            step(&MAP, &mut p, held, seconds);
            assert!(
                clear(&MAP, &p),
                "step {i}: from {before:?} into a wall at {p:?}"
            );
            let moved = ((p.x - before.x).powi(2) + (p.y - before.y).powi(2)).sqrt();
            assert!(
                moved <= WALK_SPEED * MAX_STEP + 1e-5,
                "step {i}: {before:?} to {p:?}"
            );
            assert!((0.0..TAU).contains(&p.angle), "step {i}: {p:?}");
        }
    }

    // ==========================================================================
    // The view
    // ==========================================================================

    use super::game_view::{self, BACKGROUND, Readout, SHADED, SHADES, SIZE, TEXTURES};
    use super::panel::Panel;

    /// Frames sent only where they changed show what whole ones would,
    /// however the player moves, and whatever the HUD does.
    #[test]
    fn changed_rows_are_enough() {
        for stats in [false, true] {
            changed_rows_are_enough_with(stats);
        }
    }

    /// With or without the frame readout (`game-stats`).
    fn changed_rows_are_enough_with(stats: bool) {
        let mut rng = Rng(0x1234_5678_9abc_def1);
        let mut player = Player::START;
        let mut panel = Panel::new();
        let mut readout = Readout::of(0, 0, 0, 0, 0);
        let mut time = None;
        panel.frame(&player, true, time, stats.then_some(&readout), true);
        let (mut sent, mut frames) = (0, 0);
        let mut held = Input::default();
        for i in 0..400 {
            if i % 15 == 0 {
                held = Input {
                    turn: rng.sign(),
                    walk: rng.sign(),
                };
            }
            step(&MAP, &mut player, held, 0.05);
            let time_changed = rng.next().is_multiple_of(20);
            if time_changed {
                time = Some(((rng.next() % 24) as u8, (rng.next() % 60) as u8));
            }
            let readout_changed = rng.next().is_multiple_of(10);
            if readout_changed {
                readout = Readout::of(rng.next() as u32 % 300, 50_000, 60_000, 8_000, 30_000);
            }
            sent += panel.frame_changing(
                &player,
                time,
                stats.then_some(&readout),
                time_changed,
                readout_changed,
            );
            frames += 1;
            let mut whole = Panel::new();
            whole.frame(&player, true, time, stats.then_some(&readout), true);
            assert!(panel.pixels == whole.pixels, "frame {i}, at {player:?}");
        }
        // And it's worth it: well under every pixel, on average.
        let average = sent / frames;
        assert!(average < SIZE * SIZE * 2 / 3, "{average} pixels a frame");
    }

    /// Columns share windows wherever they can: one for a whole frame,
    /// and few in a frame of walls close by.
    #[test]
    fn columns_share_windows() {
        let readout = Readout::of(0, 0, 0, 0, 0);
        let mut panel = Panel::new();
        panel.frame(&Player::START, true, None, Some(&readout), true);
        assert_eq!(panel.windows, 1);
        // Up against a wall, a step back: every column changes, all of it.
        let mut p = player(1.5, 1.0 + RADIUS + 0.01, 3.0 * FRAC_PI_2);
        panel.frame(&p, true, None, Some(&readout), true);
        step(&MAP, &mut p, Input { turn: 0, walk: -1 }, 0.02);
        panel.frame_changing(&p, None, Some(&readout), false, false);
        assert!(panel.windows <= 3, "{} windows", panel.windows);
        // A random walk: far fewer windows than columns.
        let mut rng = Rng(0x0dd_ba11);
        let (mut windows, mut frames) = (0, 0);
        let mut p = Player::START;
        for i in 0..300 {
            let held = Input {
                turn: if i % 40 < 20 { 1 } else { rng.sign() },
                walk: rng.sign(),
            };
            step(&MAP, &mut p, held, 0.05);
            panel.frame_changing(&p, None, Some(&readout), false, false);
            windows += panel.windows;
            frames += 1;
        }
        assert!(
            windows / frames < SIZE / 4,
            "{} windows a frame",
            windows / frames
        );
    }

    #[test]
    fn sending_more_to_share_a_window() {
        use game_view::rows_to_send;
        assert_eq!(rows_to_send(0..0, &(10..50)), 0..0);
        // Inside, with little to spare: the same.
        assert_eq!(rows_to_send(20..40, &(10..50)), 10..50);
        // Too much to spare, or not inside: padded.
        assert_eq!(rows_to_send(100..102, &(0..240)), 96..106);
        assert_eq!(rows_to_send(5..60, &(10..50)), 1..64);
        assert_eq!(rows_to_send(0..240, &(0..0)), 0..240);
    }

    #[test]
    fn far_is_darker() {
        // RGB565's channels, roughly comparable (RGB444's are coarser:
        // shades may round to the same).
        let light = |c: &u16| (c >> 11) + (c >> 5 & 0x3f) / 2 + (c & 0x1f);
        if !game_view::RGB444 {
            for pair in SHADED.windows(2) {
                for (near, far) in pair[0].iter().zip(&pair[1]) {
                    assert!(light(far) <= light(near), "{near:x} to {far:x}");
                }
            }
            for (lit, dark) in SHADED[0].iter().zip(&SHADED[SHADES - 1]) {
                assert!(light(dark) < light(lit), "{lit:x} to {dark:x}");
            }
        }
        assert_eq!(game_view::shade(0.0, false), 0);
        assert_eq!(game_view::shade(0.0, true), 1);
        assert_eq!(game_view::shade(1000.0, false), SHADES - 1);
        assert_eq!(game_view::shade(1000.0, true), SHADES - 1);
        // The ceiling and floor: darkest at the horizon, lightest at the
        // edges, top and bottom.
        assert_ne!(BACKGROUND[0], BACKGROUND[SIZE / 2 - 1]);
        assert_ne!(BACKGROUND[SIZE - 1], BACKGROUND[SIZE / 2]);
    }

    /// Every texture uses all four of its colours, and none is all one.
    #[test]
    fn textures_have_their_colours() {
        for (kind, texture) in TEXTURES.iter().enumerate() {
            let mut seen = [0; 4];
            for column in texture {
                for v in 0..32 {
                    seen[game_view::texel(column, v)] += 1;
                }
            }
            assert!(seen.iter().all(|&n| n > 0), "texture {kind}: {seen:?}");
        }
    }
}
