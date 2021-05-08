#![no_std]
use cortex_m::singleton;
use generic_array::typenum::{U6, U8};
use keyberon::matrix::PressedKeys;
use packed_struct::prelude::*;
use stm32f1::stm32f103;
use stm32f1xx_hal::gpio::{
    gpioa::{PA0, PA1, PA2, PA3, PA4, PA5, PA6, PA7},
    gpiob::{PB3, PB4, PB5, PB6, PB7, PB8},
    Input, Output, PullDown, PushPull,
};
use stm32f1xx_hal::prelude::*;
use stm32f1xx_hal::rcc::{Clocks, Enable, GetBusFreq, Reset, AHB, APB2};
use stm32f1xx_hal::time::Hertz;
use stm32f1xx_hal::{dma, pac};

#[derive(PackedStruct, Debug, Copy, Clone, PartialEq)]
#[packed_struct(bit_numbering = "msb0")]
pub struct KeyEvent {
    #[packed_field(bits = "0..=2")]
    pub row: Integer<u8, packed_bits::Bits3>,
    #[packed_field(bits = "3..=5")]
    pub col: Integer<u8, packed_bits::Bits3>,
    #[packed_field(bits = "7")]
    pub brk: bool,
}

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
/// Pin| Left Half wiring                    | Right half wiring
/// ---|-----------------------------------|-----------------------------------
/// PB3| Pinky col - 1                     | Pointer col + 1 & Thumb col +1
/// PB4| Pinky home col                    | Ponter Home col & Thumb Home col
/// PB5| Ring home col                     | Middle Home col & Thumb col -1
/// PB6| Middle Home col & Thumb col -1    | Ring Home col
/// PB7| Pointer Home col & Thumb Home col | Pinky Home col
/// PB8| Pointer col + 1 & Thumb col + 1   | Pinky col - 1
pub struct Cols(
    pub PB3<Output<PushPull>>,
    pub PB4<Output<PushPull>>,
    pub PB5<Output<PushPull>>,
    pub PB6<Output<PushPull>>,
    pub PB7<Output<PushPull>>,
    pub PB8<Output<PushPull>>,
);

/// Rows of the keyboard matrix
///
/// Pin | Wiring for both halfs
/// ----|----------------------------------
/// PA0 | Home Row + 2
/// PA1 | Home Row + 1
/// PA2 | Home Row for non-Pinky fingers
/// PA3 | Home Row - 1 and Pinky Home
/// PA4 | Howe Row - 2
/// PA5 | Home Row - 3 & Thumb Home Row + 1
/// PA6 | Thumb Home Row
/// PA7 | Thumb Home Row - 1
pub struct Rows(
    pub PA0<Input<PullDown>>,
    pub PA1<Input<PullDown>>,
    pub PA2<Input<PullDown>>,
    pub PA3<Input<PullDown>>,
    pub PA4<Input<PullDown>>,
    pub PA5<Input<PullDown>>,
    pub PA6<Input<PullDown>>,
    pub PA7<Input<PullDown>>,
);

/// All gpios used by the key matrix.
pub struct Matrix {
    pub rows: Rows,
    pub cols: Cols,
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
) -> (dma::dma1::Channels, &'static [[u8; 6]; 2]) {
    // Values to be written to the Bit Set & Reset Register (BSRR).
    //
    // The upper 16 bits (16..=31) set pins to 0 when written (reset), and the
    // lower 16 bits (0..=15) set pins to 1 when written (set). This way we won't attept
    // to write to bits that are not part of those that are part of the matrix
    #[rustfmt::skip]
    const SCANIN: [u32; 6] = [
        (0b111110000 << 16) | 0b000001000,
        (0b111101000 << 16) | 0b000010000,
        (0b111011000 << 16) | 0b000100000,
        (0b110111000 << 16) | 0b001000000,
        (0b101111000 << 16) | 0b010000000,
        (0b011111000 << 16) | 0b100000000,
    ];
    let mut dma = dma.split(ahb);
    let scanout = singleton!(: [[u8; 6]; 2] = [[0; 6]; 2]).unwrap();

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
        unsafe { (*stm32f103::GPIOB::ptr()).bsrr.as_ptr() } as u32,
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
        unsafe { (*stm32f103::GPIOA::ptr()).idr.as_ptr() } as *const u16 as u32,
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

pub fn keys_from_scan(scanout_half: &[u8; 6]) -> PressedKeys<U8, U6> {
    let mut events: PressedKeys<U8, U6> = PressedKeys::default();

    for i in 0..6 {
        let row = scanout_half[i];
        for bit in 0..=7 {
            if row & (1 << bit) != 0 {
                events.0.as_mut_slice()[bit].as_mut_slice()[i] = true;
            }
        }
    }
    events
}


/// Between the halfs of my keyboard, there is a phone line (RJ9) serial
/// connection. I tried higher speeds, but they were not as reliable.
///
/// This is the baud rate for that Serial.
/// Use this by called `.bps()` on this value.
///
/// TODO: Rework this when the following is not an error:
///  error[E0015]: calls in constants are limited to constant functions,
///  tuple structs and tuple variants
///     --> src/lib.rs:319:30
///      |
///  319 | const PHONE_LINE_BAUD: Bps = 115_200.bps();
pub const PHONE_LINE_BAUD: u32 = 115_200;
