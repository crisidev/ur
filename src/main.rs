//! `ur`: firmware for the PineTime.
//!
//! This file is board bring-up and the main UI loop. The pieces:
//!
//! - [`canvas`] / [`font`] / [`theme`]: the 80×80 pixel-art canvas, its
//!   glyphs, and colour themes.
//! - [`display`]: the async LCD driver the canvas pushes pixels through.
//! - [`spi`] / [`spi_flash`]: the bus the LCD shares with the external flash.
//! - [`ui`]: screens, navigation, and screen power.
//! - [`touch`], the button task here: input, delivered through [`INPUT`].
//! - [`motor`]: vibration patterns.
//! - [`alarm`]: when alarms (smart wake, at a time, timer) ring.
//! - [`battery`]: battery voltage and charge state.
//! - [`activity`] / [`sleep`]: steps, movement, and sleep, from the
//!   accelerometer.
//! - [`heart`] / [`ppg`]: heart rate from the optical sensor, and the
//!   signal processing behind it.
//! - [`i2c`]: the shared sensor bus.
//! - [`ble`] / [`gb`]: Bluetooth, speaking Gadgetbridge's Bangle.js protocol.
//! - [`clock`] / [`geo`]: wall-clock time and location, both from the phone,
//!   and the sunrise/sunset derived from them.
//! - [`storage`]: settings and Bluetooth bonds in flash.
//! - [`firmware`] / [`dfu`]: cooperating with the bootloader, and receiving
//!   updates over Bluetooth.
//! - [`field_log`]: logs kept in flash on watches without a debug probe.
//! - [`debug`]: screenshots and injected input over the debug probe.
//!
//! Tasks talk to the UI loop only through [`INPUT`]; the UI loop owns the
//! display and everything shown on it.

#![no_std]
#![no_main]

mod activity;
mod alarm;
mod battery;
mod ble;
mod canvas;
mod clock;
mod debug;
mod dfu;
mod display;
mod field_log;
mod firmware;
mod font;
mod gb;
mod geo;
mod glance;
mod heart;
mod i2c;
mod motor;
mod power;
mod ppg;
mod sleep;
mod spi;
mod spi_flash;
mod storage;
mod theme;
mod touch;
mod ui;

use core::cell::Cell;
use core::sync::atomic::{AtomicBool, Ordering};

use defmt::{info, warn};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_futures::select::{Either3, select3};
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use embassy_nrf::interrupt::{self, InterruptExt, Priority};
use embassy_nrf::wdt::{self, HaltConfig, Watchdog};
use embassy_nrf::{bind_interrupts, peripherals, rng, saadc, spim, twim};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Instant, Ticker, Timer};
use nrf_sdc::mpsl;
use static_cell::StaticCell;

// The panic handler is `field_log::panic`.
use defmt_rtt as _;

use crate::battery::Battery;
use crate::canvas::{Canvas, Sweep};
use crate::display::Display;
use crate::touch::Touch;
use crate::ui::{App, Status, ThemeChoice};

bind_interrupts!(struct Irqs {
    SPI2 => spim::InterruptHandler<peripherals::SPI2>;
    TWISPI1 => twim::InterruptHandler<peripherals::TWISPI1>;
    SAADC => saadc::InterruptHandler;
    RNG => rng::InterruptHandler<peripherals::RNG>;
    // Bluetooth (MPSL): radio timing at priority 0, housekeeping on a SWI.
    EGU0_SWI0 => mpsl::LowPrioInterruptHandler;
    CLOCK_POWER => mpsl::ClockInterruptHandler;
    RADIO => mpsl::HighPrioInterruptHandler;
    TIMER0 => mpsl::HighPrioInterruptHandler;
    RTC0 => mpsl::HighPrioInterruptHandler;
});

/// Priority for every interrupt of ours. MPSL needs priority 0 to itself
/// (radio timing must never wait), and Nordic reserves 1 as well, so all
/// application interrupts sit below. embassy-nrf drivers enable their
/// interrupts at the hardware default of 0, so each one is lowered explicitly
/// after the driver is created.
const APP_PRIORITY: Priority = Priority::P2;

// ==============================================================================
// Input
// ==============================================================================

#[derive(defmt::Format)]
pub enum InputEvent {
    Button,
    Touch(Touch),
    /// Something needs the user to look (e.g. a pairing passkey): wake the
    /// screen if it's off, and redraw.
    Attention,
    /// A notification, call, or media update from the phone.
    Phone(ui::PhoneEvent),
    /// The wrist turned to look at the watch (see [`glance`]).
    Glance,
}

/// Every user input, in order, for the UI loop.
pub static INPUT: Channel<CriticalSectionRawMutex, InputEvent, 8> = Channel::new();

/// Whether the side button is down right now. While it is, the UI loop
/// stops feeding the watchdog: see [`firmware`].
static BUTTON_HELD: AtomicBool = AtomicBool::new(false);

/// How often to look at the button. A press lasts 100 ms or more, so this
/// can't miss one, and it's at most this much slower to respond.
const BUTTON_POLL: Duration = Duration::from_millis(50);

/// Watches the side button and reports each (debounced) press.
///
/// The button only works while P0.15 (`enable`) is driven high, and powering
/// it costs ~34 µA continuously (measured by aykevl for the PINE64 wiki),
/// more than the nRF52 and the accelerometer's step counter together. So it
/// is powered only for a moment every [`BUTTON_POLL`] to read it, and
/// continuously only while it's held. The polls cost about a µA in all.
#[embassy_executor::task]
async fn button_task(mut enable: Output<'static>, mut button: Input<'static>) {
    loop {
        enable.set_high();
        // The line settles within a few µs of being powered (it's a switch
        // and a pull-down); ~5 µs at 64 MHz.
        cortex_m::asm::delay(320);
        let pressed = button.is_high();
        if !pressed {
            enable.set_low();
            Timer::after(BUTTON_POLL).await;
            continue;
        }
        BUTTON_HELD.store(true, Ordering::Relaxed);
        INPUT.send(InputEvent::Button).await;
        // The button is mechanical, so ignore bounces on both the press and
        // the release before arming for the next press.
        Timer::after_millis(20).await;
        button.wait_for_low().await;
        BUTTON_HELD.store(false, Ordering::Relaxed);
        enable.set_low();
        Timer::after_millis(20).await;
    }
}

/// Local seconds since 1970 (UTC plus the phone's UTC offset), once the
/// phone has set the clock; what [`alarm`] works in.
fn local_second() -> Option<i64> {
    let utc = clock::now_utc_seconds()?;
    let offset = i64::from(clock::utc_offset_minutes()?);
    Some(utc + offset * 60)
}

/// The three active-low backlight transistors (low/mid/high), which sum.
struct Backlight {
    low: Output<'static>,
    mid: Output<'static>,
    high: Output<'static>,
}

impl Backlight {
    /// Mid alone is plenty indoors.
    /// TODO: brightness setting, or adapt to time of day.
    fn on(&mut self) {
        self.low.set_high();
        self.mid.set_low();
        self.high.set_high();
    }

    /// Low alone: enough to read the glance's white on black.
    fn dim(&mut self) {
        self.low.set_low();
        self.mid.set_high();
        self.high.set_high();
    }

    fn off(&mut self) {
        self.low.set_high();
        self.mid.set_high();
        self.high.set_high();
    }
}

/// The thread-mode executor, by hand rather than `#[embassy_executor::main]`,
/// so that the sleep between polls can work around the FPU erratum and be
/// timed (see [`power::sleep`]).
#[cortex_m_rt::entry]
fn entry() -> ! {
    static EXECUTOR: StaticCell<embassy_executor::raw::Executor> = StaticCell::new();
    // `usize::MAX` is the context embassy's own thread-mode executor uses, so
    // its `__pender` (a SEV) is what wakes this loop when a task is woken.
    let executor: &'static _ =
        EXECUTOR.init(embassy_executor::raw::Executor::new(usize::MAX as *mut ()));
    let spawner = executor.spawner();
    spawner.spawn(main(spawner).expect("spawned once"));
    loop {
        // SAFETY: Only ever polled from here, in thread mode.
        unsafe { executor.poll() };
        power::sleep();
    }
}

#[embassy_executor::task]
async fn main(spawner: Spawner) {
    firmware::clean_up_after_bootloader();
    let mut config = embassy_nrf::config::Config::default();
    // The DC/DC regulator instead of the LDO: the same power at roughly half
    // the current while the CPU or radio runs. It needs an inductor, which
    // the PineTime has; InfiniTime, Mynewt, and TinyGo all enable it on this
    // board. (Without one, the chip would brown out at boot.)
    config.dcdc.reg1 = true;
    // The PineTime has a 32.768 kHz crystal. embassy defaults to the internal
    // RC oscillator, which is far less accurate, and the wall clock is kept by
    // RTC1 off this clock.
    config.lfclk_source = embassy_nrf::config::LfclkSource::ExternalXtal;
    config.gpiote_interrupt_priority = APP_PRIORITY;
    config.time_interrupt_priority = APP_PRIORITY;
    let mut p = embassy_nrf::init(config);
    info!("ur {=str} booting", env!("UR_VERSION"));
    clock::restore();
    field_log::report_last_reset();

    // ==========================================================================
    // Watchdog
    // ==========================================================================

    // The WDT survives soft resets, and the bootloader starts it (7 s) before
    // handing over, so normally it's already running and we must adopt its
    // exact configuration or keep getting reset. Otherwise (only when
    // something boots us without the bootloader) we start our own so a hung
    // UI loop recovers.
    let wdt_config = wdt::Config::try_new(&p.WDT).unwrap_or_else(|| {
        let mut config = wdt::Config::default();
        config.timeout_ticks = 5 * 32768; // 5 s
        // Don't reset out from under a debugger sitting at a breakpoint. The
        // flip side is that a core left halted by a debug session (e.g. a
        // killed `esprobe gdb`) stays frozen rather than recovering.
        //
        // TODO: switch to `HaltConfig::Run` once this isn't primarily a
        // development build, so a stray halt self-heals.
        config.action_during_debug_halt = HaltConfig::Pause;
        config
    });
    // Before adopting (which consumes the peripheral): note where we were
    // if it ever bites.
    field_log::catch_watchdog();
    let mut watchdog = match Watchdog::try_new::<_, 1>(p.WDT, wdt_config) {
        Ok((_wdt, [handle])) => Some(handle),
        Err(_) => {
            // Already running with more than one reload register enabled. We
            // can't feed it, so it'll reset us shortly; the WDT reset clears
            // its config and we'll come back up clean.
            warn!("watchdog already running with an incompatible config; expect one reset");
            None
        }
    };

    // ==========================================================================
    // Board bring-up
    // ==========================================================================
    //
    // Pin assignments are from the PineTime schematic:
    // https://wiki.pine64.org/wiki/PineTime#Pinout

    // The SPI flash shares the SPI bus with the LCD (set up below); its
    // chip-select must be high from the start, so it ignores LCD traffic.
    let flash_cs = Output::new(p.P0_05, Level::High, OutputDrive::Standard);

    let mut backlight = Backlight {
        low: Output::new(p.P0_14, Level::High, OutputDrive::Standard),
        mid: Output::new(p.P0_22, Level::High, OutputDrive::Standard),
        high: Output::new(p.P0_23, Level::High, OutputDrive::Standard),
    };

    // The button is only powered while P0.15 is driven high, and that costs
    // ~34 µA, so `button_task` powers it only to read it. It reads high when
    // pressed.
    let button_enable = Output::new(p.P0_15, Level::Low, OutputDrive::Standard);
    spawner
        .spawn(button_task(button_enable, Input::new(p.P0_13, Pull::None)).expect("spawned once"));

    // Vibration motor, active low: start off.
    let motor = Output::new(p.P0_16, Level::High, OutputDrive::Standard);
    spawner.spawn(motor::motor_task(motor).expect("spawned once"));

    // Charger status lines, both active-low with external pull-ups.
    let charging = Input::new(p.P0_12, Pull::None);
    let power_present = Input::new(p.P0_19, Pull::None);

    // Battery voltage arrives on AIN7 (P0.31) through a 1:2 divider. The
    // divider is high-impedance, so give the sample-and-hold capacitor plenty
    // of time to settle.
    let mut battery_channel = saadc::ChannelConfig::single_ended(p.P0_31);
    battery_channel.time = saadc::Time::_40US;
    let mut adc = saadc::Saadc::new(p.SAADC, Irqs, saadc::Config::default(), [battery_channel]);
    interrupt::SAADC.set_priority(APP_PRIORITY);
    adc.calibrate().await;

    // ==========================================================================
    // Sensor bus and touch
    // ==========================================================================

    i2c::recover_bus(p.P0_06.reborrow().into(), p.P0_07.reborrow().into()).await;
    static TWIM_TX_BUFFER: StaticCell<[u8; 16]> = StaticCell::new();
    static I2C_BUS: StaticCell<Mutex<CriticalSectionRawMutex, twim::Twim<'static>>> =
        StaticCell::new();
    let mut twim_config = twim::Config::default();
    // Not 400 kHz: the nRF52832's TWIM slightly violates I2C timing there
    // (Nordic anomaly 219), and the accelerometer occasionally stalled.
    twim_config.frequency = twim::Frequency::K250;
    let twim = twim::Twim::new(
        p.TWISPI1,
        Irqs,
        p.P0_06,
        p.P0_07,
        twim_config,
        TWIM_TX_BUFFER.init([0; 16]),
    );
    interrupt::TWISPI1.set_priority(APP_PRIORITY);
    let i2c_bus: &'static _ = I2C_BUS.init(Mutex::new(twim));
    let _ = i2c::BUS.init(i2c_bus);

    let touch_interrupt = Input::new(p.P0_28, Pull::Up);
    let touch_reset = Output::new(p.P0_10, Level::High, OutputDrive::Standard);
    spawner.spawn(
        touch::touch_task(I2cDevice::new(i2c_bus), touch_interrupt, touch_reset)
            .expect("spawned once"),
    );
    spawner.spawn(activity::activity_task(I2cDevice::new(i2c_bus)).expect("spawned once"));
    spawner.spawn(activity::glance_task(I2cDevice::new(i2c_bus)).expect("spawned once"));
    spawner.spawn(heart::heart_task(I2cDevice::new(i2c_bus)).expect("spawned once"));

    // ==========================================================================
    // Bluetooth
    // ==========================================================================

    // MPSL (Nordic's radio scheduler) must come up before the controller. It
    // takes over the LF clock too, so tell it the same source embassy uses.
    let lfclk = mpsl::raw::mpsl_clock_lfclk_cfg_t {
        source: mpsl::raw::MPSL_CLOCK_LF_SRC_XTAL as u8,
        // Calibration intervals only apply to the RC oscillator.
        rc_ctiv: 0,
        rc_temp_ctiv: 0,
        // The crystal is ±20 ppm; 50 leaves margin for temperature and ageing.
        // Overstating it only costs a little power (wider receive windows).
        accuracy_ppm: 50,
        skip_wait_lfclk_started: false,
    };
    static MPSL: StaticCell<mpsl::MultiprotocolServiceLayer> = StaticCell::new();
    let mpsl_peripherals =
        mpsl::Peripherals::new(p.RTC0, p.TIMER0, p.TEMP, p.PPI_CH19, p.PPI_CH30, p.PPI_CH31);
    // One timeslot session: MPSL's flash driver (used by `storage`) books
    // radio-free time slots for erases and writes, and fails with ENOMEM
    // without session memory to do so.
    static MPSL_SESSIONS: StaticCell<mpsl::SessionMem<1>> = StaticCell::new();
    let sessions = MPSL_SESSIONS.init(mpsl::SessionMem::new());
    let mpsl = MPSL.init(
        mpsl::MultiprotocolServiceLayer::with_timeslots(mpsl_peripherals, Irqs, lfclk, sessions)
            .expect("MPSL initialises once, with valid clock config"),
    );
    spawner.spawn(ble::mpsl_task(mpsl).expect("spawned once"));
    // Flash writes go through MPSL so they don't disturb the radio; see
    // `storage`.
    storage::init(mpsl::Flash::take(mpsl, p.NVMC));

    static RNG: StaticCell<rng::Rng<'static, embassy_nrf::mode::Async>> = StaticCell::new();
    let rng = RNG.init(rng::Rng::new(p.RNG, Irqs));
    interrupt::RNG.set_priority(APP_PRIORITY);

    // The controller's memory needs depend on its configuration; `build`
    // logs the required size if this is wrong (and fails if it's too small).
    static SDC_MEM: StaticCell<nrf_sdc::Mem<5184>> = StaticCell::new();
    let sdc_mem = SDC_MEM.init(nrf_sdc::Mem::new());
    let sdc_peripherals = nrf_sdc::Peripherals::new(
        p.PPI_CH17, p.PPI_CH18, p.PPI_CH20, p.PPI_CH21, p.PPI_CH22, p.PPI_CH23, p.PPI_CH24,
        p.PPI_CH25, p.PPI_CH26, p.PPI_CH27, p.PPI_CH28, p.PPI_CH29,
    );
    let packet_size =
        <trouble_host::prelude::DefaultPacketPool as trouble_host::PacketPool>::MTU as u16;
    let controller = nrf_sdc::Builder::new()
        .and_then(|b| {
            b.support_adv()
                .support_peripheral()
                .peripheral_count(ble::CONNECTIONS as u8)
        })
        // Data length extension, so 251-byte packets go out as one radio
        // packet instead of ten 27-byte ones.
        .map(|b| b.support_dle_peripheral())
        .and_then(|b| b.buffer_cfg(packet_size, packet_size, 3, 3))
        .and_then(|b| b.build(sdc_peripherals, rng, mpsl, sdc_mem))
        .expect("controller configuration is valid and its memory is large enough");

    // A stable address, so the phone recognises us after a reboot. FICR holds
    // a factory-random 48-bit address per chip; static random addresses need
    // the top two bits set.
    let ficr = embassy_nrf::pac::FICR;
    let (lo, hi) = (ficr.deviceaddr(0).read(), ficr.deviceaddr(1).read());
    let mut address = [0u8; 6];
    address[..4].copy_from_slice(&lo.to_le_bytes());
    address[4..].copy_from_slice(&hi.to_le_bytes()[..2]);
    address[5] |= 0xC0;
    spawner.spawn(ble::host_task(controller, address).expect("spawned once"));

    // ==========================================================================
    // Display
    // ==========================================================================

    // ST7789 over SPI mode 3. 8 MHz is the nRF52832's SPIM ceiling. A full
    // 240×240×16bpp frame is ~115 KB, i.e. ~115 ms, which is why the canvas
    // only pushes regions that changed, and why the driver is async (other
    // tasks run while pixels go out).
    let mut spi_config = spim::Config::default();
    spi_config.frequency = spim::Frequency::M8;
    spi_config.mode = spim::MODE_3;
    // The SPI flash (mode 0 or 3, up to 120 MHz) is happy with the same.
    let spi = spim::Spim::new(p.SPI2, Irqs, p.P0_02, p.P0_04, p.P0_03, spi_config);
    interrupt::SPI2.set_priority(APP_PRIORITY);
    static SPI_BUS: StaticCell<spi::Bus> = StaticCell::new();
    let _ = spi::BUS.init(SPI_BUS.init(Mutex::new(spi)));
    let lcd_cs = Output::new(p.P0_25, Level::High, OutputDrive::Standard);
    let lcd_dc = Output::new(p.P0_18, Level::Low, OutputDrive::Standard);
    let lcd_reset = Output::new(p.P0_26, Level::High, OutputDrive::Standard);
    let mut display = Display::new(lcd_cs, lcd_dc, lcd_reset).await;
    info!("display initialised");
    *spi_flash::FLASH.lock().await = Some(spi_flash::SpiFlash::new(flash_cs).await);
    #[cfg(feature = "field-log")]
    spawner.spawn(field_log::task().expect("spawned once"));

    // ==========================================================================
    // UI loop
    // ==========================================================================
    //
    // Wakes once a second and on every input. Each pass: update state, then
    // (if the screen is on) render a full frame and push what changed.

    // Static rather than on the stack: two 3.2 KB frame buffers.
    static CANVAS: StaticCell<Canvas> = StaticCell::new();
    let canvas = CANVAS.init(Canvas::new());
    debug::register_canvas(canvas);
    spawner.spawn(debug::input_task().expect("spawned once"));
    let mut battery = Battery::new();
    let load_u8 = async |key| storage::load::<1>(key).await.map_or(0, |[value]| value);
    let themes = ThemeChoice {
        light: load_u8(storage::key::THEME_LIGHT).await,
        dark: load_u8(storage::key::THEME_DARK).await,
    };
    let alarms = storage::load(storage::key::ALARMS)
        .await
        .map(alarm::Settings::from_bytes)
        .unwrap_or_default();
    let mut app = App::new(Instant::now(), themes, alarms);
    if let Some(location) = storage::load(storage::key::LOCATION).await {
        geo::set_location(geo::Location::from_bytes(location));
    }
    // Start "asleep" so the first pass goes through the wake path: render the
    // first frame, then turn the backlight on. (Waking an already-awake
    // ST7789 is harmless.)
    let mut panel_awake = false;
    // The panel's memory holds noise after power-up; sweeping the first frame
    // in covers every block of it, and doubles as a retro boot effect.
    let mut first_frame = true;

    let mut ticker = Ticker::every(Duration::from_secs(1));
    // The first reading is still settling (it's smoothed); log after a minute.
    let mut next_battery_log = Instant::now() + Duration::from_secs(60);
    let mut last_power_report = Instant::now();
    // Since when the panel (and backlight) have been on, for `power`.
    let mut panel_since = Instant::now();
    // Whether the last frame was in the dim look; see below.
    let mut was_dim = false;
    loop {
        let now = Instant::now();
        let mut samples = [0i16; 1];
        adc.sample(&mut samples).await;
        battery.update(samples[0], charging.is_low(), power_present.is_low());
        let report = ble::BatteryReport {
            percent: battery.percent() as u8,
            charging: battery.charging,
            millivolts: battery.millivolts(),
        };
        ble::BATTERY.lock(|cell| cell.set(report));
        // For battery-life analysis from the field log.
        if now >= next_battery_log {
            clock::log_now();
            if now - last_power_report >= Duration::from_secs(60 * 60) {
                if panel_awake {
                    power::add(power::Meter::Panel, panel_since);
                    power::add(power::Meter::Backlight, panel_since);
                    panel_since = now;
                }
                power::report(now - last_power_report);
                last_power_report = now;
            }
            info!(
                "battery: {} mV, {}%, charging: {}",
                report.millivolts, report.percent, report.charging
            );
            next_battery_log = now + Duration::from_secs(10 * 60);
        }

        // Apply screen power changes. Waking renders before the backlight
        // comes on, so the old frame never flashes up.
        if app.awake {
            if !panel_awake {
                display.wake().await;
            }
            let status = Status {
                uptime_secs: now.as_secs(),
                local_time: clock::now(),
                connected: ble::CONNECTED.load(Ordering::Relaxed),
                passkey: ble::PASSKEY.lock(Cell::get),
                update_progress: dfu::PROGRESS.lock(Cell::get),
                steps: activity::steps_today(),
                sleep: activity::last_sleep(),
                local_second: local_second(),
                heart_rate: heart::current(now),
                battery: &battery,
            };
            app.set_daytime(geo::is_daytime());
            app.set_connected(status.connected);
            // Into or out of the dim white-on-black look (a glance, a quiet
            // notification) with the panel lit: go dark for the redraw.
            // Otherwise the old frame (often a light theme, at full
            // backlight) flashes up while the new one is drawn over it.
            let relighting = panel_awake && app.dim() != was_dim;
            if relighting {
                backlight.off();
            }
            was_dim = app.dim();
            canvas.set_theme(app.theme());
            app.render(canvas, &status, now);
            match app.take_transition() {
                // Waking straight into a transition would animate over a
                // stale frame; just show the new one.
                Some(sweep) if panel_awake => canvas.transition(&mut display, sweep).await,
                _ if first_frame => canvas.transition(&mut display, Sweep::LeftToRight).await,
                _ => canvas.flush(&mut display).await,
            }
            first_frame = false;
            // Light only a finished frame: after waking, the output is blank
            // until shown (see `Display::sleep`); after a redraw in the dark,
            // the panel's scan needs a refresh to reach the bottom rows.
            if !panel_awake {
                display.show().await;
            } else if relighting {
                Timer::after(display::REFRESH).await;
            }
            // Dim for a glance (white on black reads fine), and back up if
            // it turns into a proper wake.
            if app.dim() {
                backlight.dim();
            } else {
                backlight.on();
            }
            if !panel_awake {
                panel_awake = true;
                panel_since = Instant::now();
            }
        } else if panel_awake {
            backlight.off();
            power::add(power::Meter::Panel, panel_since);
            power::add(power::Meter::Backlight, panel_since);
            // In sleep the ST7789 keeps its frame memory, so the canvas's
            // idea of what's on the panel stays valid across sleep/wake.
            display.sleep().await;
            panel_awake = false;
        }
        // Someone looking at the clock face gets a fresh heart rate, but one
        // is enough: the LED (3–8 mA) needn't stay on for the rest of the
        // glance once there's a reading from the last few minutes.
        let fresh = heart::latest().is_some_and(|(_, at)| now - at < Duration::from_secs(5 * 60));
        heart::set_watching(app.showing_clock() && !fresh);
        glance::set_screen_on(app.awake);
        // Holding the button long enough to starve the watchdog is the
        // user's way back into the bootloader (and to rollback), even when
        // this firmware is otherwise wedged in a way that still runs this loop.
        if let Some(watchdog) = &mut watchdog
            && !BUTTON_HELD.load(Ordering::Relaxed)
        {
            watchdog.pet();
        }
        clock::checkpoint();

        // Wake for the next second, the next input, or, while text is
        // scrolling, the next animation frame (which only redraws).
        let frame = async {
            if app.animating() {
                Timer::after(canvas::MARQUEE_FRAME).await;
            } else {
                core::future::pending::<()>().await;
            }
        };
        let alarms_before = app.alarms;
        match select3(ticker.next(), INPUT.receive(), frame).await {
            Either3::Third(()) => {}
            Either3::First(()) => {
                // A stir after stillness: what a smart alarm waits for.
                let stirring = activity::movement() >= sleep::STILL_LIMIT;
                app.check_alarms(Instant::now(), local_second(), stirring);
                // Pairing and updates need the user to watch the screen.
                let watching = ble::PASSKEY.lock(Cell::get).is_some()
                    || dfu::PROGRESS.lock(Cell::get).is_some();
                app.on_tick(Instant::now(), watching)
            }
            Either3::Second(event) => {
                info!("input: {}", event);
                if !app.awake {
                    power::count(match event {
                        InputEvent::Button => power::Count::WakeButton,
                        InputEvent::Touch(_) => power::Count::WakeTouch,
                        InputEvent::Phone(_) => power::Count::WakePhone,
                        InputEvent::Attention => power::Count::WakeAttention,
                        InputEvent::Glance => power::Count::WakeGlance,
                    });
                }
                let before = app.themes;
                app.on_input(event, Instant::now());
                if app.themes.light != before.light {
                    storage::save(storage::key::THEME_LIGHT, &[app.themes.light]).await;
                }
                if app.themes.dark != before.dark {
                    storage::save(storage::key::THEME_DARK, &[app.themes.dark]).await;
                }
            }
        }
        // Edited, switched, or (a one-off alarm) rung and switched off.
        if app.alarms != alarms_before {
            storage::save(storage::key::ALARMS, &app.alarms.to_bytes()).await;
        }
    }
}
