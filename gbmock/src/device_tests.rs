//! [`Gadgetbridge`] against lines written out by hand from the Java.

use super::*;

fn connected() -> Gadgetbridge {
    let mut gb = Gadgetbridge::new();
    gb.connect();
    gb.take_sent();
    gb
}

fn lines(gb: &mut Gadgetbridge) -> Vec<String> {
    gb.take_sent()
        .iter()
        .map(|sent| sent.bytes.iter().map(|&b| char::from(b)).collect())
        .collect()
}

#[test]
fn connecting_sets_the_time_and_asks_about_gps() {
    let mut gb = Gadgetbridge::new();
    gb.connect();
    assert_eq!(
        lines(&mut gb),
        [
            "\x10setTime(1794312000);E.setTimeZone(1.0);(s=>s&&(s.timezone=1.0,require('Storage').write('setting.json',s)))(require('Storage').readJSON('setting.json',1))\n",
            "\x10GB({\"t\":\"is_gps_active\"})\n",
        ]
    );
    gb.prefs.sync_time = false;
    gb.zone_offset_ms = -(9 * 60 + 30) * 60_000;
    gb.connect();
    assert_eq!(lines(&mut gb), ["\x10GB({\"t\":\"is_gps_active\"})\n"]);
    gb.on_set_time();
    let sent = lines(&mut gb);
    assert_eq!(sent[0], "\x10GB({\"t\":\"force_calendar_sync_start\"})\n");
    assert!(sent[1].starts_with("\x10setTime(1794312000);E.setTimeZone(-9.5);"));
}

#[test]
fn writes_are_cut_at_the_mtu_which_grows_with_what_arrives() {
    let mut gb = connected();
    gb.on_find_device(true);
    let sent = gb.take_sent();
    assert_eq!(sent[0].json(), Some(&br#"{"t":"find","n":true}"#[..]));
    let packets: Vec<&[u8]> = sent[0].packets().collect();
    assert_eq!(
        packets,
        [&b"\x10GB({\"t\":\"find\",\"n\":"[..], b"true})\n"]
    );
    gb.receive(&[b' '; 60]);
    gb.on_find_device(false);
    assert_eq!(gb.take_sent()[0].packets().count(), 1);
}

#[test]
fn a_notification_as_it_goes_out() {
    let mut gb = connected();
    gb.on_notification(&NotificationSpec {
        id: 17,
        source_name: Some("Signal".into()),
        title: Some("Kari".into()),
        body: Some("Hei \u{1F600}".into()),
        actions: vec![NotificationAction {
            kind: ActionKind::WearableReply,
            title: None,
            handle: 99,
        }],
        ..NotificationSpec::default()
    });
    assert_eq!(
        lines(&mut gb),
        [
            "\x10GB({\"t\":\"notify\",\"id\":17,\"src\":\"Signal\",\"title\":\"Kari\",\"subject\":\"\",\"body\":\"Hei :-D\",\"sender\":\"\",\"reply\":true})\n"
        ]
    );
    // The watch's reply finds the action's handle.
    gb.receive(b"{\"t\":\"notify\",\"n\":\"reply\",\"id\":17,\"msg\":\"ok\"}\r\n");
    assert_eq!(
        gb.take_events(),
        [Event::NotificationControl {
            event: "REPLY",
            handle: 99,
            phone_number: None,
            reply: Some("ok".into()),
        }]
    );
}

#[test]
fn the_character_before_a_line_feed_is_dropped_whatever_it_is() {
    let mut gb = connected();
    gb.receive(b"{\"t\":\"findPhone\",\"n\":true}\r\n");
    assert_eq!(gb.take_events(), [Event::FindPhone(true)]);
    // Without the `\r`, the closing brace goes instead.
    gb.receive(b"{\"t\":\"findPhone\",\"n\":true}\n");
    assert!(matches!(gb.take_events()[..], [Event::MalformedJson(_)]));
    // Split anywhere, with flow control in it.
    gb.receive(b"{\"t\":\"find\x11Phone\",");
    assert!(gb.has_partial_line());
    gb.receive(b"\"n\":false}\r");
    gb.receive(b"\n\r\n");
    assert_eq!(gb.take_events(), [Event::FindPhone(false)]);
}

#[test]
fn a_partial_line_survives_a_reconnect() {
    let mut gb = connected();
    gb.receive(b"{\"t\":\"act\",\"ts\":17943");
    gb.disconnect();
    gb.connect();
    gb.receive(b"{\"t\":\"ver\",\"fw\":\"ur 1.0\",\"hw\":\"PineTime\"}\r\n");
    assert!(matches!(gb.take_events()[..], [Event::MalformedJson(_)]));
    assert_eq!(gb.firmware, None);
}

#[test]
fn an_unknown_command_name_is_an_uncaught_exception() {
    let mut gb = connected();
    gb.receive(b"{\"t\":\"music\",\"n\":\"shuffle\"}\r\n{\"t\":\"music\",\"n\":\"next\"}\r\n");
    // The second line waits for the next packet.
    assert!(matches!(gb.take_events()[..], [Event::Uncaught(_)]));
    gb.receive(b"");
    assert_eq!(gb.take_events(), [Event::MusicControl("NEXT")]);
}

#[test]
fn the_battery_level_only_moves_the_way_its_state_says() {
    let mut gb = connected();
    let level = |gb: &mut Gadgetbridge, line: &[u8]| {
        gb.receive(line);
        match gb.take_events()[..] {
            [Event::Battery { level, .. }] => level,
            _ => panic!("a battery event"),
        }
    };
    assert_eq!(
        level(
            &mut gb,
            b"{\"t\":\"status\",\"bat\":50,\"chg\":0,\"volt\":3.85}\r\n"
        ),
        Some(50)
    );
    assert_eq!(
        level(&mut gb, b"{\"t\":\"status\",\"bat\":52,\"chg\":0}\r\n"),
        Some(50)
    );
    assert_eq!(
        level(&mut gb, b"{\"t\":\"status\",\"bat\":49,\"chg\":0}\r\n"),
        Some(49)
    );
    assert_eq!(
        level(&mut gb, b"{\"t\":\"status\",\"bat\":40,\"chg\":1}\r\n"),
        Some(49)
    );
    assert_eq!(
        level(&mut gb, b"{\"t\":\"status\",\"bat\":140,\"chg\":1}\r\n"),
        Some(100)
    );
    gb.connect();
    assert_eq!(
        level(&mut gb, b"{\"t\":\"status\",\"bat\":30,\"chg\":0}\r\n"),
        Some(30)
    );
}

fn sample(timestamp: i32, steps: i32) -> Sample {
    Sample {
        timestamp,
        raw_kind: kinds::ACTIVITY,
        heart_rate: 0,
        steps,
        raw_intensity: NOT_MEASURED,
    }
}

#[test]
fn samples_within_two_minutes_merge() {
    let mut gb = Gadgetbridge::new();
    gb.upsert_sample(sample(1000, 10));
    gb.upsert_sample(sample(1121, 20));
    assert_eq!(gb.samples.len(), 2);
    // Exactly two minutes is still "near": the steps are replaced, the
    // timestamp isn't.
    gb.upsert_sample(sample(880, 30));
    assert_eq!(gb.samples.len(), 2);
    assert_eq!(gb.samples[&1000].steps, 30);
    // Zero steps and no heart rate change nothing.
    gb.upsert_sample(sample(1000, 0));
    assert_eq!(gb.samples[&1000].steps, 30);
}

#[test]
fn the_sample_merged_into_isnt_always_the_nearest() {
    // Stored at t-10 and t+100: the loop moves to t+100, as it's nearer
    // the new sample (100) than it is to t-10 (110). A new sample
    // between two stored ones always goes into the later.
    assert_eq!(upsert_target(&[990, 1100], 1000), 1);
    assert_eq!(upsert_target(&[990, 1120], 1000), 1);
    // Only one before both can stay with the earlier.
    assert_eq!(upsert_target(&[1010, 1100], 1000), 0);
    assert_eq!(upsert_target(&[880, 1001], 1000), 1);
}

#[test]
fn what_the_fixes_changed() {
    let light = Sample {
        raw_kind: 2,
        raw_intensity: 40,
        ..sample(1000, 0)
    };
    let awake = Sample {
        raw_intensity: NOT_MEASURED,
        ..sample(1000, 5)
    };
    let mut gb = Gadgetbridge::new();
    gb.upsert_sample(light);
    gb.upsert_sample(awake);
    assert_eq!(
        gb.samples[&1000],
        Sample {
            steps: 5,
            raw_intensity: 40,
            ..awake
        }
    );
    gb.fixes = Fixes::NONE;
    gb.samples.clear();
    gb.upsert_sample(light);
    gb.upsert_sample(awake);
    assert_eq!(
        gb.samples[&1000],
        Sample {
            steps: 5,
            raw_intensity: NOT_MEASURED,
            ..light
        }
    );

    // The end of a fetch, and an empty track list after it.
    for (fixes, events) in [
        (
            Fixes::ALL,
            vec![Event::NewData, Event::UnsetBusyWhenIdle, Event::NewData],
        ),
        (
            Fixes::NONE,
            vec![
                Event::UnsetBusyWhenIdle,
                Event::NewData,
                Event::Toast {
                    level: Level::Info,
                    text: "End : Fetching sports details".into(),
                },
            ],
        ),
    ] {
        let mut gb = connected();
        gb.fixes = fixes;
        gb.receive(b"{\"t\":\"actfetch\",\"state\":\"start\"}\r\n");
        assert_eq!(gb.busy, Some(BusyTask::FetchingActivity));
        gb.receive(
            b"{\"t\":\"actfetch\",\"state\":\"end\"}\r\n{\"t\":\"actTrksList\",\"list\":[]}\r\n",
        );
        assert_eq!(gb.take_events(), events);
        assert_eq!(gb.last_sync_ms, gb.now_ms - 60_000);
    }
}

#[test]
fn activity_samples_as_they_are_read() {
    let mut gb = connected();
    gb.receive(
        b"{\"t\":\"act\",\"ts\":1794311999000,\"stp\":12,\"mov\":7,\"hrm\":61,\"act\":\"light_sleep\"}\r\n",
    );
    gb.receive(b"{\"t\":\"act\",\"ts\":1794312299000,\"stp\":3,\"mov\":0,\"hrm\":0}\r\n");
    gb.receive(b"{\"t\":\"act\",\"ts\":1794312599999,\"act\":\"NAPPING\"}\r\n");
    let stored: Vec<_> = gb
        .samples
        .values()
        .map(|s| {
            (
                s.timestamp,
                s.kind(),
                s.steps,
                s.heart_rate,
                s.raw_intensity,
            )
        })
        .collect();
    assert_eq!(
        stored,
        [
            (1794311999, "LIGHT_SLEEP", 12, 61, 7),
            (1794312299, "ACTIVITY", 3, 0, 0),
            (1794312599, "UNKNOWN", 0, 0, NOT_MEASURED),
        ]
    );
    // Live samples aren't stored, and only passed on while live.
    gb.take_events();
    gb.receive(b"{\"t\":\"act\",\"stp\":2,\"hrm\":70,\"rt\":1}\r\n");
    assert_eq!(gb.take_events(), [Event::SleepAsAndroidHeartRate(70)]);
    gb.live_activity(true);
    gb.receive(b"{\"t\":\"act\",\"stp\":2,\"hrm\":70,\"rt\":1}\r\n");
    assert!(matches!(
        gb.take_events()[..],
        [
            _,
            Event::RealtimeSample(Sample {
                steps: 2,
                heart_rate: 70,
                ..
            })
        ]
    ));
    assert_eq!(gb.samples.len(), 3);
}

#[test]
fn live_activity_flags_as_the_screens_set_them() {
    let mut gb = connected();
    gb.live_activity(true);
    gb.live_activity(false);
    assert_eq!(
        lines(&mut gb),
        [
            "\x10GB({\"t\":\"act\",\"hrm\":false,\"stp\":true,\"int\":10})\n",
            "\x10GB({\"t\":\"act\",\"hrm\":true,\"stp\":true,\"int\":10})\n",
            "\x10GB({\"t\":\"act\",\"hrm\":true,\"stp\":false,\"int\":10})\n",
            "\x10GB({\"t\":\"act\",\"hrm\":false,\"stp\":false,\"int\":10})\n",
        ]
    );
    // The heart rate dialog first, and steps can't be turned on.
    gb.on_enable_realtime_heart_rate_measurement(true);
    gb.take_sent();
    gb.on_enable_realtime_steps(true);
    assert!(gb.take_sent().is_empty());
    assert!(!gb.realtime_step);
    // Nothing is sent again after a reconnect.
    gb.disconnect();
    gb.connect();
    gb.take_sent();
    gb.on_enable_realtime_heart_rate_measurement(true);
    assert!(gb.take_sent().is_empty());
}

#[test]
fn intents_are_refused_with_an_answer_unless_allowed() {
    let line = b"{\"t\":\"intent\",\"action\":\"ur.watch.BUTTON\",\"extra\":{\"button\":3,\"gesture\":\"tap\"}}\r\n";
    let mut gb = connected();
    gb.receive(line);
    assert_eq!(
        lines(&mut gb),
        [
            "\x10GB({\"t\":\"intent\",\"err\":\"Android Intents not enabled, check Gadgetbridge Device Settings\"})\n"
        ]
    );
    assert!(gb.take_events().is_empty());
    gb.prefs.intents = true;
    gb.receive(line);
    assert_eq!(
        gb.take_events(),
        [Event::Intent {
            target: "broadcastreceiver".into(),
            action: Some("ur.watch.BUTTON".into()),
            package: None,
            class: None,
            extras: vec![
                ("button".into(), Json::Int(3)),
                ("gesture".into(), Json::Str("tap".into()))
            ],
        }]
    );
}

#[test]
fn calls_music_and_the_rest_as_they_go_out() {
    let mut gb = connected();
    gb.on_set_call_state(&CallSpec {
        command: CallSpec::CALL_INCOMING,
        name: None,
        number: Some("+4712345678".into()),
    });
    gb.on_set_call_state(&CallSpec {
        command: CallSpec::CALL_END,
        ..CallSpec::default()
    });
    let music = MusicSpec {
        artist: Some("R\u{f6}yksopp".into()),
        track: Some("Eple".into()),
        duration: 216,
        track_count: 10,
        track_nr: 2,
        ..MusicSpec::default()
    };
    gb.on_set_music_info(&music);
    gb.on_set_music_info(&music);
    gb.on_set_music_state(MusicStateSpec {
        state: 0,
        position: 12,
        ..MusicStateSpec::default()
    });
    gb.on_set_alarms(&[
        Alarm {
            hour: 7,
            minute: 30,
            repetition: 31,
            enabled: true,
            unused: false,
        },
        Alarm {
            hour: 0,
            minute: 0,
            repetition: 0,
            enabled: false,
            unused: true,
        },
    ]);
    gb.on_set_navigation_info(&NavigationInfoSpec {
        instruction: Some("Storgata".into()),
        distance_to_turn: Some("150 m".into()),
        next_action: 2,
        eta: None,
    });
    gb.on_set_navigation_info(&NavigationInfoSpec {
        next_action: 18,
        ..NavigationInfoSpec::default()
    });
    gb.prefs.use_updated_location = true;
    gb.on_set_gps_location(&Location {
        latitude: 51.4779,
        longitude: -0.0005,
        altitude: 46.0,
        speed: 1.5,
        bearing: None,
        time: 1794312000123,
        satellites: None,
        accuracy: 3.9,
        provider: Some("fused".into()),
    });
    gb.on_fetch_activity();
    gb.last_sync_ms = 1794311000000;
    gb.on_fetch_activity();
    gb.on_full_sync();
    assert_eq!(
        lines(&mut gb),
        [
            "\x10GB({\"t\":\"call\",\"cmd\":\"incoming\",\"number\":\"+4712345678\"})\n",
            "\x10GB({\"t\":\"call\",\"cmd\":\"end\"})\n",
            "\x10GB({\"t\":\"musicinfo\",\"artist\":\"R\u{f6}yksopp\",\"track\":\"Eple\",\"dur\":216,\"c\":10,\"n\":2})\n",
            "\x10GB({\"t\":\"musicstate\",\"state\":\"play\",\"position\":12,\"shuffle\":-1,\"repeat\":-1})\n",
            "\x10GB({\"t\":\"alarm\",\"d\":[{\"h\":7,\"m\":30,\"rep\":31,\"on\":true}]})\n",
            "\x10GB({\"t\":\"nav\",\"instr\":\"Storgata\",\"distance\":\"150 m\",\"action\":\"left\"})\n",
            "\x10GB({\"t\":\"nav\"})\n",
            "\x10GB({\"t\":\"gps\",\"lat\":51.4779,\"lon\":-5.0E-4,\"alt\":46.0,\"speed\":5.4,\"time\":1794312000123,\"satellites\":0,\"hdop\":3.9000000953674316,\"externalSource\":true,\"gpsSource\":\"fused\"})\n",
            "\x10GB({\"t\":\"actfetch\",\"ts\":1794225600000})\n",
            "\x10GB({\"t\":\"actfetch\",\"ts\":1794311000000})\n",
            "\x10GB({\"t\":\"actfetch\",\"ts\":0})\n",
        ]
    );
}
