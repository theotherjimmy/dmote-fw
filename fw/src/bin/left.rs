#![no_main]
#![no_std]
use keyberon::layout::LogicalState;
use nb::block;
use packed_struct::prelude::*;
use panic_halt as _;
use rtic::app;
use stm32f1xx_hal::prelude::*;
use stm32f1xx_hal::serial::Tx;
use stm32f1xx_hal::{dma, pac};

use dmote_fw::{
    dma_key_scan, keys_from_scan, Cols, KeyEvent, Log, Matrix, QuickDraw, Rows, PHONE_LINE_BAUD,
};

/// Resources to build a keyboard
pub struct Keyboard {
    pub tx: Tx<pac::USART3>,
    pub debouncer: [[QuickDraw; 8]; 6],
    pub now: u32,
    pub timeout: u32,
    pub log: &'static mut Log,
}

#[app(device = stm32f1xx_hal::pac, peripherals = true)]
mod app {
    use super::*;
    use stm32f1xx_hal::serial::Config;
    use stm32f1xx_hal::serial::Serial;

    #[resources]
    struct Resources {
        keyboard: Keyboard,
        dma: dma::dma1::Channels,
        scanout: &'static [[u8; 6]; 2],
    }

    #[init]
    fn init(c: init::Context) -> (init::LateResources, init::Monotonics) {
        let mut flash = c.device.FLASH.constrain();
        let mut rcc = c.device.RCC.constrain();
        let debouncer = QuickDraw::build_array();

        let clocks = rcc
            .cfgr
            .use_hse(8_u32.mhz())
            .sysclk(72_u32.mhz())
            .pclk1(36_u32.mhz())
            .freeze(&mut flash.acr);

        let mut gpioa = c.device.GPIOA.split(&mut rcc.apb2);
        let mut gpiob = c.device.GPIOB.split(&mut rcc.apb2);
        let mut afio = c.device.AFIO.constrain(&mut rcc.apb2);
        let (_, pb3, pb4) = afio.mapr.disable_jtag(gpioa.pa15, gpiob.pb3, gpiob.pb4);

        let pin_tx = gpiob.pb10.into_alternate_push_pull(&mut gpiob.crh);
        let pin_rx = gpiob.pb11;

        let serial = Serial::usart3(
            c.device.USART3,
            (pin_tx, pin_rx),
            &mut afio.mapr,
            Config::default().baudrate(PHONE_LINE_BAUD.bps()),
            clocks,
            &mut rcc.apb1,
        );

        let (tx, _) = serial.split();

        #[rustfmt::skip]
        let cols = Cols(
                  pb3.into_push_pull_output(&mut gpiob.crl),
                  pb4.into_push_pull_output(&mut gpiob.crl),
            gpiob.pb5.into_push_pull_output(&mut gpiob.crl),
            gpiob.pb6.into_push_pull_output(&mut gpiob.crl),
            gpiob.pb7.into_push_pull_output(&mut gpiob.crl),
            gpiob.pb8.into_push_pull_output(&mut gpiob.crh),
        );
        let rows = Rows(
            gpioa.pa0.into_pull_down_input(&mut gpioa.crl),
            gpioa.pa1.into_pull_down_input(&mut gpioa.crl),
            gpioa.pa2.into_pull_down_input(&mut gpioa.crl),
            gpioa.pa3.into_pull_down_input(&mut gpioa.crl),
            gpioa.pa4.into_pull_down_input(&mut gpioa.crl),
            gpioa.pa5.into_pull_down_input(&mut gpioa.crl),
            gpioa.pa6.into_pull_down_input(&mut gpioa.crl),
            gpioa.pa7.into_pull_down_input(&mut gpioa.crl),
        );

        let (dma, scanout) = dma_key_scan(
            5.khz(),
            Matrix { rows, cols },
            c.device.DMA1,
            c.device.TIM1,
            &mut rcc.ahb,
            &mut rcc.apb2,
            &clocks,
        );

        (
            init::LateResources {
                dma,
                scanout,
                keyboard: Keyboard {
                    debouncer,
                    tx,
                    now: 0,
                    timeout: 75,
                    log: Log::get(),
                },
            },
            init::Monotonics(),
        )
    }

    #[task(binds = DMA1_CHANNEL5, priority = 1, resources = [&dma, &scanout, keyboard])]
    fn tick(mut c: tick::Context) {
        let tick::Resources {
            ref mut keyboard,
            dma,
            scanout,
        } = c.resources;
        let half = if dma.5.isr().htif4().bits() { 0 } else { 1 };
        // Clear all pending interrupts, irrespective of type
        dma.5.ifcr().write(|w| w.cgif4().clear());
        keyboard.lock(
            |Keyboard {
                 tx,
                 debouncer,
                 now,
                 timeout,
                 log,
             }| {
                *now += 1;
                for event in keys_from_scan(&scanout[half], debouncer, log, *now, *timeout) {
                    let kevent = KeyEvent{
                        brk: event.state == LogicalState::Press,
                        row: event.coord.0.into(),
                        col: event.coord.1.into(),
                    };
                    let packed: [u8; 1] = match kevent.pack() {
                        Ok(p) => p,
                        Err(_e) => panic!(),
                    };
                    //NOTE: Despite the call to block here, this is real time. when
                    // fewer than 3 keys are pressed within 200us, on this half of
                    // the keyboard, we can transmit them all before the next
                    // interrupt, but just barely. transmitting 2 press/release
                    // events takes about 174us at the selected baud rate, 115_200
                    // bps.
                    //
                    // Luckliy, we interleave packing and sending, so really we
                    // have to acomplish debouncing in 27us to meet this deadline.
                    // This allows us 1900 cycles worth of time to leave the prior
                    // interrupt, enter this interrupt, debounce and start
                    // transmitting. That's a pretty tight deadline.
                    //
                    // The prior `unwrap` probably only failed when you hit two keys
                    // in the same 200us window, which was  pretty unlikely, but not
                    // impossible to do during normal typing. I'm okay with a slight
                    // delay if you manage to do that.   Especially considering that
                    // the debouncer adds another 5ms of latency.
                    match block!(tx.write(packed[0])) {
                        Ok(_) => (),
                        // NOTE: This is of  the type `Infallible`, so it's
                        // actually impossible to hit
                        Err(_) => unreachable!(),
                    }
                }
            },
        );
    }
}
