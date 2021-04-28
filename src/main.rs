#![no_main]
#![no_std]
use cortex_m::singleton;
use generic_array::typenum::{U6, U8};
use keyberon::debounce::Debouncer;
use keyberon::key_code::KbHidReport;
use keyberon::layout::Layout;
use keyberon::matrix::PressedKeys;
use panic_halt as _;
use rtic::app;
use stm32f1::stm32f103;
use stm32f1xx_hal::gpio::{
    gpioa::{PA0, PA1, PA2, PA3, PA4, PA5, PA6, PA7},
    gpiob::{PB3, PB4, PB5, PB6, PB7, PB8},
    Input, Output, PullDown, PushPull,
};
use stm32f1xx_hal::prelude::*;
use stm32f1xx_hal::rcc::{Clocks, Enable, GetBusFreq, Reset, AHB, APB2};
use stm32f1xx_hal::time::Hertz;
use stm32f1xx_hal::usb::{Peripheral, UsbBus, UsbBusType};
use stm32f1xx_hal::{dma, pac};
use usb_device::bus::UsbBusAllocator;
use usb_device::class::UsbClass as _;

#[inline(always)]
fn compute_arr_presc(freq: u32, clock: u32) -> (u16, u16) {
    let ticks = clock / freq;
    let psc = ((ticks - 1) / (1 << 16)) as u16;
    let arr = (ticks / (psc + 1) as u32) as u16;
    (psc, arr)
}

// NOTE: () is used in place of LEDs, as we don't care about them right now
type UsbClass = keyberon::Class<'static, UsbBusType, ()>;
type UsbDevice = usb_device::device::UsbDevice<'static, UsbBusType>;

pub fn usb_poll(usb_dev: &mut UsbDevice, keyboard: &mut UsbClass) {
    if usb_dev.poll(&mut [keyboard]) {
        keyboard.poll();
    }
}

/**
 * Setup DMA to scan an 8 row, 6 column keyboard matrix.
 *
 * # Matrix Scanning
 *
 * We setup a PWM timer and a few DMA cyclic transfers to make the DMA hardware
 * scan the keyboard matrix without the involvement of the CPU. This allows faster
 * scans or more time spent asleep, as the CPU can spend all of it's time handling
 * debouncing, matrix to keycode translation and USB traffic.
 *
 * The matrix will be represented in a semi-packed way, in that the scans will
 * produce a u8 per row with a bit for each matrix intersection.
 *
 * Starting from the timer initilalization, the DMA implement the following
 * timing:
 * ```text
 * TIM1 |    0    |    1    |    2    |    3    |    4    |    0    |
 * OUT  |000000000|000000000|000001000|000001000|000001000|000001000|-\
 * IN   |000000000|000000000|Settling |Settling |Settling |0rrrrrrrr| |
 *    ________________________________________________________________/
 *   /
 *   |  |    1    |    2    |    3    |    4    |    0    |    1    |
 *   \->|000001000|000010000|000010000|000010000|000010000|000010000| -> etc.
 *      |0rrrrrrrr|Settling |Settling |Settling |0rrrrrrrr|0rrrrrrrr|
 * ```
 *
 * # Buffering
 *
 * Since it's a bad idea to attempt to read a scan out when it's being written, the
 * DMA is given 2 buffers for scanning out. This allows user code to treat the scan
 * as if it were a double-buffered peripheral.
 *
 * # Interrupts
 *
 * This enables both the half-complete and complete DMA interrutps for DMA1 channel 5.
 * These interrupts both trigger the same handler, as the interrupt trigger is a
 * logical or of all interrupt signals for a single channel. Users of these interrupts
 * should be able to use the half-complete interrupt status bit to determine which
 * buffer is safe to read. In particular, when the half-complete interrupt status bit
 * is set, use buffer 0, and when it's clear, indicating that the interrupt was
 * generated with the DMA transfer complete interrupt, buffer 1 should be used.
 *
 * # Panics
 *
 * This function is intended as initialization, and so will panic if called more than
 * once. However, as this takes ownership of the DMA1 and TIM1 structs without returning
 * them, it should not be possible to call this more than once.
 */
// TODO: better return type? Perhaps it would be better to accept DMA1CH2 and DMA1CH5
// and return DMA1CH5's interrupt status register?
pub fn dma_key_scan(
    freq: impl Into<Hertz>,
    dma: pac::DMA1,
    tim1: pac::TIM1,
    ahb: &mut AHB,
    apb2: &mut APB2,
    clocks: &Clocks,
) -> (dma::dma1::Channels, &'static [[u8; 6]; 2]) {
    let mut dma = dma.split(ahb);
    let scanout = singleton!(: [[u8; 6]; 2] = [[0; 6]; 2]).unwrap();

    // Implementation Notes:
    //
    // To acomplish the timing diagram in the doc comment, we have to setup Timer 1
    // to have a period that matches 6 * the input frequency, and we have to setup output
    // compare for the 2/5 point of that period.
    //
    // DMA CH2 is connected to the output compare, so it must do the column strobe signal.
    //
    // DMA CH5 is connected to the the update/reset of the timer, so it must be the row
    // read.
    //
    // Registers initialisms are defined in line

    // # DMA1 CH2: Requested by Output Compare 1 (ch1) with Timer 1
    dma.2.set_peripheral_address(
        // Safety: we don't enable pointer incrimenting of Perihperal addresses
        // Further, this pointer dereference is always safe.
        unsafe { (*stm32f103::GPIOB::ptr()).odr.as_ptr() } as u32,
        false,
    );
    // Safety: we have the lenth correct below. This should probably be unsafe, because
    // we're asking the DMA hardware to derefrence a raw pointer. But hey, it's not.
    dma.2.set_memory_address(SCANIN.as_ptr() as u32, true);
    dma.2
        .set_transfer_length(core::mem::size_of_val(&SCANIN) / core::mem::size_of_val(&SCANIN[0]));
    #[rustfmt::skip]
    dma.2.ch().cr.modify(|_read, write| {
        write
            // EN: Enable
            // NOTE: we're enabling DMA here, but no triggeres have been enabled yet
            .en().enabled()
            // CIRC: CIRCular mode
            // Uppon end of transfer, start another one
            .circ().enabled()
            // DIR: DIRection
            .dir().from_memory()
            // MINC: Memory address INCriment
            .minc().enabled()
            // PSIZE: Peripheral SIZE
            // NOTE: The perihperal is always 32 bits
            .psize().bits32()
            // MSIZE: Memory SIZE
            // Since we're using bit 8 of port B, we have to store u16s
            .msize().bits16()
    });

    // # DMA1 CH5: Requested by Update/Overflow of Timer 1
    dma.5.set_peripheral_address(
        // Safety: we don't enable pointer incrimenting of Perihperal addresses
        // Further, this pointer dereference is always safe.
        unsafe { (*stm32f103::GPIOA::ptr()).idr.as_ptr() } as *const u16 as u32,
        false,
    );
    // Safety: we set the transfer length correctly, and we only read the half of the
    // buffer that's not in use by DMA.
    dma.5
        .set_memory_address(scanout.as_mut_ptr() as *mut u8 as u32, true);
    // NOTE: This is the number of elements.
    dma.5.set_transfer_length(
        core::mem::size_of_val(scanout)
            / core::mem::size_of_val(&scanout[0][0]),
    );
    #[rustfmt::skip]
    dma.5.ch().cr.modify(|_read, write| {
        write
            .en().enabled()
            .circ().enabled()
            .dir().from_peripheral()
            .minc().enabled()
            .psize().bits32()
            .msize().bits8()
            // HTIE: Half Transfer Interrupt Enable
            // We want to enable both the half and full transfer interrupts
            // as we have a double-buffering setup going.
            // Once half the transfer is complete, we have scanned the matrix once.
            .htie().enabled()
            // TCIE: Transfer Complete Interrupt Enable
            // When the full transfer is complete, we have scanned the matrix a second time.
            .tcie().enabled()
    });

    let clk = APB2::get_timer_frequency(&clocks);
    pac::TIM1::enable(apb2);
    pac::TIM1::reset(apb2);
    let timeout = (freq.into() * 6).0;
    let (psc, arr) = compute_arr_presc(timeout, clk.0);
    // CCR1: Counter Compare Register 1 (channel 1, I think).
    // CCR: Courter Compare Register (it's the value to compare with).
    tim1.ccr1.modify(|_, w| w.ccr().bits(arr * 2 / 5));
    // Impl NOTE: We enable the follwing
    // UDE: Update DMA Event
    // CC1DE: Counter Compare 1 DMA Event
    tim1.dier.modify(|_, w| w.ude().enabled().cc1de().enabled());
    // CC1E: Counter Compare 1 Enable (should probably be .enabled, but for some reason
    // the hal only exports .set_bit)
    tim1.ccer.modify(|_, w| w.cc1e().set_bit());

    // pause
    // CEN: Counter ENabled
    tim1.cr1.modify(|_, w| w.cen().clear_bit());
    // PSC: Prescaller
    tim1.psc.write(|w| w.psc().bits(psc));
    // ARR: Auto Reload Register
    tim1.arr.write(|w| w.arr().bits(arr));

    // URS: Update Request Source
    // Trigger an update event to load the prescaler value to the clock
    // Sets the URS bit to prevent an interrupt from being triggered by
    // the UG bit
    tim1.cr1.modify(|_, w| w.urs().set_bit());

    // EGR: Event Generation Register
    // UG: Force an update
    tim1.egr.write(|w| w.ug().set_bit());
    tim1.cr1.modify(|_, w| w.urs().clear_bit());

    // start counter
    tim1.cr1.modify(|_, w| w.cen().set_bit());

    (dma, &*scanout)
}

pub struct Cols(
    // Left Half meaning                   | Right half Meaning
    //-------------------------------------+-----------------------------------
    // Pinky col - 1                       | Pointer col + 1 and Thumb col +1
    pub PB3<Output<PushPull>>,
    // Pinky home col                      | Ponter Home col and Thumb Home col
    pub PB4<Output<PushPull>>,
    // Ring home col                       | Middle Home col and Thumb col -1
    pub PB5<Output<PushPull>>,
    // Middle Home col and Thumb col -1    | Ring Home col
    pub PB6<Output<PushPull>>,
    // Pointer Home col and Thumb Home col | Pinky Home col
    pub PB7<Output<PushPull>>,
    // Pointer col + 1 and Thumb col + 1   | Pinky col - 1
    pub PB8<Output<PushPull>>,
);

pub struct Rows(
    pub PA0<Input<PullDown>>, // Home Row + 2
    pub PA1<Input<PullDown>>, // Home Row + 1
    pub PA2<Input<PullDown>>, // Home Row
    pub PA3<Input<PullDown>>, // Home Row - 1
    pub PA4<Input<PullDown>>, // Howe Row - 2
    pub PA5<Input<PullDown>>, // Home Row - 3 and Thumb Home Row + 1
    pub PA6<Input<PullDown>>, // Thumb Home Row
    pub PA7<Input<PullDown>>, // Thumb Home Row - 1
);

#[rustfmt::skip]
pub static LAYOUT: keyberon::layout::Layers<()> = keyberon::layout::layout!{{
    [_      _     2     3      4      5    ]
    [=      1     W     E      R      T    ]
    [Tab    Q     S     D      F      G    ]
    [Escape A     X     C      V      B    ]
    [LShift Z     ~     Left   Right  _    ]
    [_      _     _     '`'
                               LShift LCtrl]
    [_      _     _     Escape Space  LAlt ]
    [_      _     _     Pause  Home   End  ]
}};

#[rustfmt::skip]
const SCANIN: [u16; 6] = [
    0b000001000,
    0b000010000,
    0b000100000,
    0b001000000,
    0b010000000,
    0b100000000
];

#[app(device = stm32f1xx_hal::pac, peripherals = true)]
mod app {
    use super::*;
    use embedded_hal::digital::v2::OutputPin;

    #[resources]
    struct Resources {
        usb_dev: UsbDevice,
        usb_class: UsbClass,
        debouncer: Debouncer<PressedKeys<U8, U6>>,
        layout: Layout<()>,
        dma: dma::dma1::Channels,
        scanout: &'static [[u8; 6]; 2],
    }

    #[init]
    fn init(c: init::Context) -> (init::LateResources, init::Monotonics) {
        static mut USB_BUS: Option<UsbBusAllocator<UsbBusType>> = None;

        let mut flash = c.device.FLASH.constrain();
        let mut rcc = c.device.RCC.constrain();
        let debouncer = Debouncer::new(PressedKeys::default(), PressedKeys::default(), 5);
        let layout = Layout::new(LAYOUT);

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

        // NOTE: These have to be setup, though they are dropped, as without this setup
        // code, it's not possible to read the matrix.
        #[rustfmt::skip]
        let _fcols = Cols(
                  pb3.into_push_pull_output(&mut gpiob.crl),
                  pb4.into_push_pull_output(&mut gpiob.crl),
            gpiob.pb5.into_push_pull_output(&mut gpiob.crl),
            gpiob.pb6.into_push_pull_output(&mut gpiob.crl),
            gpiob.pb7.into_push_pull_output(&mut gpiob.crl),
            gpiob.pb8.into_push_pull_output(&mut gpiob.crh),
        );
        let _frows = Rows(
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
            (5 * 6).khz(),
            c.device.DMA1,
            c.device.TIM1,
            &mut rcc.ahb,
            &mut rcc.apb2,
            &clocks,
        );

        (
            init::LateResources {
                usb_dev,
                usb_class,
                dma,
                scanout,
                debouncer,
                layout,
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

    #[task(binds = DMA1_CHANNEL5, priority = 1, resources = [usb_class, debouncer, layout, &dma, &scanout])]
    fn tick(mut c: tick::Context) {
        let tick::Resources {
            ref mut usb_class,
            ref mut debouncer,
            ref mut layout,
            dma,
            scanout,
        } = c.resources;
        let half = dma.5.isr().htif4().bits();
        // Clear all pending interrupts, irrespective of type
        dma.5.ifcr().write(|w| w.cgif4().clear());

        let mut events: PressedKeys<U8, U6> = PressedKeys::default();

        for i in 0..6 {
            let row: u8 = scanout[if half { 0 } else { 1 }][i];
            for bit in 0..=7 {
                if row & (1 << bit) != 0 {
                    events.0.as_mut_slice()[bit].as_mut_slice()[i] = true;
                }
            }
        }

        let report: KbHidReport = (layout, debouncer).lock(|l, d| {
            for event in d.events(events) {
                l.event(event);
            }
            l.tick();
            l.keycodes().collect()
        });

        if usb_class.lock(|k| k.device_mut().set_keyboard_report(report.clone())) {
            while let Ok(0) = usb_class.lock(|k| k.write(report.as_bytes())) {}
        }
    }
}
