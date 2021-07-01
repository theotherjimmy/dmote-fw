//! Layout management.

use crate::key_code::KeyCode;

/// It's a layout, maps (col, row) to KeyCode
pub type Layout<const COL: usize, const ROW: usize> = [[KeyCode; COL]; ROW];

pub fn keycodes<'a, const COL: usize, const ROW: usize>(
    layout: &'static Layout<COL, ROW>,
    pressed: impl Iterator<Item = (u8, u8)> + 'a
) -> impl Iterator<Item = KeyCode> + 'a {
    pressed.filter_map(move |coord|
        layout
            .get(coord.0 as usize)
            .and_then(|l| l.get(coord.1 as usize))
            .map(|k| *k)
    )
}
