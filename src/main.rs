//! `ur`: firmware for the PineTime.
//!
//! This file is board bring-up and the main UI loop. The pieces:
//!
//! - [`canvas`] / [`font`] / [`theme`]: the 80×80 pixel-art canvas, its
//!   glyphs, and colour themes.
//! - [`display`]: the async LCD driver the canvas pushes pixels through.
//! - [`ui`]: screens, navigation, and screen power.
//! - [`touch`], the button task here: input, delivered through [`INPUT`].
//! - [`motor`]: vibration patterns.
//! - [`battery`]: battery voltage and charge state.
//! - [`i2c`]: the shared sensor bus.
//!
//! Tasks talk to the UI loop only through [`INPUT`]; the UI loop owns the
//! display and everything shown on it.

#![no_std]
#![no_main]

mod battery;
mod canvas;
mod display;
mod font;
mod i2c;
mod motor;
mod theme;
mod touch;
mod ui;

use defmt::{info, warn};
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use embassy_nrf::wdt::{self, HaltConfig, Watchdog};
use embassy_nrf::{bind_interrupts, peripherals, saadc, spim, twim};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Instant, Ticker, Timer};
use static_cell::StaticCell;

use {defmt_rtt as _, panic_probe as _};

use crate::battery::Battery;
use crate::canvas::{Canvas, Sweep};
use crate::display::Display;
use crate::touch::Touch;
use crate::ui::{App, Status};

bind_interrupts!(struct Irqs {
    SPI2 => spim::InterruptHandler<peripherals::SPI2>;
    TWISPI1 => twim::InterruptHandler<peripherals::TWISPI1>;
    SAADC => saadc::InterruptHandler;
});

// ==============================================================================
// Input
// ==============================================================================

#[derive(Clone, Copy, defmt::Format)]
pub enum InputEvent {
    Button,
    Touch(Touch),
}

/// Every user input, in order, for the UI loop.
pub static INPUT: Channel<CriticalSectionRawMutex, InputEvent, 8> = Channel::new();

/// Watches the side button and reports each (debounced) press.
#[embassy_executor::task]
async fn button_task(mut button: Input<'static>) {
    loop {
        button.wait_for_rising_edge().await;
        INPUT.send(InputEvent::Button).await;
        // The button is mechanical, so ignore bounces on both the press and
        // the release before arming for the next press.
        Timer::after_millis(20).await;
        button.wait_for_low().await;
        Timer::after_millis(20).await;
    }
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

    fn off(&mut self) {
        self.low.set_high();
        self.mid.set_high();
        self.high.set_high();
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // TODO: enable the DC/DC regulator (the PineTime has the inductor for it)
    // to roughly halve current draw; the LDO default is just the safe choice.
    let mut p = embassy_nrf::init(Default::default());
    info!("ur booting");

    // ==========================================================================
    // Watchdog
    // ==========================================================================

    // The WDT survives soft resets, and the dev kit's stock InfiniTime firmware
    // starts it. If it's already running (e.g. we were just flashed over
    // InfiniTime), we must adopt its exact configuration or keep getting
    // reset. Otherwise we start our own so a hung UI loop recovers.
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

    // The SPI-NOR flash shares the SPI bus with the LCD. It never gets used
    // here, but its chip-select must be held high so it doesn't drive MISO or
    // interpret LCD traffic as commands.
    // TODO: put it in deep power-down; it idles at a few µA otherwise.
    let _flash_cs = Output::new(p.P0_05, Level::High, OutputDrive::Standard);

    let mut backlight = Backlight {
        low: Output::new(p.P0_14, Level::High, OutputDrive::Standard),
        mid: Output::new(p.P0_22, Level::High, OutputDrive::Standard),
        high: Output::new(p.P0_23, Level::High, OutputDrive::Standard),
    };

    // The button is only powered while P0.15 is driven high, presumably so
    // firmware can cut its (small) bias current. It reads high when pressed.
    let _button_enable = Output::new(p.P0_15, Level::High, OutputDrive::Standard);
    spawner.spawn(button_task(Input::new(p.P0_13, Pull::None)).expect("spawned once"));

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
    adc.calibrate().await;

    // ==========================================================================
    // Sensor bus and touch
    // ==========================================================================

    i2c::recover_bus(p.P0_06.reborrow().into(), p.P0_07.reborrow().into()).await;
    static TWIM_TX_BUFFER: StaticCell<[u8; 16]> = StaticCell::new();
    static I2C_BUS: StaticCell<Mutex<CriticalSectionRawMutex, twim::Twim<'static>>> =
        StaticCell::new();
    let mut twim_config = twim::Config::default();
    twim_config.frequency = twim::Frequency::K400;
    let twim = twim::Twim::new(
        p.TWISPI1,
        Irqs,
        p.P0_06,
        p.P0_07,
        twim_config,
        TWIM_TX_BUFFER.init([0; 16]),
    );
    let i2c_bus = I2C_BUS.init(Mutex::new(twim));

    let touch_interrupt = Input::new(p.P0_28, Pull::Up);
    let touch_reset = Output::new(p.P0_10, Level::High, OutputDrive::Standard);
    spawner.spawn(
        touch::touch_task(I2cDevice::new(i2c_bus), touch_interrupt, touch_reset)
            .expect("spawned once"),
    );

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
    // MISO is wired (for the flash), but the LCD is write-only, so leave it
    // unconnected rather than claiming a pin we never read.
    let spi = spim::Spim::new_txonly(p.SPI2, Irqs, p.P0_02, p.P0_03, spi_config);
    let lcd_cs = Output::new(p.P0_25, Level::High, OutputDrive::Standard);
    let lcd_dc = Output::new(p.P0_18, Level::Low, OutputDrive::Standard);
    let lcd_reset = Output::new(p.P0_26, Level::High, OutputDrive::Standard);
    let mut display = Display::new(spi, lcd_cs, lcd_dc, lcd_reset).await;
    info!("display initialised");

    // ==========================================================================
    // UI loop
    // ==========================================================================
    //
    // Wakes once a second and on every input. Each pass: update state, then
    // (if the screen is on) render a full frame and push what changed.

    // Static rather than on the stack: two 3.2 KB frame buffers.
    static CANVAS: StaticCell<Canvas> = StaticCell::new();
    let canvas = CANVAS.init(Canvas::new());
    let mut battery = Battery::new();
    let mut app = App::new(Instant::now());
    // Start "asleep" so the first pass goes through the wake path: render the
    // first frame, then turn the backlight on. (Waking an already-awake
    // ST7789 is harmless.)
    let mut panel_awake = false;
    // The panel's memory holds noise after power-up; sweeping the first frame
    // in covers every block of it, and doubles as a retro boot effect.
    let mut first_frame = true;

    let mut ticker = Ticker::every(Duration::from_secs(1));
    loop {
        let now = Instant::now();
        let mut samples = [0i16; 1];
        adc.sample(&mut samples).await;
        battery.update(samples[0], charging.is_low(), power_present.is_low());

        // Apply screen power changes. Waking renders before the backlight
        // comes on, so the old frame never flashes up.
        if app.awake {
            if !panel_awake {
                display.wake().await;
            }
            let status = Status {
                uptime_secs: now.as_secs(),
                battery: &battery,
            };
            canvas.set_theme(app.theme());
            app.render(canvas, &status);
            match app.take_transition() {
                // Waking straight into a transition would animate over a
                // stale frame; just show the new one.
                Some(sweep) if panel_awake => canvas.transition(&mut display, sweep).await,
                _ if first_frame => canvas.transition(&mut display, Sweep::LeftToRight).await,
                _ => canvas.flush(&mut display).await,
            }
            first_frame = false;
            if !panel_awake {
                backlight.on();
                panel_awake = true;
            }
        } else if panel_awake {
            backlight.off();
            // In sleep the ST7789 keeps its frame memory, so the canvas's
            // idea of what's on the panel stays valid across sleep/wake.
            display.sleep().await;
            panel_awake = false;
        }
        if let Some(watchdog) = &mut watchdog {
            watchdog.pet();
        }

        match select(ticker.next(), INPUT.receive()).await {
            Either::First(()) => app.on_tick(Instant::now()),
            Either::Second(event) => {
                info!("input: {}", event);
                app.on_input(event, Instant::now());
            }
        }
    }
}
