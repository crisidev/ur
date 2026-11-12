//! Replays a night from a decoded field log through the smart alarm, as
//! the watch would have run it: `cargo run --release -- <decoded log>
//! [<reference stages>] [--from HH:MM] [--step MINUTES]`.
//!
//! The log has every accelerometer poll's movement (`activity:` lines,
//! every 2 seconds) and every heart rate reading, so the watch's live view
//! can be rebuilt exactly: each minute, as `crate::activity` does, the
//! history is classified (`sleep`) and staged (`stages`), with the new
//! minute holding only its first poll; then each poll is a check of
//! [`alarm::State::due`], with the wearer stirring at a poll's movement of
//! `sleep::STILL_LIMIT` and in light sleep as `stages::light` said at the
//! minute's start.
//!
//! For each wake time, every `--step` minutes (2) from `--from` (05:00
//! local) until the wearer woke (the log's first alarm ringing after
//! `--from`, or its end), a fresh smart alarm at that time runs over its
//! window. Each prints when it rang, how far into the window, and, with
//! `reference` (`<UTC minute> <stage>` lines, as from
//! `scripts/hc-stages.py`), the reference's stage then (or the minute
//! before, as a Garmin's sleep ends at the minute it's woken). Then a
//! tally. Windows overlap, so their rings repeat: a tally of 100 windows
//! is not 100 mornings.

use std::collections::BTreeMap;

use crate::alarm::{self, Phone, Settings, State, Wearer};
use crate::sleep::{self, Kind, Minute};
use crate::stages;

/// The firmware's history, up to minute `newest`, for staging.
struct Record<'a> {
    kinds: &'a mut [Kind],
    minutes: &'a [Minute],
    heart: &'a [u8],
    newest: i64,
}

impl stages::Record for Record<'_> {
    fn minute(&self, minute: i64) -> (Kind, Option<u16>, u8) {
        if !(0..=self.newest).contains(&minute) {
            return (Kind::Unknown, None, 0);
        }
        let t = minute as usize;
        let m = self.minutes[t];
        (
            self.kinds[t],
            m.seen.then_some(m.movement),
            self.heart.get(t).copied().unwrap_or(0),
        )
    }

    fn newest(&self) -> i64 {
        self.newest
    }

    fn set_stage(&mut self, minute: i64, kind: Kind) {
        self.kinds[minute as usize] = kind;
    }
}

/// One poll: UTC seconds and movement.
struct Poll {
    utc: f64,
    movement: u16,
}

struct Night {
    polls: Vec<Poll>,
    /// The first UTC minute of `light`.
    first: i64,
    /// Whether `stages::light` held at each minute's start.
    light: Vec<bool>,
    /// Local time minus UTC, in seconds.
    offset: i64,
    /// Local seconds of each `alarm: ringing` line.
    rang: Vec<i64>,
}

/// A minute's polls: movement × samples, samples, steps, whether worn,
/// and the first poll's movement and steps.
type Sums = (u64, u64, u16, Option<bool>, (u16, u16));

fn parse(path: &str) -> Night {
    let text = std::fs::read_to_string(path).expect("a readable log");
    let mut anchor: Option<(f64, f64)> = None;
    let mut offset = None;
    let mut polls = Vec::new();
    let mut rang = Vec::new();
    let mut minutes: BTreeMap<i64, Sums> = BTreeMap::new();
    let mut heart: BTreeMap<i64, u8> = BTreeMap::new();
    let mut worn = None;
    for line in text.lines() {
        let Some((uptime, rest)) = line.split_once(' ') else {
            continue;
        };
        let Ok(uptime) = uptime.parse::<f64>() else {
            continue;
        };
        let message = rest.trim_start_matches(|c: char| c.is_ascii_uppercase() || c == ' ');
        if message.contains("booting") {
            anchor = None;
        }
        // "clock: 1794350477 UTC = 2026-11-10 23:41:17 local"
        if let Some(rest) = message.strip_prefix("clock: ") {
            let mut words = rest.split(' ');
            if let Some(utc) = words.next().and_then(|u| u.parse::<i64>().ok()) {
                anchor = Some((uptime, utc as f64));
                let clock = words.nth(3).and_then(|hms| {
                    let mut parts = hms.split(':').map(|p| p.parse::<i64>().ok());
                    Some(parts.next()?? * 3600 + parts.next()?? * 60 + parts.next()??)
                });
                if let Some(clock) = clock {
                    // To the quarter hour, within a day either way.
                    let off = (clock - utc.rem_euclid(86_400) + 43_200).rem_euclid(86_400) - 43_200;
                    offset = Some((off as f64 / 900.0).round() as i64 * 900);
                }
            }
            continue;
        }
        let Some(utc) = anchor.map(|(at, utc)| utc + (uptime - at)) else {
            continue;
        };
        let minute = (utc as i64).div_euclid(60);
        if message.starts_with("alarm: ringing") {
            rang.push(utc as i64);
        }
        if let Some(rest) = message.strip_prefix("heart: ") {
            if message.contains(" BPM") && !message.contains("window") {
                worn = Some(true);
                if let Some(bpm) = rest.split_once(" BPM").and_then(|(b, _)| b.parse().ok()) {
                    heart.insert(minute, bpm);
                }
            } else if message.contains("not on a wrist") || message.contains("too much light") {
                worn = Some(false);
            } else if message.contains("no reading") {
                worn = None;
            }
            continue;
        }
        // "activity: 24 samples, movement 33, 0 new steps (counter 178), …"
        let Some(rest) = message
            .strip_prefix("activity: ")
            .filter(|rest| rest.contains(" samples, movement "))
        else {
            continue;
        };
        let numbers: Vec<u64> = rest
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|s| s.parse().ok())
            .collect();
        let [samples, movement, steps, ..] = numbers[..] else {
            continue;
        };
        let movement = movement.min(u64::from(u16::MAX)) as u16;
        let steps = steps.min(u64::from(u16::MAX)) as u16;
        polls.push(Poll { utc, movement });
        let entry = minutes
            .entry(minute)
            .or_insert((0, 0, 0, None, (movement, steps)));
        entry.0 += u64::from(movement) * samples;
        entry.1 += samples;
        entry.2 = entry.2.saturating_add(steps);
        entry.3 = worn;
    }
    let offset = offset.expect("a `clock: … local` line");
    let (Some(&first), Some(&last)) = (minutes.keys().next(), minutes.keys().last()) else {
        panic!("no activity lines with wall-clock anchors in {path}");
    };
    let full: Vec<Minute> = (first..=last)
        .map(|m| match minutes.get(&m) {
            Some(&(weighted, samples, steps, worn, _)) => Minute {
                seen: true,
                movement: (weighted / samples.max(1)).min(u64::from(u16::MAX)) as u16,
                steps,
                worn,
            },
            None => Minute::default(),
        })
        .collect();
    let heart: Vec<u8> = (first..=last)
        .map(|m| heart.get(&m).copied().unwrap_or(0))
        .collect();

    // Each minute's pass, as `activity` runs it at the minute's first
    // poll: the new minute holds that poll only, and readings only of
    // the minutes before.
    let mut kinds = vec![Kind::Unknown; full.len()];
    let mut evidence = full.clone();
    let mut light = Vec::with_capacity(full.len());
    for now in 0..full.len() {
        if let Some(&(_, _, _, worn, (movement, steps))) = minutes.get(&(first + now as i64)) {
            evidence[now] = Minute {
                seen: true,
                movement,
                steps,
                worn,
            };
        }
        if now > 0 {
            evidence[now - 1] = full[now - 1];
        }
        let window = (now + 1).saturating_sub(sleep::WINDOW);
        let start = window.saturating_sub(sleep::CONTEXT);
        let before = if start == 0 {
            Kind::Unknown
        } else {
            kinds[start - 1]
        };
        let mut out = vec![Kind::Unknown; now + 1 - start];
        let before_evidence = start
            .checked_sub(1)
            .map_or(Default::default(), |t| evidence[t]);
        sleep::classify(before, before_evidence, &evidence[start..=now], &mut out);
        kinds[window..=now].copy_from_slice(&out[window - start..]);
        let mut record = Record {
            kinds: &mut kinds,
            minutes: &evidence,
            heart: &heart[..now],
            newest: now as i64,
        };
        let recent = sleep::last_sleep(now + 1, |i| record.kinds[i]);
        if let Some((start, end, _)) = recent
            && now < end + sleep::WINDOW
        {
            stages::stage(&mut record, start as i64, end as i64);
        }
        light.push(stages::light(&record));
    }
    Night {
        polls,
        first,
        light,
        offset,
        rang,
    }
}

fn hhmmss(local: i64) -> String {
    let s = local.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn hhmm(arg: &str) -> i64 {
    let (h, m) = arg.split_once(':').expect("HH:MM");
    h.parse::<i64>().expect("hours") * 60 + m.parse::<i64>().expect("minutes")
}

pub fn main(args: &[String]) {
    let mut paths = Vec::new();
    let (mut from, mut step) = (5 * 60, 2);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--from" => from = hhmm(args.next().expect("--from HH:MM")),
            "--step" => step = args.next().expect("--step N").parse().expect("minutes"),
            _ => paths.push(arg.as_str()),
        }
    }
    let night = parse(paths[0]);
    let reference: BTreeMap<i64, String> = paths
        .get(1)
        .map(|path| {
            std::fs::read_to_string(path)
                .expect("readable stages")
                .lines()
                .filter_map(|l| {
                    let (m, s) = l.split_once(' ')?;
                    Some((m.parse().ok()?, s.trim().to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    let stage_at = |utc: i64| {
        let m = utc.div_euclid(60);
        reference
            .get(&m)
            .or_else(|| reference.get(&(m - 1)))
            .map_or("-", String::as_str)
    };
    let local = |utc: f64| utc as i64 + night.offset;
    let flips = night.light.windows(2).filter(|w| w[0] != w[1]).count();
    println!(
        "{} polls; `sleep: light` changed {flips} times; rang at {:?} (local)",
        night.polls.len(),
        night
            .rang
            .iter()
            .map(|&r| hhmmss(r + night.offset))
            .collect::<Vec<_>>()
    );

    // The morning: from `from` on the day of the log's last poll, to the
    // first ring after it (or the log's end).
    let end = local(night.polls.last().expect("polls").utc);
    let day = end.div_euclid(86_400);
    let morning = day * 86_400 + from * 60;
    let woke = night
        .rang
        .iter()
        .map(|&r| r + night.offset)
        .find(|&r| r > morning)
        .unwrap_or(end);
    let mut tally: BTreeMap<(bool, String), usize> = BTreeMap::new();
    let (mut windows, mut minutes_in) = (0, 0.0);
    let mut time = (morning / 60 + step) * 60;
    while time <= woke {
        let opens = time - alarm::SMART_WINDOW * 60;
        let of_day = (time.rem_euclid(86_400) / 60) as u16;
        let mut settings = Settings {
            smart: true,
            smart_weekday: of_day,
            smart_weekend: of_day,
            ..Settings::default()
        };
        let (mut state, mut phone) = (State::default(), Phone::default());
        // Every poll in the window, then the time itself.
        let checks = night
            .polls
            .iter()
            .filter(|p| (opens..time).contains(&local(p.utc)))
            .map(|p| {
                let minute = (p.utc as i64).div_euclid(60) - night.first;
                let light = night.light.get(minute as usize).copied().unwrap_or(false);
                let wearer = Wearer {
                    stirring: p.movement >= sleep::STILL_LIMIT,
                    light,
                };
                (local(p.utc), wearer)
            })
            .chain([(time, Wearer::default())]);
        let mut rang = None;
        for (at, wearer) in checks {
            if let Some(due) = state.due(&mut settings, &mut phone, at, wearer) {
                rang = Some((at, due.early));
                break;
            }
        }
        let (at, early) = rang.expect("a smart alarm rings by its time");
        let stage = stage_at(at - night.offset);
        let into = (at - opens) as f64 / 60.0;
        println!(
            "  {}: rang {} {:>4.1} min in, {}{stage}",
            &hhmmss(time)[..5],
            hhmmss(at),
            into,
            if early { "early, " } else { "at its time, " },
        );
        *tally.entry((early, stage.to_string())).or_default() += 1;
        windows += 1;
        minutes_in += into;
        time += step * 60;
    }
    println!(
        "{windows} windows; {:.1} minutes in on average; stages at the ring (early / at the time):",
        minutes_in / windows.max(1) as f64
    );
    for stage in ["light", "rem", "deep", "awake", "-"] {
        let n = |early| tally.get(&(early, stage.to_string())).copied().unwrap_or(0);
        if n(true) + n(false) > 0 {
            println!("  {stage:>5}: {} / {}", n(true), n(false));
        }
    }
}
