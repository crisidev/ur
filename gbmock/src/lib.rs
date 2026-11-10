//! A mock of Gadgetbridge's Bangle.js support: what the Android app sends a
//! Bangle.js over the Nordic UART service, and what it makes of what
//! comes back.
//!
//! `ur` presents itself to phones as a Bangle.js, so this is the other end
//! of everything in `src/gb.rs`. It's a model, written from Gadgetbridge's
//! source, to test the watch against without a phone: in the host tests
//! (`host-tests/gb`), under Bolero and Kani there, and in the simulator.
//!
//! # The Gadgetbridge it models
//!
//! ```text
//! commit deffcca650e96ffcccf0dc2f0aa34d8efb34041f
//! Date:  2026-11-10T12:50:14Z
//!
//!     Bangle.js: Allow updating of sample raw kind during upsert (#7296)
//! ```
//!
//! (Also in [`GADGETBRIDGE_COMMIT`], and `Cargo.toml`'s description.)
//! That's `master` of <https://codeberg.org/Freeyourgadget/Gadgetbridge>,
//! 128 commits past its 2026.11.0 tag. Its last three changes to the
//! Bangle.js support are fixes to how activity samples are stored, which
//! phones didn't have before 2026-11-09; [`Fixes`] turns each off, to
//! test against the Gadgetbridge people run.
//!
//! Every behaviour here cites where it comes from, as `file:line` **at
//! that commit**. A path without a directory is in
//! `app/src/main/java/nodomain/freeyourgadget/gadgetbridge/`; one without
//! a file at all is `BangleJSDeviceSupport.java`.
//!
//! # Bringing it up to date
//!
//! Diff the pinned commit against the new one, over exactly the files
//! modelled (from Gadgetbridge's checkout):
//!
//! ```sh
//! cd app/src/main/java/nodomain/freeyourgadget/gadgetbridge
//! git diff deffcca650e96ffcccf0dc2f0aa34d8efb34041f..NEW -- \
//!     service/devices/banglejs/BangleJSDeviceSupport.java \
//!     service/devices/banglejs/BangleJSActivityTrack.java \
//!     devices/banglejs/BangleJSSampleProvider.java \
//!     devices/banglejs/BangleJSCoordinator.java \
//!     devices/banglejs/BangleJSConstants.java \
//!     devices/AbstractSampleProvider.java \
//!     model/ActivityKind.java \
//!     model/ActivitySample.java \
//!     model/Alarm.java \
//!     model/CallSpec.kt \
//!     model/MusicSpec.kt \
//!     model/MusicStateSpec.kt \
//!     model/NotificationSpec.kt \
//!     model/NavigationInfoSpec.kt \
//!     model/CalendarEventSpec.kt \
//!     model/WeatherSpec.kt \
//!     deviceevents/GBDeviceEventMusicControl.java \
//!     deviceevents/GBDeviceEventCallControl.java \
//!     deviceevents/GBDeviceEventNotificationControl.java \
//!     util/MediaManager.java \
//!     util/EmojiConverter.java \
//!     util/LimitedQueue.java \
//!     util/BundleUtils.java \
//!     util/GB.java \
//!     service/DeviceCommunicationService.java \
//!     activities/charts/LiveActivityFragment.java \
//!     activities/HeartRateDialog.java
//! ```
//!
//! Then, for each hunk: find the line range it touches in this crate's
//! citations (`grep -rn ':838' src/`), change the behaviour to match, and
//! move the citations the hunk shifted. `model/ActivityKind.java` is
//! mechanical: `./kinds.sh /path/to/Gadgetbridge > src/kinds.rs`. Last,
//! change the hash, date and title above, in [`GADGETBRIDGE_COMMIT`] and
//! in `Cargo.toml`, and decide whether the change deserves a switch in
//! [`Fixes`] (it does if phones without it will be around for a while and
//! it changes what the watch should do).
//!
//! Android's `org.json` and `android.util.Base64`, and Java's number
//! formatting, aren't in Gadgetbridge's tree: [`json`] and [`java`] follow
//! AOSP and the JDK's documentation, and change with neither.
//!
//! # What's in it
//!
//! - [`Gadgetbridge`] is the app with one watch: its `on…` methods send
//!   what `BangleJSDeviceSupport`'s send, byte for byte
//!   ([`Gadgetbridge::take_sent`], with the writes each is cut into), and
//!   [`Gadgetbridge::receive`] takes the watch's bytes as its
//!   `onCharacteristicChanged` does, into the state the app would keep:
//!   the activity samples in its database after its upsert
//!   ([`Gadgetbridge::samples`]), where the next fetch asks from, what's
//!   live, the battery after smoothing, and an [`Event`] for everything a
//!   user or another app would notice.
//! - [`generate`] makes things for it to do out of a fuzzer's bytes.
//! - [`json`], [`text`] and [`java`] are the pieces: Gadgetbridge's
//!   JavaScript-ish writer and Android's lenient reader, its text
//!   handling (cropping, emoji, "Text as Bitmaps"), and Java's number
//!   formatting.
//!
//! # What isn't
//!
//! Each of these is a `TODO` where it would go:
//!
//! - **Android itself:** text isn't drawn (a "Text as Bitmaps" image has
//!   the right header and size, and noise for pixels), and the emoji
//!   library's table of aliases is a handful of entries.
//! - **Bluetooth:** no pairing, no MTU negotiation (only its effect on
//!   how writes are cut), no flow control beyond dropping XON/XOFF, no
//!   write queue: what's sent arrives.
//! - **What the watch never uses:** recorder logs' contents, HTTP
//!   responses, the calendar's database, weather forecasts, the app
//!   manager's intents, firmware/app installation.
//! - **Time:** the phone's clock moves only when told
//!   ([`Gadgetbridge::now_ms`]); the one timer, a missed call's
//!   notification, runs on [`Gadgetbridge::run_delayed`].
//! - **The rest of the app's triggers:** when it fetches, syncs the time
//!   or sends weather is up to the caller (see
//!   [`generate::connect_sequence`]).

pub mod device;
pub mod generate;
pub mod java;
pub mod json;
pub mod kinds;
pub mod spec;
pub mod text;

pub use device::{
    BatteryState, BusyTask, Event, Fixes, Gadgetbridge, Level, NOT_MEASURED, Prefs, Sample, Sent,
    upsert_merge, upsert_target,
};

/// The Gadgetbridge commit this crate was written from (2026-11-10,
/// "Bangle.js: Allow updating of sample raw kind during upsert (#7296)").
/// See the crate docs for how to move it.
pub const GADGETBRIDGE_COMMIT: &str = "deffcca650e96ffcccf0dc2f0aa34d8efb34041f";
