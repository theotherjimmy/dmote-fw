use shared_types::DebState;

/// A quick draw style switch Schmitt trigger.
///
/// "Debouncing" is the act of converting a noisy signal into a noiseless
/// Schmitt trigger. Usually, this looks something like:
/// ```text
///                        __      ___________________________
/// Signal  ______________/  \/\/\/
///                       |           ________________________
/// Trigger _________________________/
///                       | debounce |
///                       |  period  |
///                       |  5-10ms  |
/// ```
///
/// Notice that this introduces a latency that corresponds to the period used
/// as an implementation detail of the trigger logic, typically in the range
/// of 5 to 10 milliseconds.
///
/// Idealy, We would want a Schmitt trigger that looked something like:
/// ```text
///                        __      ___________________________
/// Signal  ______________/  \/\/\/
///                        ___________________________________
/// Trigger ______________/
/// ```
///
/// That is, a Schmitt trigger with no delay between the start of the key press
/// and the resulting noiseless "triger" signal.
///
/// This debouncer is designed for minimum latency, and not much else.
///
/// The idea is that we say that a change is reported _before_ debouncing.
/// That means that we send a press/release event the moment it changes state,
/// and afterwards we ensure that we don't send any of the bounces until we're
/// confident that the key has stabilized.
///
/// So, something like the following state diagram:
///
///  ```text
///  text on arrows is input
///  [] surround output events
///  {} surround states
///  D - a down or pressed key from a scan or as an event
///  U - an up or released key from a scan or as an event
///  S - stable timeout has been reached
///
/// ┌───────────S[D]─┬─D─{Bouncing_D_D[U]}
/// │               !S     ^   │
/// │                └─────┤   U
/// ├─────────────┐        D   │
/// v             D        │   v
/// {Stable_D[D]}─┴─U─┬─>{Bouncing_D_U[U]}
/// ^                 │        │
/// │                 │        U
/// S                 └─────!S─┤
/// ├─!S───────────────┐       S
/// D                  │       │
/// │                  │       v
/// {Bouncing_U_D[D]}<─┴─D─┬─{Stable_U[U]}
/// ^     │                │   ^
/// │     U                └─U─┤
/// D     ├─────────!S─┐       │
/// │     v            │       │
/// {Bouncing_U_U[D]}─U┴─S─────┘
///  ```
///
///  Interestingly, the state diagram is rotationally semetric, leading to a
///  simplified state diagram:
///
///  ```text
///  text on arrows is input
///  [] surround output events
///  {} surround states
///  N - A key press or release
///  !N - The other kind of key press or release
///  S - stable timeout has been reached
///  N=!N - swap the press/release state
///
/// ┌───────────S─┬──N───{Bouncing_N_N[!N]}
/// │            !S        ^   │
/// │             └────────┤  !N
/// ├─────────────┐        N   │
/// v             N        │   v
/// {Stable_N[N]}─┴─!N─┬─>{Bouncing_N_!N[!N]}
/// ^                  !S    │
/// └───────S,N=!N─────┴─!N──┘
/// ```
///
/// We could rename Bouncing_N_!N to BouncingNoEvent and BouncingEvent and go
/// with that. Instead, I think of this as a single state with 2 arguments,
/// corresponding to the most recent Stable state and the current state.
/// After applying this trivial modification to the state diagram, the final
/// state diagram emerges:
///
///  ```text
///  text on arrows is input
///  [] surround output events
///  {} surround states
///  (..) Argument(s) to a state
///  N - A key press or release
///  !N - The other kind of key press or release
///  S - stable timeout has been reached
///  N=!N - swap the press/release state
///
/// ┌───────────S─┬──N───{Bouncing(N,N)*[!N]}
/// │            !S        ^   │
/// │             └────────┤  !N
/// ├──────────────┐       N   │
/// v              N       │   v
/// {Stable(N)[N]}─┴─!N─┬─>{Bouncing(N,!N)*[!N]}
/// ^                   !S   │
/// └───────S,N=!N───────┴!N─┘
/// ```
///
/// *: Note  that these states are the same state with different arguments
///
/// In all of these diagrams actually take an additional argument t, which,
/// when compared with the current timestamp, will decide between S and !S.
/// Since S and !S is not used in the transitions out of Stable, the argument
/// t is not stored in that state.
///
/// For ease of implementation, I have given the arguments to the Bouncing state
/// names. Since Stable only has one arugemnt, it's pretty clear how it should
/// be used.
#[derive(Clone, Copy, PartialEq)]
pub enum QuickDraw<const T: u8> {
    /// The key is stable at the contained state
    Stable(bool),
    /// The key is bouncing
    Bouncing {
        /// The stable state from before the bouncing began
        prior: bool,
        /// The most recent state that we observed
        current: bool,
        /// The time that we observed the current state
        since: u8,
    },
}

impl<const T: u8> Default for QuickDraw<T> {
    fn default() -> Self {
        QuickDraw::Stable(false)
    }
}

impl<const T: u8> QuickDraw<T> {
    pub fn state_name(&self) -> DebState {
        use DebState::*;
        match self {
            Self::Stable(true) => StableD,
            Self::Stable(false) => StableU,
            Self::Bouncing {
                prior: true,
                current: true,
                ..
            } => BouncingDD,
            Self::Bouncing {
                prior: true,
                current: false,
                ..
            } => BouncingDU,
            Self::Bouncing {
                prior: false,
                current: false,
                ..
            } => BouncingUU,
            Self::Bouncing {
                prior: false,
                current: true,
                ..
            } => BouncingUD,
        }
    }

    /// Is this state associated with a pressed key?
    pub fn is_pressed(&self) -> bool {
        match self {
            QuickDraw::Stable(pressed) => *pressed,
            QuickDraw::Bouncing { prior, .. } => !prior,
        }
    }

    /// Step the state machine
    ///
    /// The state machine progresses as described  in the struct documentation.
    pub fn step(&mut self, state: bool, now: u8) {
        let next_state = match self {
            QuickDraw::Stable(prior) => {
                if state != *prior {
                    QuickDraw::Bouncing {
                        prior: *prior,
                        current: state,
                        since: now,
                    }
                } else {
                    self.clone()
                }
            }
            QuickDraw::Bouncing {
                prior,
                current,
                since,
            } => {
                if state != *current {
                    // a bounce happened in the bouncing state so we reset out
                    // time stamp and  record the new state as the current
                    // state.
                    //
                    // In the state diagram, this is the 4 transitions between
                    //  the bouncing states.
                    QuickDraw::Bouncing {
                        prior: *prior,
                        current: state,
                        since: now,
                    }
                } else if now.wrapping_sub(*since) < T {
                    // no bounce happened, and we are not yet stable. Nothing
                    // happens here.
                    //
                    // This is the 4 self-transitions of the bouncing states in
                    // the state diagram.
                    self.clone()
                } else {
                    // We have hit or exceeded the stable_time and no bouncing
                    // happened.
                    //
                    // This corresponds to the transitions marked with an (S).
                    QuickDraw::Stable(*current)
                }
            }
        };
        *self = next_state;
    }
}
