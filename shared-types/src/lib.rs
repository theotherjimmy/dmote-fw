#![no_std]
#[derive(Debug, Copy, Clone, PartialEq)]
#[repr(u8)]
pub enum DebState {
    StableU,
    BouncingUD,
    BouncingUU,
    StableD,
    BouncingDD,
    BouncingDU,
}

#[derive(Debug, Copy, Clone, PartialEq)]
#[repr(u8)]
pub enum PressRelease {
    None,
    Press,
    Release,
}

/// A packed representation of any debounce event used for observing the state
/// of debouncing with a debugger.
#[derive(Debug, Copy, Clone, PartialEq)]
#[repr(C)]
pub struct KeyState {
    /// The Time that this state change happened
    pub timestamp: u32,
    /// The row that changed
    pub row: u8,
    /// The column that changed
    pub col: u8,
    /// The new state
    pub deb: DebState,
    /// The event that was produced, if any
    pub event: PressRelease,
}

impl Default for KeyState {
    fn default() -> Self {
        Self {
            timestamp: 0,
            row: 0,
            col: 0,
            deb: DebState::StableU,
            event: PressRelease::None,
        }
    }
}
