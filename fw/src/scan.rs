use core::sync::atomic::{AtomicBool, Ordering};
use cortex_m::singleton;
use stm32f1::stm32f103;
use stm32f1xx_hal::gpio::{
    gpioa::{PA0, PA1, PA2, PA3, PA4, PA5},
    gpiob::{PB10, PB11, PB12, PB13, PB14, PB15, PB3, PB4, PB5, PB6, PB7, PB8, PB9},
    Input, Output, PullDown, PushPull,
};
use stm32f1xx_hal::prelude::*;
use stm32f1xx_hal::rcc::{Clocks, Enable, GetBusFreq, Reset, AHB, APB2};
use stm32f1xx_hal::time::Hertz;
use stm32f1xx_hal::{dma, pac};

use shared_types::{DebState, KeyState, PressRelease};

use crate::key_code::{keycode, KbHidReport, Layout};
use crate::trigger::QuickDraw;

/// Compute the Auto Reload Register and Prescaller Register values for a timer
#[inline(always)]
fn compute_arr_presc(freq: u32, clock: u32) -> (u16, u16) {
    let ticks = clock / freq;
    let psc = ((ticks - 1) / (1 << 16)) as u16;
    let arr = (ticks / (psc + 1) as u32) as u16;
    (psc, arr)
}

/// Columns of the keyboard matrix
///
/// Pin | Left Half wiring                  | Right half wiring
/// ----|-----------------------------------|---------------------------------
/// PA0 | Pinky col - 1                     | Pointer col + 1 & Thumb col +1
/// PA1 | Pinky home col                    | Ponter Home col & Thumb Home col
/// PA2 | Ring home col                     | Middle Home col & Thumb col -1
/// PA3 | Middle Home col & Thumb col -1    | Ring Home col
/// PA4 | Pointer Home col & Thumb Home col | Pinky Home col
/// PA5 | Pointer col + 1 & Thumb col + 1   | Pinky col - 1
pub struct Cols(
    pub PA0<Output<PushPull>>,
    pub PA1<Output<PushPull>>,
    pub PA2<Output<PushPull>>,
    pub PA3<Output<PushPull>>,
    pub PA4<Output<PushPull>>,
    pub PA5<Output<PushPull>>,
);

/// Rows of the keyboard matrix
///
/// Pin  | Half  | Wiring
/// -----|-------|--------------------------
/// PB3  | Left  | Home Row + 2
/// PB4  | Left  | Home Row + 1
/// PB5  | Left  | Home Row for non-Pinky fingers
/// PB6  | Left  | Home Row - 1 and Pinky Home
/// PB7  | Left  | Home Row - 2
/// PB8  | Both  | Home Row - 3 & Thumb Home Row + 1
/// PB9  | Both  | Thumb Home Row
/// PB10 | Both  | Thumb Home Row - 1
/// PB11 | Right | Home Row + 2
/// PB12 | Right | Home Row + 1
/// PB13 | Right | Home Row for non-Pinky fingers
/// PB14 | Right | Home Row - 1 and Pinky Home
/// PB15 | Right | Howe Row - 2
pub struct Rows(
    pub PB3<Input<PullDown>>,
    pub PB4<Input<PullDown>>,
    pub PB5<Input<PullDown>>,
    pub PB6<Input<PullDown>>,
    pub PB7<Input<PullDown>>,
    pub PB8<Input<PullDown>>,
    pub PB9<Input<PullDown>>,
    pub PB10<Input<PullDown>>,
    pub PB11<Input<PullDown>>,
    pub PB12<Input<PullDown>>,
    pub PB13<Input<PullDown>>,
    pub PB14<Input<PullDown>>,
    pub PB15<Input<PullDown>>,
);

/// All gpios used by the key matrix.
pub struct Matrix {
    pub rows: Rows,
    pub cols: Cols,
}

/**
 * Setup DMA to scan an 13 row, 6 column keyboard matrix.
 *
 * # Matrix Scanning
 *
 * We setup a PWM timer and a few DMA cyclic transfers to make the DMA hardware
 * scan the keyboard matrix without the involvement of the CPU. This allows faster
 * scans or more time spent asleep, as the CPU can spend all of it's time handling
 * debouncing, matrix to keycode translation and USB traffic.
 *
 * The matrix will be represented in a semi-packed way, in that the scans will
 * produce a u16 per row with a bit for each matrix intersection, and an extra 3
 * bits.
 *
 * Starting from the timer initilalization, the DMA implement the following
 * timing:
 * ```text
 * TIM1 |    0    |    1    |    2    |    3    |    4    |    0    |
 * OUT  |000000000|000000000|000001000|000001000|000001000|000001000|-\
 * IN   |000000000|000000000|Settling |Settling |Settling |rrrrrrrrr| |
 *    ________________________________________________________________/
 *   /
 *   |  |    1    |    2    |    3    |    4    |    0    |    1    |
 *   \->|000001000|000010000|000010000|000010000|000010000|000010000| -> etc.
 *      |rrrrrrrrr|Settling |Settling |Settling |rrrrrrrrr|rrrrrrrrr|
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
// TODO: better return type? Perhaps it would be better to accept DMA1CH4 and DMA1CH5
// and return DMA1CH5's interrupt status register?
pub fn dma_key_scan(
    freq: impl Into<Hertz>,
    _matrix: Matrix,
    dma: pac::DMA1,
    tim1: pac::TIM1,
    ahb: &mut AHB,
    apb2: &mut APB2,
    clocks: &Clocks,
) -> (dma::dma1::Channels, &'static [[u16; 6]; 2]) {
    // Values to be written to the Bit Set & Reset Register (BSRR).
    //
    // The upper 16 bits (16..=31) set pins to 0 when written (reset), and the
    // lower 16 bits (0..=15) set pins to 1 when written (set). This way we won't attept
    // to write to bits that are not part of those that are part of the matrix
    #[rustfmt::skip]
    const SCANIN: [u32; 6] = [
        (0b111110 << 16) | 0b000001,
        (0b111101 << 16) | 0b000010,
        (0b111011 << 16) | 0b000100,
        (0b110111 << 16) | 0b001000,
        (0b101111 << 16) | 0b010000,
        (0b011111 << 16) | 0b100000,
    ];
    let mut dma = dma.split(ahb);
    let scanout = singleton!(: [[u16; 6]; 2] = [[0; 6]; 2]).unwrap();

    // Implementation Notes:
    //
    // To acomplish the timing diagram in the doc comment, we have to setup Timer 1
    // to have a period that matches 6 * the input frequency, and we have to setup output
    // compare for the 2/5 point of that period.
    //
    // DMA CH2 is connected to the output compare 1, so it was used as the column strobe
    // signal. However, It's also triggered by a UART3 TX empty fifo, which may always
    // be empty. This causes an unending cascade of spurious DMA requests that causes
    // the columns to be strobed as fast as the memory bus allows. This breaks the
    // synchronization between the row read and column strobe, which is required for this
    // code to function.
    //
    // Instead, we use output compare 4, which is mapped to DMA CH4, for column strobe.
    //
    // I could have also disabled the DMA request, but it seems a bit harder than changing
    // output compare and DMA channels, as `s/dma.2/dma.4/g`, `s/cc1/cc4/g` etc. suffices.
    //
    // DMA CH5 is connected to the the update/reset of the timer, so it must be the row
    // read.
    //
    // Registers initialisms are defined in line

    // # DMA1 CH4: Requested by Output Compare 4 (ch4) with Timer 1
    dma.4.set_peripheral_address(
        // Safety: we don't enable pointer incrimenting of Perihperal addresses
        // Further, this pointer dereference is always safe.
        unsafe { (*stm32f103::GPIOA::ptr()).bsrr.as_ptr() } as u32,
        false,
    );
    // Safety: we have the lenth correct below. This should probably be unsafe, because
    // we're asking the DMA hardware to derefrence a raw pointer. But hey, it's not.
    dma.4.set_memory_address(SCANIN.as_ptr() as u32, true);
    dma.4
        .set_transfer_length(core::mem::size_of_val(&SCANIN) / core::mem::size_of_val(&SCANIN[0]));
    #[rustfmt::skip]
    dma.4.ch().cr.modify(|_read, write| {
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
            // We're storing to the BSRR, which is of size 32
            .msize().bits32()
    });

    // # DMA1 CH5: Requested by Update/Overflow of Timer 1
    dma.5.set_peripheral_address(
        // Safety: we don't enable pointer incrimenting of Perihperal addresses
        // Further, this pointer dereference is always safe.
        unsafe { (*stm32f103::GPIOB::ptr()).idr.as_ptr() } as *const u16 as u32,
        false,
    );
    // Safety: we set the transfer length correctly, and we only read the half of the
    // buffer that's not in use by DMA.
    dma.5
        .set_memory_address(scanout.as_mut_ptr() as *mut u8 as u32, true);
    // NOTE: This is the number of elements.
    dma.5.set_transfer_length(
        core::mem::size_of_val(scanout) / core::mem::size_of_val(&scanout[0][0]),
    );
    #[rustfmt::skip]
    dma.5.ch().cr.modify(|_read, write| {
        write
            .en().enabled()
            .circ().enabled()
            .dir().from_peripheral()
            .minc().enabled()
            .psize().bits32()
            .msize().bits16()
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
    // CCR4: Counter Compare Register 4 (channel 4, I think).
    // CCR: Courter Compare Register (it's the value to compare with).
    tim1.ccr4.modify(|_, w| w.ccr().bits(arr * 2 / 5));
    // Impl NOTE: We enable the follwing
    // UDE: Update DMA Event
    // CC4DE: Counter Compare 4 DMA Event
    tim1.dier.modify(|_, w| w.ude().enabled().cc4de().enabled());
    // CC4E: Counter Compare 4 Enable (should probably be .enabled, but for some reason
    // the hal only exports .set_bit)
    tim1.ccer.modify(|_, w| w.cc4e().set_bit());

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

const LOG_SIZE: usize = 1024;

/// A log structure that's accessable by the debugger
///
/// It's actually a circular buffer that writes over itself when full. The hope
/// is that a debugger, or the state-slurp program will be able to keep up with
/// the pace of adding events. A debugger can read 3200 records at full spead,
/// when it's doing nothing else, so it should be able to reasonably keep up
/// with something like 500 events/second, which would be quite a few events.
/// Further, if the debugger falls behind, it would have to fall behind by the
/// size of the log, 1024, in order to actually loose data. If the debugger
/// is unable to keep up, but then there is a lul in activity, it should be
/// possible for the debugger to catch up eventually.
///
pub struct Log {
    /// Location of the next b
    head: usize,
    body: [KeyState; LOG_SIZE],
}

static mut THELOG: Log = Log {
    head: 0,
    body: [KeyState {
        timestamp: 0,
        col: 0,
        row: 0,
        deb: DebState::StableU,
        event: PressRelease::None,
    }; LOG_SIZE],
};
impl Log {
    pub fn log(&mut self, elem: KeyState) {
        self.body[self.head] = elem;
        self.head += 1;
        self.head %= LOG_SIZE;
    }

    /// Return the log singleton. Panics if called twice
    pub fn get() -> &'static mut Self {
        // NOTE: This is a manual implementation of the singleton macro so that the
        // names are more predictable
        static TAKEN: AtomicBool = AtomicBool::new(false);
        if TAKEN.swap(true, Ordering::AcqRel) {
            // The aforementioned panic when called twice
            panic!();
        }
        unsafe { &mut THELOG }
    }
}

/// Zero sized type that binds a scan to reporting. This requires you to do a
/// build a report with that data. My hope is that this will help prevent a
/// user from forgetting to scan first.
pub struct ReportToken();

/// Scan all keys into the triggers and generate a HID report.
pub fn scan<'a, const R: usize, const C: usize, const T: u8>(
    scanout_half: &'a [u16; C],
    triggers: &'a mut [[QuickDraw<T>; R]; C],
    log: &'a mut Log,
    timestamp: u32,
) -> ReportToken {
    for (col, (row_val, trigger_row)) in scanout_half.iter().zip(&mut triggers[..]).enumerate() {
        for row in 0..R {
            let press = (row_val & (1 << (row + 3))) != 0;
            let old: QuickDraw<T> = trigger_row[row].clone();
            let is_old_pressed = old.is_pressed();
            trigger_row[row].step(press, timestamp as u8);
            let new = &trigger_row[row];
            let is_new_pressed = new.is_pressed();
            if *new != old {
                let event = if is_old_pressed == is_new_pressed {
                    PressRelease::None
                } else if is_old_pressed {
                    PressRelease::Release
                } else {
                    PressRelease::Press
                };
                log.log(KeyState {
                    timestamp,
                    row: row as u8,
                    col: col as u8,
                    deb: new.state_name(),
                    event,
                });
            }
        }
    }
    ReportToken()
}

pub fn report<'a, const R: usize, const C: usize, const T: u8>(
    layout: &'static Layout<R, C>,
    triggers: &'a [[QuickDraw<T>; R]; C],
    #[allow(unused_variables)]
    token: ReportToken,
) -> KbHidReport {
    let mut rep = KbHidReport::default();
    for (col, trigger_row) in triggers.iter().enumerate() {
        for row in 0..R {
            if trigger_row[row].is_pressed() {
                if let Some(&kc) = keycode(layout, row, col) {
                    rep.pressed(kc);
                }
            }
        }
    }
    rep
}
