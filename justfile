build-dir := "target/thumbv7m-none-eabi/"
connect := "target extended-remote | just _openocd-pipe"


# Hidden because it's not meant to be run from the command line
_openocd-pipe:
    #!/usr/bin/env -S openocd -p -f
    script interface/cmsis-dap.cfg
    cmsis_dap_vid_pid 0x1209 0xda42
    script target/stm32f1x.cfg

# Debug a the side of the keyboard with gdb
debug side: build
    #!/usr/bin/env -S gdb -q -ix
    file {{build-dir}}/release/{{side}}
    {{connect}}
    load

# Flash a the side of the keyboard with gdb
flash side: build
    #!/usr/bin/env -S gdb -q --batch -x
    set style enable on
    file {{build-dir}}/release/{{side}}
    {{connect}}
    monitor reset
    load
    monitor reset run

# Build firmware for both the left and right side of the keyboard
build:
    cargo build --release
