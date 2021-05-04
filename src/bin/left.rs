#![no_main]
#![no_std]
use generic_array::typenum::{U6, U8};
use keyberon::debounce::Debouncer;
use keyberon::layout::Event;
use keyberon::matrix::PressedKeys;
use packed_struct::prelude::*;
use panic_halt as _;
use rtic::app;
use stm32f1xx_hal::prelude::*;
use stm32f1xx_hal::serial::Tx;
use stm32f1xx_hal::{dma, pac};

use dmote_fw::{dma_key_scan, keys_from_scan, Cols, KeyEvent, Matrix, Rows};

/// Resources to build a keyboard
pub struct Keyboard {
    pub tx: Tx<pac::USART3>,
    pub debouncer: Debouncer<PressedKeys<U8, U6>>,
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
        let debouncer = Debouncer::new(PressedKeys::default(), PressedKeys::default(), 25);

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
            Config::default().baudrate(115_200.bps()),
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
                keyboard: Keyboard { debouncer, tx },
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
        let half = dma.5.isr().htif4().bits();
        // Clear all pending interrupts, irrespective of type
        dma.5.ifcr().write(|w| w.cgif4().clear());
        let events = keys_from_scan(&scanout[if half { 0 } else { 1 }]);

        keyboard.lock(|Keyboard { tx, debouncer }| {
            for event in debouncer.events(events) {
                let kevent = match event {
                    Event::Press(row, col) => KeyEvent {
                        brk: false,
                        row: row.into(),
                        col: col.into(),
                    },
                    Event::Release(row, col) => KeyEvent {
                        brk: true,
                        row: row.into(),
                        col: col.into(),
                    },
                };
                let packed: [u8; 1] = match kevent.pack() {
                    Ok(p) => p,
                    Err(_e) => panic!(),
                };
                tx.write(packed[0]).unwrap();
            }
        });
    }
}
