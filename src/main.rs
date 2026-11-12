//! `ur`: firmware for the PineTime.
//!
//! This file is board bring-up and the main UI loop. The pieces:
//!
//! - [`canvas`] / [`font`] / [`palette`]: the 80×80 pixel-art canvas, its
//!   glyphs, and colour palettes.
//! - [`display`]: the async LCD driver the canvas pushes pixels through.
//! - [`spi`] / [`spi_flash`]: the bus the LCD shares with the external flash.
//! - [`ui`] / [`nav`]: screens, navigation, and screen power;
//!   [`ui_loop`]: what the UI loop does each pass, which the simulator runs
//!   too; [`themes`]: how every screen is drawn, and in which colours.
//! - [`buttons`] / [`bthome`]: the button grid, and its presses as
//!   advertisements for Home Assistant.
//! - [`brightness`]: how bright the backlight is.
//! - [`touch`] / [`gesture`], the button task here: input, delivered
//!   through [`INPUT`].
//! - [`motor`]: vibration patterns.
//! - [`alarm`]: when alarms (smart wake, at a time, from the phone, timer)
//!   ring.
//! - [`battery`]: battery voltage and charge state.
//! - [`system_off`] / [`snapshot`]: switching off when the battery is
//!   critically low, or for storage, saving what the phone hasn't fetched.
//! - [`activity`] / [`sleep`]: steps, movement, and sleep, from the
//!   accelerometer; [`minute_ring`]: which minutes their history keeps as
//!   the clock moves; [`day`]: the day's activity by the hour, for the UI.
//! - [`heart`] / [`ppg`] / [`heart_schedule`]: heart rate from the optical
//!   sensor, the signal processing behind it, and when to measure.
//! - [`i2c`] / [`backoff`]: the shared sensor bus, and retrying a sensor's
//!   setup until it works.
//! - [`ble`] / [`gb`] / [`link`]: Bluetooth, speaking Gadgetbridge's Bangle.js
//!   protocol, and keeping its link working.
//! - [`clock`] / [`geo`] / [`sun`]: wall-clock time and location, both from
//!   the phone, and the sunrise/sunset derived from them.
//! - [`temperature`] / [`aranet`] / [`bthome_sensor`]: the room's
//!   temperature for the clock face, from a nearby room sensor (an Aranet4
//!   air monitor, or a BTHome thermometer; their readings get a screen of
//!   their own) or estimated from the watch's own.
//! - [`storage`]: settings and Bluetooth bonds in flash.
//! - [`firmware`] / [`dfu`] / [`mcuboot`]: cooperating with the bootloader,
//!   and receiving updates over Bluetooth, checked before they're installed.
//! - [`field_log`]: logs kept in flash on watches without a debug probe.
//! - [`debug`]: screenshots over the debug probe, and with the
//!   `debug-input` feature, injected input; and with the `compare`
//!   feature, `compare`: what the UI does and renders with, for checking
//!   the simulator against a dev kit.
//!
//! Tasks talk to the UI loop only through [`INPUT`]; the UI loop owns the
//! display and everything shown on it.

#![no_std]
#![no_main]
// What every theme may draw with (the canvas and fonts, each screen's
// view, the helpers in `themes`, and what they read of the rest) is
// offered whole: a theme draws with what it likes, and needn't use, nor
// can it delete, the rest. Retro uses all of it, so in a build with
// Retro (the default, `themes-all`, and every one CI lints), anything
// unused is dead and warned about as ever. Without it (another theme's
// package), what that theme leaves unused isn't: only the theme's own
// code is held to it (its `mod.rs` warns again; see Retro's).
#![cfg_attr(
    not(feature = "theme-retro"),
    allow(
        dead_code,
        reason = "a theme alone needn't use every part of what themes draw with"
    )
)]

mod activity;
mod alarm;
mod aranet;
mod arrival;
mod backoff;
mod battery;
mod ble;
mod brightness;
mod bthome;
mod bthome_sensor;
mod buttons;
mod calendar;
mod canvas;
mod clock;
#[cfg(feature = "compare")]
mod compare;
mod day;
mod debug;
mod decimal;
mod dfu;
mod dfu_slot;
mod display;
mod field_log;
mod firmware;
#[cfg(feature = "flash-stress")]
mod flash_stress;
mod font;
mod gb;
mod gb_buckets;
mod geo;
mod gesture;
mod glance;
mod heart;
mod heart_reading;
mod heart_schedule;
mod i2c;
mod link;
mod live;
mod mcuboot;
mod minute_ring;
mod motor;
mod nav;
mod nvmc;
mod on_time;
mod packet_pool;
mod palette;
mod phone;
mod power;
mod ppg;
mod qr;
mod route;
#[cfg(feature = "field-log")]
mod rtt_ring;
mod sleep;
mod sleep_hold;
mod snapshot;
mod spi;
mod spi_flash;
mod stages;
mod step_counter;
mod storage;
mod sun;
mod system_off;
mod temperature;
mod themes;
mod touch;
mod ui;
mod ui_loop;
mod unpaired;
mod version;
mod wake;

use core::sync::atomic::{AtomicBool, Ordering};

use defmt::{info, warn};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_futures::select::{Either3, select3};
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use embassy_nrf::interrupt::{self, InterruptExt, Priority};
use embassy_nrf::wdt::{self, HaltConfig, Watchdog, WatchdogHandle};
use embassy_nrf::{bind_interrupts, peripherals, rng, saadc, spim, twim};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Ticker, Timer};
use nrf_sdc::mpsl;
use static_cell::StaticCell;

// The panic handler is `field_log::panic`.
use defmt_rtt as _;

use crate::canvas::Canvas;
use crate::display::Display;
use crate::themes::Theme as _;
use crate::ui_loop::{Board, Stored, UiLoop, Wake};

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

pub use crate::ui_loop::InputEvent;

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

/// The three active-low backlight transistors (low/mid/high), which sum;
/// see [`brightness`] for which to light when.
struct Backlight {
    low: Output<'static>,
    mid: Output<'static>,
    high: Output<'static>,
}

impl Backlight {
    fn set(&mut self, level: brightness::Level) {
        let [low, mid, high] = level.drivers();
        for (pin, on) in [
            (&mut self.low, low),
            (&mut self.mid, mid),
            (&mut self.high, high),
        ] {
            pin.set_level(if on { Level::Low } else { Level::High });
        }
    }

    fn off(&mut self) {
        self.set(brightness::Level::OFF);
    }
}

/// What the UI loop drives directly: the panel, the backlight, the
/// sensors it reads itself, and the watchdog (see [`ui_loop::Board`]).
struct Hardware {
    display: Display,
    backlight: Backlight,
    /// The battery's voltage, through a divider (see `main`).
    adc: saadc::Saadc<'static, 1>,
    /// The charger's status lines, both active-low.
    charging: Input<'static>,
    power_present: Input<'static>,
    /// For the die temperature, which MPSL owns.
    mpsl: &'static mpsl::MultiprotocolServiceLayer<'static>,
    watchdog: Option<WatchdogHandle>,
}

impl Board for Hardware {
    fn die_temperature(&mut self) -> i32 {
        // From thread mode, as MPSL requires: the same priority as its
        // `mpsl_task`. This blocks for ~50 µs.
        self.mpsl.get_temperature().raw()
    }

    fn sample_battery<'a>(&'a mut self, sample: &'a mut [i16; 1]) -> impl Future<Output = ()> + 'a {
        self.adc.sample(sample)
    }

    fn charger(&mut self) -> (bool, bool) {
        debug::charger_forced().unwrap_or((self.charging.is_low(), self.power_present.is_low()))
    }

    fn daytime(&mut self) -> bool {
        geo::is_daytime()
    }

    fn finger_down(&mut self) -> bool {
        touch::finger_down()
    }

    fn display(&mut self) -> &mut Display {
        &mut self.display
    }

    fn wake_panel(&mut self) -> impl Future<Output = ()> {
        self.display.wake()
    }

    fn sleep_panel(&mut self) -> impl Future<Output = ()> {
        self.display.sleep()
    }

    fn show_panel(&mut self) -> impl Future<Output = ()> {
        self.display.show()
    }

    fn set_idle(&mut self, idle: bool) -> impl Future<Output = ()> {
        self.display.set_idle(idle)
    }

    fn wait_for_refresh(&mut self) -> impl Future<Output = ()> {
        Timer::after(display::REFRESH)
    }

    fn backlight(&mut self, level: brightness::Level) {
        self.backlight.set(level);
    }

    fn bthome(&mut self) -> Option<bthome::Identity> {
        bthome::identity()
    }

    fn advertise(
        &mut self,
        press: Option<buttons::Press>,
        layout: buttons::Layout,
        battery: u8,
    ) -> impl Future<Output = bool> {
        bthome::send(press, layout, battery)
    }

    async fn switch_off(
        &mut self,
        reason: system_off::Reason,
        canvas: &mut Canvas,
        panel_awake: bool,
    ) {
        system_off::shut_down(
            reason,
            &mut self.display,
            canvas,
            &mut self.backlight,
            panel_awake,
            &mut self.watchdog,
        )
        .await
    }

    fn pet_watchdog(&mut self) {
        // Holding the button long enough to starve the watchdog is the
        // user's way back into the bootloader: see `Board::pet_watchdog`.
        if let Some(watchdog) = &mut self.watchdog
            && !BUTTON_HELD.load(Ordering::Relaxed)
        {
            watchdog.pet();
        }
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
    let p = embassy_nrf::init(config);
    info!("ur {=str} booting", env!("UR_VERSION"));
    clock::restore();
    field_log::report_last_reset();
    field_log::guard_stack();

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
    let watchdog = match Watchdog::try_new::<_, 1>(p.WDT, wdt_config) {
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

    let backlight = Backlight {
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
    // Before the heart-rate task starts, so that a boot on the charger
    // never lights its LED (the UI loop keeps it up to date from here).
    heart_reading::set_charger(power_present.is_low());

    // Battery voltage arrives on AIN7 (P0.31) through a 1:2 divider. The
    // divider is high-impedance, so give the sample-and-hold capacitor plenty
    // of time to settle.
    let mut battery_channel = saadc::ChannelConfig::single_ended(p.P0_31);
    battery_channel.time = saadc::Time::_40US;
    let adc = saadc::Saadc::new(p.SAADC, Irqs, saadc::Config::default(), [battery_channel]);
    interrupt::SAADC.set_priority(APP_PRIORITY);
    adc.calibrate().await;

    // ==========================================================================
    // Sensor bus and touch
    // ==========================================================================

    i2c::recover_bus().await;
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
    #[cfg(feature = "wedge-i2c")]
    spawner.spawn(i2c::wedge_task().expect("spawned once"));

    let touch_interrupt = Input::new(p.P0_28, Pull::Up);
    let touch_reset = Output::new(p.P0_10, Level::High, OutputDrive::Standard);
    spawner.spawn(
        touch::touch_task(I2cDevice::new(i2c_bus), touch_interrupt, touch_reset)
            .expect("spawned once"),
    );
    spawner.spawn(activity::activity_task(I2cDevice::new(i2c_bus)).expect("spawned once"));
    // The accelerometer's INT1, driven push-pull once configured.
    let motion_interrupt = Input::new(p.P0_08, Pull::None);
    spawner.spawn(
        activity::motion_task(I2cDevice::new(i2c_bus), motion_interrupt).expect("spawned once"),
    );
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
            // Not `expect`, which would format the error with `Debug` and so
            // keep its every name in flash (~0.9 KB): the errno will do.
            .unwrap_or_else(|e| {
                panic!(
                    "MPSL initialises once, with valid clock config: {}",
                    i32::from(e)
                )
            }),
    );
    spawner.spawn(ble::mpsl_task(mpsl).expect("spawned once"));
    // Flash writes go through MPSL so they don't disturb the radio; see
    // `nvmc` and `storage`.
    storage::init(nvmc::Flash::new(mpsl, p.NVMC));
    #[cfg(feature = "flash-stress")]
    spawner.spawn(flash_stress::stress_task().expect("spawned once"));

    static RNG: StaticCell<rng::Rng<'static, embassy_nrf::mode::Async>> = StaticCell::new();
    let rng = RNG.init(rng::Rng::new(p.RNG, Irqs));
    interrupt::RNG.set_priority(APP_PRIORITY);

    // A stable address, so the phone recognises us after a reboot. FICR holds
    // a factory-random 48-bit address per chip; static random addresses need
    // the top two bits set.
    let ficr = embassy_nrf::pac::FICR;
    let (lo, hi) = (ficr.deviceaddr(0).read(), ficr.deviceaddr(1).read());
    let mut address = [0u8; 6];
    address[..4].copy_from_slice(&lo.to_le_bytes());
    address[4..].copy_from_slice(&hi.to_le_bytes()[..2]);
    address[5] |= 0xC0;
    // The button grid's key for Home Assistant, made from the RNG on first
    // boot: before the controller takes the RNG over.
    bthome::init(rng, address).await;

    // The controller's memory needs depend on its configuration; `build`
    // logs the required size if this is wrong (and fails if it's too small).
    // Scanning (the multirole library, and the scanner's own buffers) took
    // it from 5184 to 6064 bytes.
    static SDC_MEM: StaticCell<nrf_sdc::Mem<6064>> = StaticCell::new();
    let sdc_mem = SDC_MEM.init(nrf_sdc::Mem::new());
    let sdc_peripherals = nrf_sdc::Peripherals::new(
        p.PPI_CH17, p.PPI_CH18, p.PPI_CH20, p.PPI_CH21, p.PPI_CH22, p.PPI_CH23, p.PPI_CH24,
        p.PPI_CH25, p.PPI_CH26, p.PPI_CH27, p.PPI_CH28, p.PPI_CH29,
    );
    let packet_size = <packet_pool::Pool as trouble_host::PacketPool>::MTU as u16;
    let controller = nrf_sdc::Builder::new()
        .and_then(|b| {
            b.support_adv()
                .support_peripheral()
                // Passive scanning for room sensors (see `ble::scan`), which
                // takes the multirole library.
                .support_scan()
                .peripheral_count(ble::CONNECTIONS as u8)
        })
        // Scanning only: no connections of our own as a central, which
        // would reserve a link's buffers (~2.3 KB).
        .and_then(|b| b.central_count(0))
        // The fewest report buffers the scanner takes (104 bytes each,
        // default 3). Too few only loses scan responses, which a passive
        // scan never asks for.
        .and_then(|b| b.scan_buffer_cfg(2))
        // Data length extension, so 251-byte packets go out as one radio
        // packet instead of ten 27-byte ones.
        .map(|b| b.support_dle_peripheral())
        .and_then(|b| b.buffer_cfg(packet_size, packet_size, 3, 3))
        .and_then(|b| b.build(sdc_peripherals, rng, mpsl, sdc_mem))
        // The errno, not `expect`'s `Debug` of the error; see MPSL's above.
        .unwrap_or_else(|e| {
            panic!(
                "controller configuration is valid and its memory is large enough: {}",
                i32::from(e)
            )
        });
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
    let display = Display::new(lcd_cs, lcd_dc, lcd_reset).await;
    info!("display initialised");
    *spi_flash::FLASH.lock().await = Some(spi_flash::SpiFlash::new(flash_cs).await);
    #[cfg(feature = "field-log")]
    spawner.spawn(field_log::task().expect("spawned once"));

    // ==========================================================================
    // UI loop
    // ==========================================================================
    //
    // Wakes once a second and on every input. Each pass: update state, then
    // (if the screen is on) render a full frame and push what changed. What
    // a pass does is `ui_loop`'s, free of hardware, so that the simulator
    // runs it too; here is only when it runs, and with what hardware.

    // Static rather than on the stack: two 3.2 KB frame buffers.
    static CANVAS: StaticCell<Canvas> = StaticCell::new();
    let canvas = CANVAS.init(Canvas::new(themes::get(false, 0).palette()));
    debug::register_canvas(canvas);
    #[cfg(feature = "debug-input")]
    spawner.spawn(debug::input_task().expect("spawned once"));
    let (mut app, mut ui) = UiLoop::boot(Stored::load().await);
    let mut board = Hardware {
        display,
        backlight,
        adc,
        charging,
        power_present,
        mpsl,
        watchdog,
    };
    let mut ticker = Ticker::every(ui_loop::TICK);
    loop {
        ui.pass(&mut app, &mut board, canvas).await;
        // Wake for the next second, the next input, or the loop's timer
        // (see `ui_loop::timer_at`).
        let timer_at = ui_loop::timer_at(&app, touch::finger_down());
        let timer = async {
            match timer_at {
                Some(at) => Timer::at(at).await,
                None => core::future::pending::<()>().await,
            }
        };
        let wake = match select3(ticker.next(), INPUT.receive(), timer).await {
            Either3::First(()) => Wake::Tick,
            Either3::Second(event) => Wake::Input(event),
            Either3::Third(()) => Wake::Timer,
        };
        ui.handle(&mut app, &mut board, wake).await;
    }
}

// Last, so that nothing above moves (panic messages carry line numbers, and
// a release image must stay byte for byte the same without the feature).
#[cfg(feature = "coverage")]
mod coverage;
