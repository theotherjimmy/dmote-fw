//! Layout management.

use crate::key_code::KeyCode;
use heapless::Vec;

/// The Layers type.
///
/// The first level correspond to the layer, the two others to the
/// switch matrix.  For example, `layers[1][2][3]` correspond to the
/// key i=2, j=3 on the layer 1.
/// TODO: The above is incorrect
pub type Layers<const COL: usize, const ROW: usize> = [[KeyCode; COL]; ROW];


#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LogicalState {
    Press,
    Release,
}

/// An event on the key matrix.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Event {
    pub coord: (u8, u8),
    pub state: LogicalState,
}
impl Event {
    /// Returns `true` if the event is a key press.
    pub fn is_press(self) -> bool {
        self.state == LogicalState::Press
    }

    /// Returns `true` if the event is a key release.
    pub fn is_release(self) -> bool {
        self.state == LogicalState::Release
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
struct State {
    keycode: KeyCode,
    coord: (u8, u8)
}

/// The layout manager. It takes `Event`s and `tick`s as input, and
/// generate keyboard reports.
pub struct Layout<const COL: usize, const ROW: usize> {
    layers: Layers<COL, ROW>,
    states: Vec<State, 64>,
}

impl<const COL: usize, const ROW: usize> Layout<COL, ROW> {
    /// Creates a new `Layout` object.
    pub fn new(layers: Layers<COL, ROW>) -> Self {
        Self {
            layers,
            states: Vec::new(),
        }
    }
    /// Iterates on the key codes of the current state.
    pub fn keycodes(&self) -> impl Iterator<Item = KeyCode> + '_ {
        self.states.iter().map(|s| s.keycode)
    }

    /// Register a key event.
    pub fn event(&mut self, event: Event) {
        match event.state {
            LogicalState::Release => {
                self.states = self.states
                    .iter()
                    .filter(|s| s.coord != event.coord)
                    .cloned()
                    .collect();
            }
            LogicalState::Press => {
                let kc = self
                    .layers
                    .get(event.coord.0 as usize)
                    .and_then(|l| l.get(event.coord.1 as usize))
                    .map(|k| *k)
                    .unwrap_or(KeyCode::No);
                if kc != KeyCode::No {
                    let _ = self.states.push(State { coord: event.coord , keycode: kc});
                }
            }
        }
    }
}
