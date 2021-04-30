build-dir := "target/thumbv7m-none-eabi/"
fw-name := "/dmote-fw"


# Hidden because it's not meant to be run from the command line
_openocd-pipe:
    #!/usr/bin/env -S openocd -p -f
    script interface/cmsis-dap.cfg
    script target/stm32f1x.cfg

debug profile side: (build profile side)
    #!/usr/bin/env -S gdb -q -ix
    file {{build-dir}}{{profile}}{{fw-name}}
    target extended-remote | just _openocd-pipe
    load

flash profile side: (build profile side)
    #!/usr/bin/env -S gdb -q --batch -x
    set style enable on
    file {{build-dir}}{{profile}}{{fw-name}}
    target extended-remote | just _openocd-pipe
    monitor reset
    load
    monitor reset run

build profile side:
    cargo build -Z unstable-options --profile {{profile}} --features {{side}}
