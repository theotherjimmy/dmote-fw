#![no_main]
#![no_std]

use panic_halt as _;
use embedded_hal::digital::v2::OutputPin;
use stm32f1xx_hal::prelude::*;
use stm32f1xx_hal::usb::{Peripheral, UsbBus, UsbBusType};
use stm32f1xx_hal::pac::Peripherals;
use usb_device::bus::UsbBusAllocator;
use usb_device::prelude::*;
use cortex_m_rt::entry;
use core::default::Default;

mod hid;
mod key_code;
mod keyboard;
mod scan;
mod trigger;

use key_code::{KeyCode::*, Layout};
use scan::{dma_key_scan, scan, report, Cols, Log, Matrix, Rows};
use trigger::QuickDraw;

/// A handly shortcut for the USB class type.
pub type UsbClass = hid::HidClass<'static, UsbBusType, keyboard::Keyboard>;

const VID: u16 = 0x1209;

const PID: u16 = 0x345c;

/// Constructor for `Class`.
pub fn new_class(bus: &'static UsbBusAllocator<UsbBusType>) -> UsbClass {
    hid::HidClass::new(keyboard::Keyboard::default(), bus)
}

/// Constructor for a USB keyboard device.
pub fn new_device(
    bus: &UsbBusAllocator<UsbBusType>,
) -> usb_device::device::UsbDevice<'_, UsbBusType> {
    UsbDeviceBuilder::new(bus, UsbVidPid(VID, PID))
        .manufacturer("Me")
        .product("Dactyl Manuform: OTE")
        .serial_number(env!("CARGO_PKG_VERSION"))
        .device_class(hid::INTERFACE_CLASS_HID)
        .build()
}

/// Mapping from switch positions to keys symbols; 'a', '1', '$', etc.
#[rustfmt::skip]
#[cfg(feature = "dmote")]
 pub static LAYOUT: Layout<13, 6> = [
     /*                 Port A                          */
     /* 0     1       2            3          4       5 */
     /* -------------- Left Fingers -------------------      Port B */
     [__,     __,     __,          __,        __,     __    ], /* 3 */
     [__,     __,     W,           E,         R,      T     ], /* 4 */
     [__,     Q,      S,           D,         F,      G     ], /* 5 */
     [__,     A,      X,           C,         V,      B     ], /* 6 */
     [__,     Z,      NonUsBslash, Left,      Right,  __    ], /* 7 */
     /* ------------------- Thumbs -------------------- */
     /*  Thumb Cluster  Last Middle key  Thumb Cluster
      *      +---+       +---+   +---+       +---+
      *  +---+9,0+---+   |8,2|   |8,3|   +---+9,5+---+
      *  |a,0+---+8,0|   +---+   +---+   |a,5+---+8,5|
      *  +---+9,1+---+                   +---+9,4+---+
      *  |a,1+---+8,1|     Face keys     |a,4+---+a,4|
      *  +---+9,2+---+   +---+   +---+   +---+9,3+---+
      *      +---+       |a,2|   |a,3|       +---+
      *                  +---+   +---+
      */
     /* --- Right  ------------|---------- Left ------- */
     [__,     BSpace, RBracket,    Grave,     LShift, LCtrl ], /* 8 */
     [RAlt,   Enter,  Tab,         Escape,    Space,  LAlt  ], /* 9 */
     [PgUp,   PgDown, F12,         Pause,     End,    Home  ], /* 10(a) */
     /* ------------- Right Fingers ----------------- */
     [__,     __,     __,          __,        __,     __    ], /* 11 */
     [Y,      U,      I,           O,         __,     __    ], /* 12 */
     [H,      J,      K,           L,         P,      Bslash], /* 13 */
     [N,      M,      Comma,       Dot,       SColon, Quote ], /* 14 */
     [__,     Up,     Down,        LBracket,  Slash,  RShift], /* 15 */
];
#[rustfmt::skip]
#[cfg(feature = "dmote")]
 pub static LAYOUT_ALT: Layout<13, 6> = [
     /*                 Port A                          */
     /* 0     1       2            3          4       5 */
     /* -------------- Left Fingers -------------------      Port B */
     [__,     __,     __,          __,        __,     __    ], /* 3 */
     [__,     __,     F3,          F4,        F5,     F6    ], /* 4 */
     [F1,     F2,     Kb2,         Kb3,       Kb4,    Kb5   ], /* 5 */
     [Equal,  Kb1,    X,           C,         V,      B     ], /* 6 */
     [__,     Z,      NonUsBslash, Left,      Right,  __    ], /* 7 */
     /* ------------------- Thumbs -------------------- */
     /*  Thumb Cluster  Last Middle key  Thumb Cluster
      *      +---+       +---+   +---+       +---+
      *  +---+9,0+---+   |8,2|   |8,3|   +---+9,5+---+
      *  |a,0+---+8,0|   +---+   +---+   |a,5+---+8,5|
      *  +---+9,1+---+                   +---+9,4+---+
      *  |a,1+---+8,1|     Face keys     |a,4+---+a,4|
      *  +---+9,2+---+   +---+   +---+   +---+9,3+---+
      *      +---+       |a,2|   |a,3|       +---+
      *                  +---+   +---+
      */
     /* --- Right  ------------|---------- Left ------- */
     [__,     BSpace, RBracket,    Grave,     LShift, LCtrl ], /* 8 */
     [RAlt,   Enter,  Tab,         Escape,    Space,  LAlt  ], /* 9 */
     [PgUp,   PgDown, F12,         Pause,     End,    Home  ], /* 10(a) */
     /* ------------- Right Fingers ----------------- */
     [__,     __,     __,          __,        __,     __    ], /* 11 */
     [F7,     F8,     F9,          F10,       __,    __     ], /* 12 */
     [Kb6,    Kb7,    Kb8,         Kb9,       F11,    F12   ], /* 13 */
     [N,      M,      Comma,       Dot,       Kb0,    Minus ], /* 14 */
     [__,     Up,     Down,        LBracket,  Slash,  RShift], /* 15 */
];
#[rustfmt::skip]
#[cfg(feature = "dactyl")]
pub static LAYOUT: Layout<13, 6> = [
    /*                 Port A                            */
    /* 0     1       2            3          4         5 */
    /* -------------- Left Fingers ------------------------- Port B */
    [Equal,  Kb1,    Kb2,         Kb3,      Kb4,      Kb5   ], /* 3 */
    [Tab,    Q,      W,           E,        R,        T     ], /* 4 */
    [Escape, A,      S,           D,        F,        G     ], /* 5 */
    [LShift, Z,      X,           C,        V,        B     ], /* 6 */
    [Delete, Grave,  NonUsBslash, Left,     Right,    __    ], /* 7 */
    /*
     *                        Left thumb pad
     *                            +---+---+
     *                            | 5 | 4 |
     *                        +---+---+---+
     *                        |   |   | 3 |
     *                        | 1 | 0 +---+
     *                        |   |   | 2 |
     *                        +---+---+---+
     */
    [LShift, BSpace, End,         Home,     LAlt,     LCtrl ], /* 8 */
    /* PB9 is not wired to anything on the Dactyl */
    [__,     __,     __,          __,       __,       __    ], /* 9 */
    /* ------------- Right Fingers --------------------------         */
    [Kb6,    Kb7,    Kb8,         Kb9,      Kb0,      Minus ], /* 10 */
    [Y,      U,      I,           O,        P,        Bslash], /* 11 */
    [H,      J,      K,           L,        SColon,   Quote ], /* 12 */
    [N,      M,      Comma,       Dot,      Slash,    RShift], /* 13 */
    [__,     Up,     Down,        LBracket, RBracket, F12   ], /* 14 */
    /*
     *                        Right thumb pad
     *                        +-------+
     *                        | 1 | 0 |
     *                        +---+---+---+
     *                        | 2 |   |   |
     *                        +---+ 4 | 5 |
     *                        | 3 |   |   |
     *                        +---+---+---+
     */
    [RCtrl,  RGui,   PgUp,        PgDown,   Enter,    Space], /* 15 */
];

static mut USB_BUS: Option<UsbBusAllocator<UsbBusType>> = None;
static mut USB_CLASS: Option<UsbClass> = None;

#[entry]
fn main() -> ! {
    let device = unsafe { Peripherals::steal() };

    let mut flash = device.FLASH.constrain();
    let mut rcc = device.RCC.constrain();
    let mut debouncer: [[QuickDraw<100>; 13]; 6] = [[Default::default(); 13]; 6];
    let scan_freq = 2.khz();

    let clocks = rcc
        .cfgr
        .use_hse(8_u32.mhz())
        .sysclk(72_u32.mhz())
        .pclk1(36_u32.mhz())
        .freeze(&mut flash.acr);

    let mut gpioa = device.GPIOA.split(&mut rcc.apb2);
    let mut gpiob = device.GPIOB.split(&mut rcc.apb2);
    let mut afio = device.AFIO.constrain(&mut rcc.apb2);
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
        usb: device.USB,
        pin_dm: gpioa.pa11,
        pin_dp: usb_dp.into_floating_input(&mut gpioa.crh),
    };

    let usb_bus = unsafe {
        USB_BUS = Some(UsbBus::new(usb));
        // If we can't do this, we can't be a keyboard, so we _should_ panic if this
        // fails
        match USB_BUS.as_ref() {
            Some(ub) => ub,
            None => panic!(),
        }
    };

    let usb_class = unsafe {
        USB_CLASS = Some(new_class(usb_bus));
        match USB_CLASS.as_mut() {
            Some(uc) => uc,
            None => panic!(),
        }
    };

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
        device.DMA1,
        device.TIM1,
        &mut rcc.ahb,
        &mut rcc.apb2,
        &clocks,
    );
    let mut usb_dev = new_device(usb_bus);
    let _ = usb_dev.force_reset();

    let log = Log::get();
    let mut now: u32 = 0;
    loop {
        usb_dev.poll(&mut [usb_class]);
        let dma_isr = dma.5.isr();
        if dma_isr.bits() != 0 {
            let half: usize = if dma_isr.htif4().bits() { 0 } else { 1 };
            dma.5.ifcr().write(|w| w.cgif5().clear());
            now = now.wrapping_add(1);
            let token = scan(&scanout[half], &mut debouncer, log, now);
            #[cfg(feature = "dmote")]
            let layout = if debouncer[0][5].is_pressed() {
                &LAYOUT_ALT
            } else {
                &LAYOUT
            };
            #[cfg(feature = "dactyl")]
            let layout = &LAYOUT;
            let rep = report(layout, &debouncer, token);
            let _ = usb_class.write(rep.as_bytes());
        }
    }
}
