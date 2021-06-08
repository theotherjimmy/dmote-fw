use core::ops::Range;
use core::mem::size_of;
use std::time::Instant;

use probe_rs::Session;
use probe_rs::MemoryInterface;
use probe_rs::config::{MemoryRegion, RamRegion};

fn get_ram(sesh: &Session) -> Option<Range<u32>> {
    for mem in sesh.memory_map() {
        match mem {
            MemoryRegion::Ram(RamRegion{range, ..}) => return Some(range.clone()),
            _ => ()
        }
    }
    None
}
// The debugger reads 20480 bytes in 800ms (it's very stable too),
// or 25.6 kbps.
// Copying all samples, 5kilohz * 6 bytes/sample, takes 30kbps.
// So we're going to have to come up with another strategy.
//
// I'm hoping that I can record all relevant events, that is
// all state changes in the debouncer. I'll have to have
// something that contains row col and the state. It would be really
// convinient if it fit in an integer number of 32bit transfers.


fn main() {
    let mut sesh = Session::auto_attach("stm32f103c8").unwrap();
    let ram = get_ram(&sesh).unwrap();
    let mut core = sesh.core(0).unwrap();
    let mut buf = vec![0; ram.len() / size_of::<u32>()];
    let before = Instant::now();
    core.read_32(ram.start, &mut buf).unwrap();
    let duration = before.elapsed();
    eprintln!("Slurped {} bytes in {:?}", ram.len(), duration);
}
