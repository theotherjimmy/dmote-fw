#![no_main]
#![no_std]
use keyberon::key_code::KbHidReport;
use keyberon::layout::{Event, Layout, LogicalState};
use keyberon::key_code::KeyCode::*;
use packed_struct::prelude::*;
use panic_halt as _;
use rtic::app;
use stm32f1xx_hal::dma;
use stm32f1xx_hal::prelude::*;
use stm32f1xx_hal::serial::{Rx, Error as SError};
use stm32f1xx_hal::usb::{Peripheral, UsbBus, UsbBusType};
use usb_device::bus::UsbBusAllocator;
use usb_device::class::UsbClass as _;

use dmote_fw::{
    dma_key_scan, keys_from_scan, Cols, KeyEvent, Log, Matrix, QuickDraw, Rows, PHONE_LINE_BAUD,
};

// NOTE: () is used in place of LEDs, as we don't care about them right now
/// Type alias for a keyboard with no LEDs.
type UsbClass = keyberon::Class<'static, UsbBusType, ()>;
/// Type alias for usb devices.
type UsbDevice = usb_device::device::UsbDevice<'static, UsbBusType>;


/// Mapping from switch positions to keys symbols; 'a', '1', '$', etc.
#[rustfmt::skip]
pub static LAYOUT: keyberon::layout::Layers<12, 8> = [
    [No,     No,  Kb2,         Kb3,    Kb4,    Kb5,   Kb6,   Kb7,    Kb8,      Kb9,       No,     No    ],
    [Equal,  Kb1, W,           E,      R,      T,     Y,     U,      I,        O,         Kb0,    Minus ],
    [Tab,    Q,   S,           D,      F,      G,     H,     J,      K,        L,         P,      Bslash],
    [Escape, A,   X,           C,      V,      B,     N,     M,      Comma,    Dot,       SColon, Quote ],
    [LShift, Z,   NonUsBslash, Left,   Right,  No,    No,    Up,     Down,     LBracket,  Slash,  RShift],
    [No,     No,  No,          Grave,  LShift, LCtrl, RCtrl, BSpace, RBracket, No,        No,     No    ],
    [No,     No,  No,          Escape, Space,  LAlt,  RAlt,  Enter,  Escape,   No,        No,     No    ],
    [No,     No,  No,          Pause,  End,    Home,  PgUp,  PgDown, PScreen,  No,        No,     No    ]
    // NOTE: this keyboard is in two halfs and this   ^ is the first column of the right half
];

/// Poll usb device. Called from within USB rx and tx interrupts
pub fn usb_poll(usb_dev: &mut UsbDevice, keyboard: &mut UsbClass) {
    if usb_dev.poll(&mut [keyboard]) {
        keyboard.poll();
    }
}
/// Resources to build a keyboard
pub struct Keyboard {
    pub layout: Layout<12, 8>,
    pub debouncer: [[QuickDraw; 8]; 6],
    pub now: u32,
    pub timeout: u32,
    pub log: &'static mut Log,
}

#[app(device = stm32f1xx_hal::pac, peripherals = true)]
mod app {
    use super::*;
    use embedded_hal::digital::v2::OutputPin;
    use stm32f1xx_hal::pac::USART3;
    use stm32f1xx_hal::serial::{Config, Serial};

    #[resources]
    struct Resources {
        usb_dev: UsbDevice,
        usb_class: UsbClass,
        keyboard: Keyboard,
        rx: Rx<USART3>,
        dma: dma::dma1::Channels,
        scanout: &'static [[u8; 6]; 2],
    }

    #[init]
    fn init(c: init::Context) -> (init::LateResources, init::Monotonics) {
        static mut USB_BUS: Option<UsbBusAllocator<UsbBusType>> = None;

        let mut flash = c.device.FLASH.constrain();
        let mut rcc = c.device.RCC.constrain();
        let debouncer = QuickDraw::build_array();
        let layout = Layout::new(LAYOUT);
        let scan_freq = 5.khz();

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

        // BluePill board has a pull-up resistor on the D+ line.
        // Pull the D+ pin down to send a RESET condition to the USB bus.
        let mut usb_dp = gpioa.pa12.into_push_pull_output(&mut gpioa.crh);
        // If we can't do this, we can't be a keyboard, so we _should_ panic if this
        // fails
        match usb_dp.set_low() {
            Ok(_) => (),
            Err(_) => panic!(),
        };
        cortex_m::asm::delay(clocks.sysclk().0 / 100);

        let usb = Peripheral {
            usb: c.device.USB,
            pin_dm: gpioa.pa11,
            pin_dp: usb_dp.into_floating_input(&mut gpioa.crh),
        };

        *USB_BUS = Some(UsbBus::new(usb));
        // If we can't do this, we can't be a keyboard, so we _should_ panic if this
        // fails
        let usb_bus = match USB_BUS.as_ref() {
            Some(ub) => ub,
            None => panic!(),
        };

        let usb_class = keyberon::new_class(usb_bus, ());
        let usb_dev = keyberon::new_device(usb_bus);

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

        let (_, mut rx) = serial.split();

        // NOTE: These have to be setup, though they are dropped, as without this setup
        // code, it's not possible to read the matrix.
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
            scan_freq,
            Matrix { rows, cols },
            c.device.DMA1,
            c.device.TIM1,
            &mut rcc.ahb,
            &mut rcc.apb2,
            &clocks,
        );

        rx.listen();

        let log = Log::get();

        (
            init::LateResources {
                usb_dev,
                usb_class,
                dma,
                scanout,
                rx,
                keyboard: Keyboard { debouncer, layout, log, now: 0, timeout: 75 },
            },
            init::Monotonics(),
        )
    }

    #[task(binds = USB_HP_CAN_TX, priority = 2, resources = [usb_dev, usb_class])]
    fn usb_tx(mut c: usb_tx::Context) {
        let usb_tx::Resources {
            ref mut usb_dev,
            ref mut usb_class,
        } = c.resources;
        (usb_dev, usb_class).lock(|dev, class| usb_poll(dev, class));
    }

    #[task(binds = USB_LP_CAN_RX0, priority = 2, resources = [usb_dev, usb_class])]
    fn usb_rx(mut c: usb_rx::Context) {
        let usb_rx::Resources {
            ref mut usb_dev,
            ref mut usb_class,
        } = c.resources;
        (usb_dev, usb_class).lock(|dev, class| usb_poll(dev, class));
    }

    #[task(binds = USART3, priority = 1, resources = [keyboard, rx])]
    fn uart_rx(mut c: uart_rx::Context) {
        let maybe_byte = c.resources.rx.lock(|rx| rx.read());
        match maybe_byte {
            Ok(byte) => {
                let KeyEvent { brk, row, col } = match KeyEvent::unpack(&[byte]) {
                    Ok(p) => p,
                    Err(_e) => panic!(),
                };
                let row = row.into();
                let col = col.into();
                let state = if brk { LogicalState::Press } else { LogicalState::Release };
                let event = Event { coord: (row, col), state };
                c.resources
                    .keyboard
                    .lock(|Keyboard { layout, .. }| layout.event(event));
            }
            Err(nb::Error::Other(SError::Framing)) => panic!("a"),
            Err(nb::Error::Other(SError::Noise)) => panic!("b"),
            Err(nb::Error::Other(SError::Overrun)) => panic!("c"),
            Err(nb::Error::Other(SError::Parity)) => panic!("d"),
            Err(nb::Error::Other(_)) => panic!("e"),
            // Unlike the other cases, this one simply implies that we got
            // a spurious interrupt.
            Err(nb::Error::WouldBlock) => (),
        }
    }

    #[task(binds = DMA1_CHANNEL5, priority = 1, resources = [
        usb_class, keyboard, &dma, &scanout
    ])]
    fn tick(mut c: tick::Context) {
        let tick::Resources {
            ref mut usb_class,
            ref mut keyboard,
            dma,
            scanout,
        } = c.resources;
        let half: usize = if dma.5.isr().htif4().bits() { 0 } else { 1 };
        // Clear all pending interrupts, irrespective of type
        dma.5.ifcr().write(|w| w.cgif4().clear());
        let report: KbHidReport = keyboard.lock(|Keyboard { layout, log, debouncer, now, timeout}| {
            *now = now.wrapping_add(1);
            for mut event in keys_from_scan(&scanout[half], debouncer, log, *now, *timeout) {
                event.coord.1 += 6;
                layout.event(event);
            }
            layout.keycodes().collect()
        });

        if usb_class.lock(|k| k.device_mut().set_keyboard_report(report.clone())) {
            while let Ok(0) = usb_class.lock(|k| k.write(report.as_bytes())) {}
        }
    }
}
