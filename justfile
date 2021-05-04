build-dir := "target/thumbv7m-none-eabi/"
fw-name := "/dmote-fw"
profile := "release"


# Hidden because it's not meant to be run from the command line
_openocd-pipe:
    #!/usr/bin/env -S openocd -p -f
    script interface/cmsis-dap.cfg
    cmsis_dap_vid_pid 0x1209 0xda42
    script target/stm32f1x.cfg

debug side: build
    #!/usr/bin/env -S gdb -q -ix
    file {{build-dir}}{{profile}}/{{side}}
    target extended-remote | just _openocd-pipe
    load

flash side: build
    #!/usr/bin/env -S gdb -q --batch -x
    set style enable on
    file {{build-dir}}{{profile}}/{{side}}
    target extended-remote | just _openocd-pipe
    monitor reset
    load
    monitor reset run

build:
    cargo build -Z unstable-options --profile {{profile}}
