#![no_main]
#![no_std]

use core::convert::Infallible;
use embedded_hal::digital::v2::{InputPin, OutputPin};
use generic_array::typenum::{U6, U3};
use keyberon::debounce::Debouncer;
use keyberon::impl_heterogenous_array;
use keyberon::key_code::{KbHidReport, KeyCode};
use keyberon::layout::Layout;
use keyberon::matrix::{Matrix, PressedKeys};
use panic_halt as _;
use rtic::app;
use stm32f1xx_hal::gpio::{
    gpioa::{PA0, PA1, PA2, PA3, PA4, PA5, PA8, PA9, PA10},
    gpiob::{PB3, PB4, PB5, PB6, PB7, PB8, PB12, PB13, PB14},
    Input, Output, PullUp, PushPull,
};
use stm32f1xx_hal::prelude::*;
use stm32f1xx_hal::usb::{Peripheral, UsbBus, UsbBusType};
use stm32f1xx_hal::{pac, timer};
use usb_device::bus::UsbBusAllocator;
use usb_device::class::UsbClass as _;

// NOTE: () is used in place of LEDs, as we don't care about them right now
type UsbClass = keyberon::Class<'static, UsbBusType, ()>;
type UsbDevice = usb_device::device::UsbDevice<'static, UsbBusType>;

pub struct FCols(
    pub PB3<Input<PullUp>>,
    pub PB4<Input<PullUp>>,
    pub PB5<Input<PullUp>>,
    pub PB6<Input<PullUp>>,
    pub PB7<Input<PullUp>>,
    pub PB8<Input<PullUp>>,
);

impl_heterogenous_array! {
    FCols,
    dyn InputPin<Error = Infallible>,
    U6,
    [0, 1, 2, 3, 4, 5]
}

pub struct FRows(
    pub PA0<Output<PushPull>>,
    pub PA1<Output<PushPull>>,
    pub PA2<Output<PushPull>>,
    pub PA3<Output<PushPull>>,
    pub PA4<Output<PushPull>>,
    pub PA5<Output<PushPull>>,
);
impl_heterogenous_array! {
    FRows,
    dyn OutputPin<Error = Infallible>,
    U6,
    [0, 1, 2, 3, 4, 5]
}

pub struct TCols(
    pub PA8<Input<PullUp>>,
    pub PA9<Input<PullUp>>,
    pub PA10<Input<PullUp>>,
);

impl_heterogenous_array! {
    TCols,
    dyn InputPin<Error = Infallible>,
    U3,
    [2, 1, 0]
}

pub struct TRows(
    pub PB12<Output<PushPull>>,
    pub PB13<Output<PushPull>>,
    pub PB14<Output<PushPull>>,
);
impl_heterogenous_array! {
    TRows,
    dyn OutputPin<Error = Infallible>,
    U3,
    [0, 1, 2]
}
/*
 * Ideal shape:

                2    3    4    5           6     7    8    9
    =      1    W    E    R    T           H     N    E    I    0   '-'
    Tab    Q    R    S    T    D           H     N    E    I    ';' BSLS
    Escape A    X    C    V    B           K     M    ','  '.'  O   QUOT
    LShift Z    Home Pgup End              Left  Up   Rght '\'  '`'
                     Pgdn                        Down

                          Lshift EQL         VolD VolU
                     Lgui HAEN Entr        MUTE  PNX  (3)
                          Lapo Space       Esc  RAPC
 */

#[rustfmt::skip]
pub static LEFT_FINGERS: keyberon::layout::Layers<()> = keyberon::layout::layout!{{
    [_      _     2     3      4      5]
    [=      1     W     E      R      T]
    [Tab    Q     S     D      F      G]
    [Escape A     X     C      V      B]
    [LShift Z     ~     Down   Up     _]
    [_      _     _     '`'    _      _]
}};

#[rustfmt::skip]
pub static LEFT_THUMB: keyberon::layout::Layers<()> = keyberon::layout::layout!{{
                       [_      LShift LCtrl]
                       [Escape Space  LAlt]
                       [Pause  PgDown PgUp]
}};

pub struct Matricies {
    fingers: Matrix<FCols, FRows>,
    thumb: Matrix<TCols, TRows>,
}

pub struct Layouts {
    fingers: Layout<()>,
    thumb: Layout<()>,
}

pub struct Debouncers {
    fingers: Debouncer<PressedKeys<U6, U6>>,
    thumb: Debouncer<PressedKeys<U3, U3>>,
}

#[app(device = stm32f1xx_hal::pac, peripherals = true)]
const APP: () = {
    struct Resources {
        usb_dev: UsbDevice,
        usb_class: UsbClass,
        matrix: Matricies,
        debouncer: Debouncers,
        layout: Layouts,
        timer: timer::CountDownTimer<pac::TIM3>,
    }

    #[init]
    fn init(c: init::Context) -> init::LateResources {
        static mut USB_BUS: Option<UsbBusAllocator<UsbBusType>> = None;

        let mut flash = c.device.FLASH.constrain();
        let mut rcc = c.device.RCC.constrain();

        let clocks = rcc
            .cfgr
            .use_hse(8.mhz())
            .sysclk(72.mhz())
            .pclk1(36.mhz())
            .freeze(&mut flash.acr);

        let mut gpioa = c.device.GPIOA.split(&mut rcc.apb2);
        let mut gpiob = c.device.GPIOB.split(&mut rcc.apb2);
        let mut jtag = c.device.AFIO.constrain(&mut rcc.apb2);
        let (_, pb3, pb4) = jtag.mapr.disable_jtag(gpioa.pa15, gpiob.pb3, gpiob.pb4);

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

        let usb_dm = gpioa.pa11;
        let usb_dp = usb_dp.into_floating_input(&mut gpioa.crh);

        let usb = Peripheral {
            usb: c.device.USB,
            pin_dm: usb_dm,
            pin_dp: usb_dp,
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

        let mut timer =
            timer::Timer::tim3(c.device.TIM3, &clocks, &mut rcc.apb1).start_count_down(1.khz());
        timer.listen(timer::Event::Update);

        #[rustfmt::skip]
        let fcols = FCols(
                  pb3.into_pull_up_input(&mut gpiob.crl),
                  pb4.into_pull_up_input(&mut gpiob.crl),
            gpiob.pb5.into_pull_up_input(&mut gpiob.crl),
            gpiob.pb6.into_pull_up_input(&mut gpiob.crl),
            gpiob.pb7.into_pull_up_input(&mut gpiob.crl),
            gpiob.pb8.into_pull_up_input(&mut gpiob.crh),
        );
        let frows = FRows(
            gpioa.pa0.into_push_pull_output(&mut gpioa.crl),
            gpioa.pa1.into_push_pull_output(&mut gpioa.crl),
            gpioa.pa2.into_push_pull_output(&mut gpioa.crl),
            gpioa.pa3.into_push_pull_output(&mut gpioa.crl),
            gpioa.pa4.into_push_pull_output(&mut gpioa.crl),
            gpioa.pa5.into_push_pull_output(&mut gpioa.crl),
        );
        let fingers = match Matrix::new(fcols, frows) {
            Ok(m) => m,
            Err(_) => panic!(),
        };

        let tcols = TCols(
            gpioa.pa8.into_pull_up_input(&mut gpioa.crh),
            gpioa.pa9.into_pull_up_input(&mut gpioa.crh),
            gpioa.pa10.into_pull_up_input(&mut gpioa.crh),
        );
        let trows = TRows(
            gpiob.pb12.into_push_pull_output(&mut gpiob.crh),
            gpiob.pb13.into_push_pull_output(&mut gpiob.crh),
            gpiob.pb14.into_push_pull_output(&mut gpiob.crh),
        );
        let thumb = match Matrix::new(tcols, trows) {
            Ok(m) => m,
            Err(_) => panic!(),
        };
        let matrix = Matricies {fingers, thumb};

        init::LateResources {
            usb_dev,
            usb_class,
            timer,
            debouncer: Debouncers{
                fingers: Debouncer::new(PressedKeys::default(), PressedKeys::default(), 5),
                thumb: Debouncer::new(PressedKeys::default(), PressedKeys::default(), 5),
            },
            matrix,
            layout: Layouts {
                fingers: Layout::new(LEFT_FINGERS),
                thumb: Layout::new(LEFT_THUMB),
            }
        }
    }

    #[task(binds = USB_HP_CAN_TX, priority = 2, resources = [usb_dev, usb_class])]
    fn usb_tx(mut c: usb_tx::Context) {
        usb_poll(&mut c.resources.usb_dev, &mut c.resources.usb_class);
    }

    #[task(binds = USB_LP_CAN_RX0, priority = 2, resources = [usb_dev, usb_class])]
    fn usb_rx(mut c: usb_rx::Context) {
        usb_poll(&mut c.resources.usb_dev, &mut c.resources.usb_class);
    }

    #[task(binds = TIM3, priority = 1, resources = [usb_class, matrix, debouncer, layout, timer])]
    fn tick(mut c: tick::Context) {
        c.resources.timer.clear_update_interrupt_flag();
        let events = match c.resources.matrix.fingers.get() {
            Ok(r) => r,
            Err(_) => panic!(),
        };

        for event in c.resources.debouncer.fingers.events(events) {
            c.resources.layout.fingers.event(event);
        }
        c.resources.layout.fingers.tick();

        let events = match c.resources.matrix.thumb.get() {
            Ok(r) => r,
            Err(_) => panic!(),
        };

        for event in c.resources.debouncer.thumb.events(events) {
            c.resources.layout.thumb.event(event);
        }
        c.resources.layout.thumb.tick();
        send_report(
            (c.resources.layout.thumb.keycodes()).chain(c.resources.layout.fingers.keycodes()),
            &mut c.resources.usb_class
        );
    }
};

fn send_report(iter: impl Iterator<Item = KeyCode>, usb_class: &mut resources::usb_class<'_>) {
    use rtic::Mutex;
    let report: KbHidReport = iter.collect();
    if usb_class.lock(|k| k.device_mut().set_keyboard_report(report.clone())) {
        while let Ok(0) = usb_class.lock(|k| k.write(report.as_bytes())) {}
    }
}

fn usb_poll(usb_dev: &mut UsbDevice, keyboard: &mut UsbClass) {
    if usb_dev.poll(&mut [keyboard]) {
        keyboard.poll();
    }
}
