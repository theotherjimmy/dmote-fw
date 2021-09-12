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
use scan::{dma_key_scan, scan, Cols, Log, Matrix, Rows};
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
pub static LAYOUT: Layout<13, 6> = [
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
    usb_dev.force_reset();

    let log = Log::get();
    let mut now: u32 = 0;
    loop {
        usb_dev.poll(&mut [usb_class]);
        let dma_isr = dma.5.isr();
        if dma_isr.bits() != 0 {
            let half: usize = if dma_isr.htif4().bits() { 0 } else { 1 };
            dma.5.ifcr().write(|w| w.cgif5().clear());
            now = now.wrapping_add(1);
            let report = scan(&LAYOUT, &scanout[half], &mut debouncer, log, now);
            usb_class.write(report.as_bytes());
        }
    }
}
