#![no_main]
#![no_std]
use panic_halt as _;
use rtic::app;
use stm32f1xx_hal::dma;
use stm32f1xx_hal::prelude::*;
use stm32f1xx_hal::usb::{Peripheral, UsbBus, UsbBusType};
use usb_device::bus::UsbBusAllocator;
use usb_device::class::UsbClass as _;
use usb_device::prelude::*;

use dmote_fw::{
    dma_key_scan, scan, hid, keyboard, Cols, Log, Matrix, QuickDraw, Rows
};
use dmote_fw::key_code::{KbHidReport, KeyCode::*};

/// A handly shortcut for the keyberon USB class type.
pub type UsbClass = hid::HidClass<'static, UsbBusType, keyboard::Keyboard<()>>;

const VID: u16 = 0x1209;

const PID: u16 = 0x345c;

/// Constructor for `Class`.
pub fn new_class(bus: &'static UsbBusAllocator<UsbBusType>) -> UsbClass {
    hid::HidClass::new(keyboard::Keyboard::new(()), bus)
}

/// Constructor for a USB keyboard device.
pub fn new_device(
    bus: &UsbBusAllocator<UsbBusType>
) -> usb_device::device::UsbDevice<'_, UsbBusType> {
    UsbDeviceBuilder::new(bus, UsbVidPid(VID, PID))
        .manufacturer("Me")
        .product("Dactyl Manuform: OTE")
        .serial_number(env!("CARGO_PKG_VERSION"))
        .build()
}

/// Type alias for usb devices.
type UsbDevice = usb_device::device::UsbDevice<'static, UsbBusType>;


/// Mapping from switch positions to keys symbols; 'a', '1', '$', etc.
#[rustfmt::skip]
pub static LAYOUT: dmote_fw::Layout<13, 6> = [
    /*                 Port B                          */
    /* 3     4       5            6          7       8 */
    /* -------------- Left Fingers -------------------       Port A*/
    [__,     __,     Kb2,         Kb3,       Kb4,    Kb5   ], /* 0 */
    [Equal,  Kb1,    W,           E,         R,      T     ], /* 1 */
    [Tab,    Q,      S,           D,         F,      G     ], /* 2 */
    [Escape, A,      X,           C,         V,      B     ], /* 3 */
    [LShift, Z,      NonUsBslash, Left,      Right,  __    ], /* 4 */
    /* ------------------- Thumbs -------------------- */
    /* --- Right  ------------|---------- Left ------- */
    [RCtrl,  BSpace, RBracket,    Grave,     LShift, LCtrl ], /* 5 */
    [RAlt,   Enter,  Escape,      Escape,    Space,  LAlt  ], /* 6 */
    [PgUp,   PgDown, PScreen,     Pause,     End,    Home  ], /* 7 */
    /* ------------- Right Fingers ----------------- */
    [Kb6,    Kb7,    Kb8,         Kb9,       __,     __    ], /* 8 */
    [Y,      U,      I,           O,         Kb0,    Minus ], /* 9 */
    [H,      J,      K,           L,         P,      Bslash], /* 10 */
    [N,      M,      Comma,       Dot,       SColon, Quote ], /* 11 */
    [__,     Up,     Down,        LBracket,  Slash,  RShift], /* 12 */
];

/// Poll usb device. Called from within USB rx and tx interrupts
pub fn usb_poll(usb_dev: &mut UsbDevice, keyboard: &mut UsbClass) {
    if usb_dev.poll(&mut [keyboard]) {
        keyboard.poll();
    }
}
/// Resources to build a keyboard
pub struct Keyboard {
    pub debouncer: [[QuickDraw<75>; 13]; 6],
    pub now: u32,
    pub log: &'static mut Log,
}

#[app(device = stm32f1xx_hal::pac, peripherals = true)]
mod app {
    use super::*;
    use embedded_hal::digital::v2::OutputPin;

    #[resources]
    struct Resources {
        usb_dev: UsbDevice,
        usb_class: UsbClass,
        keyboard: Keyboard,
        dma: dma::dma1::Channels,
        scanout: &'static [[u16; 6]; 2],
    }

    #[init]
    fn init(c: init::Context) -> (init::LateResources, init::Monotonics) {
        static mut USB_BUS: Option<UsbBusAllocator<UsbBusType>> = None;

        let mut flash = c.device.FLASH.constrain();
        let mut rcc = c.device.RCC.constrain();
        let debouncer = [[Default::default(); 13]; 6];
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

        let usb_class = new_class(usb_bus);
        let usb_dev = new_device(usb_bus);

        // NOTE: These have to be setup, though they are dropped, as without this setup
        // code, it's not possible to read the matrix.
        let cols = Cols(
            gpioa.pa0.into_push_pull_output(&mut gpioa.crl),
            gpioa.pa1.into_push_pull_output(&mut gpioa.crl),
            gpioa.pa2.into_push_pull_output(&mut gpioa.crl),
            gpioa.pa3.into_push_pull_output(&mut gpioa.crl),
            gpioa.pa4.into_push_pull_output(&mut gpioa.crl),
            gpioa.pa5.into_push_pull_output(&mut gpioa.crl),
        );
        #[rustfmt::skip]
        let rows = Rows(
                  pb3.into_pull_down_input(&mut gpiob.crl),
                  pb4.into_pull_down_input(&mut gpiob.crl),
            gpiob.pb5.into_pull_down_input(&mut gpiob.crl),
            gpiob.pb6.into_pull_down_input(&mut gpiob.crl),
            gpiob.pb7.into_pull_down_input(&mut gpiob.crl),
            gpiob.pb8.into_pull_down_input(&mut gpiob.crh),
            gpiob.pb9.into_pull_down_input(&mut gpiob.crh),
            gpiob.pb10.into_pull_down_input(&mut gpiob.crh),
            gpiob.pb11.into_pull_down_input(&mut gpiob.crh),
            gpiob.pb12.into_pull_down_input(&mut gpiob.crh),
            gpiob.pb13.into_pull_down_input(&mut gpiob.crh),
            gpiob.pb14.into_pull_down_input(&mut gpiob.crh),
            gpiob.pb15.into_pull_down_input(&mut gpiob.crh),
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

        let log = Log::get();

        (
            init::LateResources {
                usb_dev,
                usb_class,
                dma,
                scanout,
                keyboard: Keyboard { debouncer, log, now: 0},
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
        let report: KbHidReport = keyboard.lock(|Keyboard { log, debouncer, now}| {
            *now = now.wrapping_add(1);
            scan(&LAYOUT, &scanout[half], debouncer, log, *now)
        });

        if usb_class.lock(|k| k.device_mut().set_keyboard_report(report.clone())) {
            while let Ok(0) = usb_class.lock(|k| k.write(report.as_bytes())) {}
        }
    }
}
