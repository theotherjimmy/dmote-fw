# Keyberon - Forked and Vendored

A rust crate to create a pure rust keyboard firmware.

It is exposed as a library giving you the different building blocks to
create a featureful keyboard firmware. As the different functionality
are interconected by the user of the crate, you can use only the parts
you are interested in or easily insert your own code in between.

This crate is a no_std crate, running on stable rust. To use it on a
given MCU, you need GPIO throw the [embedded hal
crate](https://crates.io/crates/embedded-hal) to read the key states,
and the [usb-device crate](https://crates.io/crates/usb-device) for
USB communication.

## Features

The supported features are:
 - Layers when holding a key (aka the fn key). When holding multiple
   layer keys, the numbers add (if you have a layer 1 key and a layer
   2 key, when holding the 2 together, the layer 3 will be active).
 - Transparent key, i.e. when on an alternative layer, the key will
   inherit the behavior of the default layer.
 - Change default layer dynamically.
 - Multiple keys sent on an single key press. It allows to have keys
   for complex shortcut, for example a key for copy and paste or alt tab, or
   for whatever you want.
 - hold tap: different action depending if the key is held or
   tapped. For example, you can have a key acting as layer change when
   held, and space when tapped.
   

## FAQ

### Keyberon, what's that name?

To find new, findable and memorable project names, some persons in the rust community try to mix the name of a city with some keyword related to the project. For example, you have the [Tokio project](https://tokio.rs/) that derive its name from the Japanese capital Tokyo and IO for Input Output, the main subject of this project.

So, I have to find such a name. In the mechanical keyboard community, "keeb" is slang for keyboard. Thus, I searched for a city with the sound [kib], preferably in France as it is the country of origin of the project. I found [Quiberon](https://en.wikipedia.org/wiki/Quiberon), and thus I named the project Keyberon.
