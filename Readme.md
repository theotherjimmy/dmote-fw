# DMOTE Firmware

This is the firmware, in Rust, for my Dactyl Manuform: Opposable Thumbs Edition 
(or DMOTE for short).

I used a STM32F103C8 MCU, wired into the two halfs of the keyboard as the
controller. The tooling within assumes that you will flash this controller
with a DAP42 probe attached.

This firmware has some unusual features:
 * Scanning of the key matrix is done entirely with DMA, without any interaction
   with the firmware.
 * "Qick-Draw" debouncing, minimizing key press latency, even when the debounce time
   is set very long.

# Vendoring

This repo contains some vendored dependencies. In particular:

Dependency    | Reason
--------------|--------------------------------------------------------------
stm32f1       | DMA to/from registers needs the `.ptr()` method on registers 
stm32f1xx-hal | Compatibility with vendored stm32f1 dep

I'm likely to un-vendor stm32f1 and stm32f1xx-hal when versions of them
become available that meet my needs.
