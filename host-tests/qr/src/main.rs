//! Host tests for the firmware's `qr` module: QR codes for Home
//! Assistant's key. `cargo test` runs them.
//!
//! The expected matrices are `segno`'s (a reference encoder), for the same
//! version, error-correction level, mode and mask:
//!
//! ```python
//! segno.make(text, version=2, error="m", mode="alphanumeric", mask=0,
//!            boost_error=False).matrix
//! ```

#[path = "../../../src/qr.rs"]
#[allow(dead_code)]
mod qr;

fn main() {
    println!("run `cargo test`");
}

#[cfg(test)]
mod tests {
    use super::qr;

    /// `rows` as a matrix: `#` dark, `.` light.
    fn matrix(rows: &[&str]) -> qr::Matrix {
        let mut out = [0; qr::SIZE];
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                if c == '#' {
                    out[y] |= 1 << x;
                }
            }
        }
        out
    }

    fn show(m: &qr::Matrix) -> String {
        m.iter()
            .map(|row| {
                (0..qr::SIZE)
                    .map(|x| if row >> x & 1 != 0 { '#' } else { '.' })
                    .collect::<String>()
                    + "\n"
            })
            .collect()
    }

    fn check(text: &str, rows: &[&str]) {
        let ours = qr::encode(text.as_bytes()).expect("encodable");
        let reference = matrix(rows);
        assert_eq!(show(&ours), show(&reference), "{text}");
    }

    #[test]
    fn a_key() {
        check(
            "231D39C1D7CC1AB1AEE224CD096DB932",
            &[
                "#######...##.##.#.#######",
                "#.....#.#....#.#..#.....#",
                "#.###.#....##...#.#.###.#",
                "#.###.#..#.#......#.###.#",
                "#.###.#.###...#...#.###.#",
                "#.....#..####..##.#.....#",
                "#######.#.#.#.#.#.#######",
                ".........##..###.........",
                "#.#.#.#.....##......#..#.",
                "..##.#....#.##.#....#...#",
                "..#...#....#.#.#.#..#.###",
                "#.##....###...#.##.#.#.#.",
                ".#.#..##..#.##...#..#...#",
                "...##...##.###.#..##.....",
                "#...###...###.#.#..##.##.",
                ".#......#...##.#.#...#...",
                "#....###...#....#########",
                "........####.#..#...##.#.",
                "#######...#....##.#.#.#.#",
                "#.....#..###....#...#.##.",
                "#.###.#.###.#.#######...#",
                "#.###.#....###.#.#.#.##..",
                "#.###.#.#.###.#######...#",
                "#.....#.....##.#..##...##",
                "#######.#####.#.###.###.#",
            ],
        );
    }

    #[test]
    fn an_odd_length() {
        // The last character alone takes 6 bits, not a pair's 11.
        check(
            "HELLO2WORLD",
            &[
                "#######...#...#...#######",
                "#.....#.######....#.....#",
                "#.###.#..##....#..#.###.#",
                "#.###.#...#.##..#.#.###.#",
                "#.###.#.###..#..#.#.###.#",
                "#.....#.....#.###.#.....#",
                "#######.#.#.#.#.#.#######",
                ".........####..#.........",
                "#.#.#.#....##...#...#..#.",
                ".#.##..#..####.###..#.##.",
                "###..####.....###.#..##..",
                "..#.##..###..##.#...##..#",
                "#..######.#...##..#.#.###",
                ".#.#.#..#..#..##...#..#.#",
                "#..#.##......#...#..#....",
                ".#.##....###...#.##..###.",
                "#..####..#..#...#####...#",
                "........##.###.##...#.#.#",
                "#######..####.#.#.#.###..",
                "#.....#...#.###.#...#.#..",
                "#.###.#.#..#..#######.###",
                "#.###.#...##..###.#.#..#.",
                "#.###.#.#....#.#..#.#.#.#",
                "#.....#..#.#.....#.#..###",
                "#######.##..#...#######.#",
            ],
        );
    }

    #[test]
    fn what_doesnt_fit() {
        // Lowercase isn't alphanumeric, and the mode's symbols aren't
        // supported.
        assert!(qr::encode(b"231d39c1").is_none());
        assert!(qr::encode(b"HELLO WORLD").is_none());
        assert!(qr::encode(&[b'A'; qr::MAX_TEXT]).is_some());
        assert!(qr::encode(&[b'A'; qr::MAX_TEXT + 1]).is_none());
    }
}
