//! The wake and command state machine.

use crate::wake::is_wake_word;

/// Longest wake clip, from the speech onset.
pub const WAKE_MAX_MS: u64 = 3_000;

/// How long a command may take to start.
pub const COMMAND_WAIT_MS: u64 = 10_000;

/// Longest command, from its onset.
pub const COMMAND_MAX_MS: u64 = 30_000;

/// What the demo is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Listening for speech that may be the wake word.
    AwaitWake,
    /// Recording and transcribing the word to check it.
    ConfirmWake,
    /// Waiting for a command and recording it.
    AwaitCommand,
    /// Transcribing the command.
    HandleCommand,
}

impl State {
    /// Whether the green LED is on in this state.
    pub const fn indicator_on(self) -> bool {
        matches!(self, Self::AwaitCommand)
    }
}

/// Something that happened to the demo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event<'t> {
    /// The microphone started a fresh capture.
    CaptureStarted,
    /// The segmenter heard speech begin.
    SpeechStarted,
    /// The segmenter discarded a burst that was too short.
    SpeechDropped,
    /// The segmenter completed an utterance.
    UtteranceReady,
    /// The requested transcription finished.
    Transcript(&'t str),
    /// Time passed.
    Tick,
    /// The microphone stopped delivering audio.
    CaptureFailed,
}

/// What the firmware has to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Run the entry actions of a state.
    Enter(State),
    /// Transcribe the recorded audio and report it as [`Event::Transcript`].
    Transcribe,
}

/// Decides the transitions between the states.
///
/// Any event not listed here changes nothing.
///
/// - Every state: `CaptureFailed` enters `AwaitWake`.
/// - `AwaitWake`: `SpeechStarted` enters `ConfirmWake`.
/// - `ConfirmWake`, recording: `SpeechDropped` enters `AwaitWake`;
///   `UtteranceReady`, or a `Tick` [`WAKE_MAX_MS`] after the onset, asks for
///   the transcription.
/// - `ConfirmWake`, transcribing: `Transcript` enters `AwaitCommand` when it
///   holds the wake word and `AwaitWake` otherwise.
/// - `AwaitCommand`, silent: `CaptureStarted` restarts the wait; a `Tick`
///   [`COMMAND_WAIT_MS`] after the start of the wait enters `AwaitWake`;
///   `SpeechStarted` records the onset.
/// - `AwaitCommand`, speaking: `SpeechDropped` forgets the onset, so the wait
///   still counts from its start; `UtteranceReady`, or a `Tick` [`COMMAND_MAX_MS`]
///   after the onset, enters `HandleCommand`.
/// - `HandleCommand`: `Transcript` enters `AwaitWake`.
#[derive(Debug)]
pub struct Machine {
    state: State,
    entered_at: u64,
    /// When the speech that is being recorded began.
    speech_at: Option<u64>,
    /// Whether the transcription of the wake clip was requested.
    transcribing: bool,
}

impl Machine {
    /// A machine in `AwaitWake`.
    pub const fn new(now_ms: u64) -> Self {
        Self {
            state: State::AwaitWake,
            entered_at: now_ms,
            speech_at: None,
            transcribing: false,
        }
    }

    /// Returns the current state.
    pub const fn state(&self) -> State {
        self.state
    }

    /// Applies `event`, and returns what the firmware has to do.
    pub fn on_event(&mut self, event: Event<'_>, now_ms: u64) -> Option<Effect> {
        let elapsed = |since: u64| now_ms.saturating_sub(since);
        let next = match (self.state, event, self.speech_at) {
            (_, Event::CaptureFailed, _) | (State::HandleCommand, Event::Transcript(_), _) => {
                State::AwaitWake
            }
            (State::AwaitWake, Event::SpeechStarted, _) => State::ConfirmWake,
            (State::ConfirmWake, Event::Transcript(text), _) if self.transcribing => {
                if is_wake_word(text) {
                    State::AwaitCommand
                } else {
                    State::AwaitWake
                }
            }
            (State::ConfirmWake, _, _) if self.transcribing => return None,
            (State::ConfirmWake, Event::SpeechDropped, _) => State::AwaitWake,
            (State::ConfirmWake, Event::UtteranceReady, _) => return Some(self.transcribe()),
            (State::ConfirmWake, Event::Tick, Some(onset)) if elapsed(onset) >= WAKE_MAX_MS => {
                return Some(self.transcribe());
            }
            (State::AwaitCommand, Event::Tick, None)
                if elapsed(self.entered_at) >= COMMAND_WAIT_MS =>
            {
                State::AwaitWake
            }
            (State::AwaitCommand, Event::CaptureStarted, None) => {
                self.entered_at = now_ms;
                return None;
            }
            (State::AwaitCommand, Event::SpeechStarted, None) => {
                self.speech_at = Some(now_ms);
                return None;
            }
            (State::AwaitCommand, Event::SpeechDropped, Some(_)) => {
                self.speech_at = None;
                return None;
            }
            (State::AwaitCommand, Event::UtteranceReady, Some(_)) => State::HandleCommand,
            (State::AwaitCommand, Event::Tick, Some(onset)) if elapsed(onset) >= COMMAND_MAX_MS => {
                State::HandleCommand
            }
            _ => return None,
        };
        Some(self.enter(next, now_ms))
    }

    fn enter(&mut self, state: State, now_ms: u64) -> Effect {
        self.state = state;
        self.entered_at = now_ms;
        // The wake clip starts with the speech that opens `ConfirmWake`.
        self.speech_at = (state == State::ConfirmWake).then_some(now_ms);
        self.transcribing = false;
        Effect::Enter(state)
    }

    fn transcribe(&mut self) -> Effect {
        self.transcribing = true;
        Effect::Transcribe
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine that has reached `state` through the shortest path.
    fn machine_in(state: State) -> Machine {
        let mut machine = Machine::new(0);
        let path = [
            (Event::SpeechStarted, 0),
            (Event::UtteranceReady, 500),
            (Event::Transcript("doufu"), 1_000),
            (Event::UtteranceReady, 2_000),
        ];
        let events = match state {
            State::AwaitWake => 0,
            State::ConfirmWake => 1,
            State::AwaitCommand => 3,
            State::HandleCommand => 4,
        };
        // A command only ends after its speech has started.
        for (nth, &(event, now)) in path.iter().take(events).enumerate() {
            if nth == 3 {
                machine.on_event(Event::SpeechStarted, 1_500);
            }
            machine.on_event(event, now);
        }
        assert_eq!(machine.state(), state);
        machine
    }

    const ALL: [State; 4] = [
        State::AwaitWake,
        State::ConfirmWake,
        State::AwaitCommand,
        State::HandleCommand,
    ];

    #[test]
    fn should_start_in_await_wake() {
        assert_eq!(Machine::new(0).state(), State::AwaitWake);
    }

    #[test]
    fn should_light_the_indicator_only_while_awaiting_a_command() {
        for (state, on) in ALL.into_iter().zip([false, false, true, false]) {
            assert_eq!(state.indicator_on(), on, "{state:?}");
        }
    }

    #[test]
    fn should_confirm_wake_on_speech() {
        let mut machine = Machine::new(0);
        assert_eq!(
            machine.on_event(Event::SpeechStarted, 100),
            Some(Effect::Enter(State::ConfirmWake))
        );
    }

    #[test]
    fn should_transcribe_the_wake_clip_when_it_ends() {
        let mut machine = machine_in(State::ConfirmWake);
        assert_eq!(
            machine.on_event(Event::UtteranceReady, 800),
            Some(Effect::Transcribe)
        );
    }

    #[test]
    fn should_cap_the_wake_clip() {
        let mut machine = machine_in(State::ConfirmWake);
        assert_eq!(machine.on_event(Event::Tick, 2_999), None);
        assert_eq!(
            machine.on_event(Event::Tick, 3_000),
            Some(Effect::Transcribe)
        );
    }

    #[test]
    fn should_request_wake_transcription_once() {
        let mut machine = machine_in(State::ConfirmWake);
        machine.on_event(Event::UtteranceReady, 800);
        assert_eq!(machine.on_event(Event::Tick, 5_000), None);
        assert_eq!(machine.on_event(Event::UtteranceReady, 5_000), None);
    }

    #[test]
    fn should_await_a_command_only_after_the_wake_word() {
        for (transcript, next) in [("Doufu.", State::AwaitCommand), ("hello", State::AwaitWake)] {
            let mut machine = machine_in(State::ConfirmWake);
            machine.on_event(Event::UtteranceReady, 500);
            assert_eq!(
                machine.on_event(Event::Transcript(transcript), 1_000),
                Some(Effect::Enter(next)),
                "{transcript:?}"
            );
        }
    }

    #[test]
    fn should_go_back_when_wake_speech_is_dropped() {
        let mut machine = machine_in(State::ConfirmWake);
        assert_eq!(
            machine.on_event(Event::SpeechDropped, 900),
            Some(Effect::Enter(State::AwaitWake))
        );
    }

    #[test]
    fn should_stop_awaiting_a_command_after_10_seconds() {
        let mut machine = machine_in(State::AwaitCommand);
        assert_eq!(machine.on_event(Event::Tick, 10_999), None);
        assert_eq!(
            machine.on_event(Event::Tick, 11_000),
            Some(Effect::Enter(State::AwaitWake))
        );
    }

    #[test]
    fn should_count_the_command_wait_from_the_capture_start() {
        let mut machine = machine_in(State::AwaitCommand);
        assert_eq!(machine.on_event(Event::CaptureStarted, 1_300), None);
        assert_eq!(machine.on_event(Event::Tick, 11_299), None);
        assert_eq!(
            machine.on_event(Event::Tick, 11_300),
            Some(Effect::Enter(State::AwaitWake))
        );
    }

    #[test]
    fn should_keep_listening_once_a_command_starts() {
        let mut machine = machine_in(State::AwaitCommand);
        assert_eq!(machine.on_event(Event::SpeechStarted, 10_000), None);
        assert_eq!(machine.on_event(Event::Tick, 12_000), None);
    }

    #[test]
    fn should_handle_the_command_when_it_ends() {
        let mut machine = machine_in(State::AwaitCommand);
        machine.on_event(Event::SpeechStarted, 2_000);
        assert_eq!(
            machine.on_event(Event::UtteranceReady, 4_000),
            Some(Effect::Enter(State::HandleCommand))
        );
    }

    #[test]
    fn should_cap_the_command_at_30_seconds() {
        let mut machine = machine_in(State::AwaitCommand);
        machine.on_event(Event::SpeechStarted, 2_000);
        assert_eq!(machine.on_event(Event::Tick, 31_999), None);
        assert_eq!(
            machine.on_event(Event::Tick, 32_000),
            Some(Effect::Enter(State::HandleCommand))
        );
    }

    #[test]
    fn should_keep_the_wait_deadline_on_a_capture_start_after_speech() {
        let mut machine = machine_in(State::AwaitCommand);
        machine.on_event(Event::SpeechStarted, 5_000);
        assert_eq!(machine.on_event(Event::CaptureStarted, 5_500), None);
        machine.on_event(Event::SpeechDropped, 6_000);
        assert_eq!(
            machine.on_event(Event::Tick, 11_000),
            Some(Effect::Enter(State::AwaitWake))
        );
    }

    #[test]
    fn should_keep_the_wait_deadline_after_dropped_speech() {
        let mut machine = machine_in(State::AwaitCommand);
        machine.on_event(Event::SpeechStarted, 5_000);
        assert_eq!(machine.on_event(Event::SpeechDropped, 6_000), None);
        assert_eq!(machine.on_event(Event::Tick, 10_999), None);
        assert_eq!(
            machine.on_event(Event::Tick, 11_000),
            Some(Effect::Enter(State::AwaitWake))
        );
    }

    #[test]
    fn should_await_wake_after_handling_a_command() {
        for transcript in ["turn on the lamp", ""] {
            let mut machine = machine_in(State::HandleCommand);
            assert_eq!(
                machine.on_event(Event::Transcript(transcript), 3_000),
                Some(Effect::Enter(State::AwaitWake)),
                "{transcript:?}"
            );
        }
    }

    #[test]
    fn should_confirm_wake_again_after_a_rejected_wake_word() {
        let mut machine = machine_in(State::ConfirmWake);
        machine.on_event(Event::UtteranceReady, 500);
        machine.on_event(Event::Transcript("hello"), 1_000);
        assert_eq!(machine.state(), State::AwaitWake);
        machine.on_event(Event::SpeechStarted, 2_000);
        assert_eq!(
            machine.on_event(Event::UtteranceReady, 2_500),
            Some(Effect::Transcribe)
        );
    }

    #[test]
    fn should_confirm_wake_again_after_handling_a_command() {
        let mut machine = machine_in(State::HandleCommand);
        machine.on_event(Event::Transcript("turn on the lamp"), 3_000);
        assert_eq!(machine.state(), State::AwaitWake);
        machine.on_event(Event::SpeechStarted, 4_000);
        assert_eq!(
            machine.on_event(Event::UtteranceReady, 4_500),
            Some(Effect::Transcribe)
        );
    }

    #[test]
    fn should_await_wake_after_a_capture_failure() {
        for state in ALL {
            let mut machine = machine_in(state);
            assert_eq!(
                machine.on_event(Event::CaptureFailed, 3_000),
                Some(Effect::Enter(State::AwaitWake)),
                "{state:?}"
            );
            assert_eq!(machine.state(), State::AwaitWake);
        }
    }

    #[test]
    fn should_ignore_events_that_do_not_apply() {
        let cases = [
            (State::AwaitWake, Event::Transcript("doufu")),
            (State::AwaitWake, Event::Tick),
            (State::AwaitWake, Event::UtteranceReady),
            (State::AwaitWake, Event::SpeechDropped),
            (State::AwaitWake, Event::CaptureStarted),
            (State::ConfirmWake, Event::CaptureStarted),
            (State::ConfirmWake, Event::Transcript("doufu")),
            (State::AwaitCommand, Event::Transcript("doufu")),
            (State::AwaitCommand, Event::UtteranceReady),
            (State::AwaitCommand, Event::SpeechDropped),
            (State::HandleCommand, Event::SpeechStarted),
            (State::HandleCommand, Event::Tick),
        ];
        for (state, event) in cases {
            let mut machine = machine_in(state);
            assert_eq!(machine.on_event(event, 60_000), None, "{state:?} {event:?}");
            assert_eq!(machine.state(), state, "{event:?}");
        }
    }
}
