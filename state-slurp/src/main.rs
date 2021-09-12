use core::mem::size_of;
use std::time::Instant;
use std::env;

use ddbug_parser::{File, FileHash};

use probe_rs::MemoryInterface;
use probe_rs::Session;

use shared_types::{KeyState, DebState, PressRelease};

fn event_at(buf: &[u32], i: usize) -> KeyState {
    let event = [buf[i * 2], buf[i * 2 + 1]];
    unsafe { core::mem::transmute(event) }
}

// The debugger reads 20480 bytes in 800ms (it's very stable too), or 25.6kbps.
// Copying all samples, 5kilohz * 6 bytes/sample, takes 30kbps. So we're going
// to have to come up with another strategy.
//
// I'm hoping that I can record all relevant events, that is all state changes
// in the debouncer, and all emitted key events. I'll have to have something
// that contains row col and the state. It would be really convenient if it fit
// in an integer number of 32bit transfers.
//
// I estimate that with the following structure, being a u64 in size, that the
// debugger would be able to read a maximum of 3200 of them in  a single
// second. I wrote a test program to test this theory, and was able to get about
// 3160 records per second.
//
// In other words, if you manage to trigger more than about 3000 events per
// second for longer than about 1/3 of a second, it will overflow and you will
// lose events. Don't type that fast.

fn main() {
    let mut head_address = None;
    let mut body_address = None;
    let mut body_size = None;
    for path in env::args().skip(1) {
        File::parse(&path, |file| {
            let hash = FileHash::new(file);
            for unit in file.units() {
                for var in unit.variables() {
                    if Some("THELOG") == var.name() {
                        let base_address = var.address();
                        if let Some(ty) = var.ty(&hash) {
                            for member in ty.members() {
                                if member.name() == Some("head") {
                                    head_address = base_address.map(
                                        |a| a.wrapping_add(member.bit_offset() / 8)
                                    );
                                }
                                if member.name() == Some("body") {
                                    body_address = base_address.map(
                                        |a| a.wrapping_add(member.bit_offset() / 8)
                                    );
                                    body_size = member.bit_size(&hash).map(
                                        |s| s / ((size_of::<KeyState>() * 8) as u64)
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Ok(())
        }).unwrap();
    }
    let head = head_address.unwrap();
    let body = body_address.unwrap();
    let size = body_size.unwrap();
    let mut sesh = Session::auto_attach("stm32f103c8").unwrap();
    let mut core = sesh.core(0).unwrap();
    let head_val = core.read_word_32(head as u32).unwrap() as u64;
    assert!((head_val as u64) < size);
    let mut buf = vec![0; size as usize * (size_of::<KeyState>() / size_of::<u32>())];
    let before = Instant::now();
    core.read_32(body as u32, &mut buf).unwrap();
    let duration = before.elapsed();
    let start_time = (event_at(&buf, head_val as usize).timestamp as u64) * (1_000_000_000 / 2_000);
    println!(r#"{{
        "title": "keyboard debouncing",
        "start": [0, {}],
        "states": {{
            "stable-release": {{ "value": 0, "color": "white"}},
            "bouncing-rel-to-pre": {{ "value": 1,  "color": "blue"}},
            "bouncing-rel-to-rel": {{ "value": 2, "color": "brown" }},
            "emit-release": {{ "value" : 3, "color": "white" }},
            "stable-press": {{ "value": 4, "color": "grey" }},
            "bouncing-pre-to-pre": {{ "value": 5, "color": "yellow" }},
            "bouncing-pre-to-rel": {{ "value": 6, "color": "orange" }},
            "emit-press": {{ "value" : 7, "color": "black" }}
        }}
    }}"#, start_time);
    for i in (head_val..size).chain(0..head_val) {
        let event = event_at(&buf, i as usize);
        let ns_time = ((event.timestamp as u64) * (1_000_000_000 / 2_000)) - start_time;
        println!(r#"{{
            "entity": "{}-{}-debouncer",
            "time": "{}",
            "state": {},
            "tag": null
        }}"#, event.row, event.col, ns_time, match event.deb {
            DebState::StableU    => 0,
            DebState::BouncingUD => 1,
            DebState::BouncingUU => 2,
            DebState::StableD    => 4,
            DebState::BouncingDD => 5,
            DebState::BouncingDU => 6,
        });
        if event.event != PressRelease::None {
            println!(r#"{{
                "entity": "{}-{}-trigger",
                "time": "{}",
                "state": {},
                "tag": null
            }}"#, event.row, event.col, ns_time, match event.event {
                PressRelease::Press   => 7,
                PressRelease::Release => 3,
                PressRelease::None    => unreachable!(),
            });
        }
    }
    eprintln!("Slurped {} records in {:?}", size, duration);
}
