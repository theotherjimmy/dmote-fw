//! Layout management.

use crate::key_code::KeyCode;

/// It's a layout, maps (col, row) to KeyCode
pub type Layout<const COL: usize, const ROW: usize> = [[KeyCode; COL]; ROW];

pub fn keycode<const COL: usize, const ROW: usize>(
    layout: &'static Layout<COL, ROW>,
    row: usize,
    col: usize,
) -> Option<&'static KeyCode> {
    layout
        .get(row)
        .and_then(|l| l.get(col))
}
