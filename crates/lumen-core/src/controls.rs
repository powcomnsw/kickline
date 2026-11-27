//! Hardware and remote control: what a MIDI knob, a pad or an OSC message
//! does. Pure mapping and learn; the ports themselves live in the app.

use serde::{Deserialize, Serialize};

/// Something on the Play screen a controller can drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Master,
    Left,
    Centre,
    Right,
    Crossfade,
    Blackout,
    /// Hold to build, release to drop.
    Drop,
    Tap,
    Auto,
    /// Play a column (0-based).
    Column(u8),
}

impl Target {
    /// Everything that can be bound, in the order the list shows it.
    pub fn all(columns: usize) -> Vec<Target> {
        let mut v = vec![Target::Master, Target::Left, Target::Centre, Target::Right, Target::Crossfade, Target::Blackout, Target::Drop, Target::Tap, Target::Auto];
        v.extend((0..columns.min(16)).map(|c| Target::Column(c as u8)));
        v
    }

    /// A fader or knob (a value) rather than a button.
    pub fn is_continuous(self) -> bool {
        matches!(self, Target::Master | Target::Left | Target::Centre | Target::Right | Target::Crossfade)
    }

    pub fn name(self) -> String {
        match self {
            Target::Master => "Master".into(),
            Target::Left => "Left fader".into(),
            Target::Centre => "Centre fader".into(),
            Target::Right => "Right fader".into(),
            Target::Crossfade => "Crossfader A↔B".into(),
            Target::Blackout => "Blackout".into(),
            Target::Drop => "Hold to drop".into(),
            Target::Tap => "Tap tempo".into(),
            Target::Auto => "Auto on/off".into(),
            Target::Column(c) => format!("Play column {}", c + 1),
        }
    }

    /// The OSC address Kickline answers on without any setup.
    pub fn osc_address(self) -> String {
        match self {
            Target::Master => "/kickline/master".into(),
            Target::Left => "/kickline/left".into(),
            Target::Centre => "/kickline/centre".into(),
            Target::Right => "/kickline/right".into(),
            Target::Crossfade => "/kickline/crossfade".into(),
            Target::Blackout => "/kickline/blackout".into(),
            Target::Drop => "/kickline/drop".into(),
            Target::Tap => "/kickline/tap".into(),
            Target::Auto => "/kickline/auto".into(),
            Target::Column(c) => format!("/kickline/column/{}", c + 1),
        }
    }
}

/// Where a control comes from. MIDI channels and numbers are 0-based.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    Cc { channel: u8, number: u8 },
    Note { channel: u8, number: u8 },
    Osc(String),
}

impl Source {
    pub fn describe(&self) -> String {
        match self {
            Source::Cc { channel, number } => format!("MIDI knob/fader {number}, channel {}", channel + 1),
            Source::Note { channel, number } => format!("MIDI pad/key {number}, channel {}", channel + 1),
            Source::Osc(a) => format!("OSC {a}"),
        }
    }
}

/// One thing that arrived.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Cc { channel: u8, number: u8, value: u8 },
    NoteOn { channel: u8, number: u8, velocity: u8 },
    NoteOff { channel: u8, number: u8 },
    /// An OSC message with its first argument as a number (a bool or an int
    /// becomes 0 or 1; no argument is a press).
    Osc { address: String, value: f32 },
}

impl Message {
    /// Read a raw MIDI message (status byte first).
    pub fn from_midi(bytes: &[u8]) -> Option<Message> {
        let (&status, rest) = bytes.split_first()?;
        let channel = status & 0x0F;
        match (status & 0xF0, rest) {
            (0xB0, [n, v, ..]) => Some(Message::Cc { channel, number: *n, value: *v }),
            (0x90, [n, 0, ..]) => Some(Message::NoteOff { channel, number: *n }),
            (0x90, [n, v, ..]) => Some(Message::NoteOn { channel, number: *n, velocity: *v }),
            (0x80, [n, ..]) => Some(Message::NoteOff { channel, number: *n }),
            _ => None,
        }
    }

    fn source(&self) -> Source {
        match self {
            Message::Cc { channel, number, .. } => Source::Cc { channel: *channel, number: *number },
            Message::NoteOn { channel, number, .. } | Message::NoteOff { channel, number } => Source::Note { channel: *channel, number: *number },
            Message::Osc { address, .. } => Source::Osc(address.clone()),
        }
    }

    /// 0..1: how far a knob is turned, how hard a pad is hit, or off.
    fn level(&self) -> f32 {
        match self {
            Message::Cc { value, .. } => *value as f32 / 127.0,
            Message::NoteOn { velocity, .. } => *velocity as f32 / 127.0,
            Message::NoteOff { .. } => 0.0,
            Message::Osc { value, .. } => value.clamp(0.0, 1.0),
        }
    }
}

/// What a message asks the show to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// A fader or knob moved to this value (0..1).
    Set(Target, f32),
    /// A button went down.
    Press(Target),
    /// A button came up.
    Release(Target),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub source: Source,
    pub target: Target,
}

/// The controller setup that is saved with the show.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ControlMap {
    pub bindings: Vec<Binding>,
    /// The MIDI input to listen to (by name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi_port: Option<String>,
    /// UDP port to receive OSC on (off when none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub osc_in: Option<u16>,
    /// Where to send the faders' state to, as "host:port" (off when none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub osc_out: Option<String>,
    /// The next control that arrives gets bound to this.
    #[serde(skip)]
    pub learn: Option<Target>,
}

impl ControlMap {
    pub fn source_of(&self, t: Target) -> Option<&Source> {
        self.bindings.iter().find(|b| b.target == t).map(|b| &b.source)
    }

    pub fn clear(&mut self, t: Target) {
        self.bindings.retain(|b| b.target != t);
    }

    /// Bind a source to a target, replacing the target's old control and
    /// anything else that used the same control.
    pub fn bind(&mut self, source: Source, target: Target) {
        self.bindings.retain(|b| b.target != target && b.source != source);
        self.bindings.push(Binding { source, target });
    }

    /// Handle what arrived: in learn mode it makes a binding (a knob has to
    /// move, a pad has to go down), otherwise it is turned into events.
    pub fn handle(&mut self, m: &Message) -> Vec<Event> {
        if let Some(t) = self.learn {
            let counts = !matches!(m, Message::NoteOff { .. }) && !matches!(m, Message::Osc { value, .. } if *value == 0.0 && !t.is_continuous());
            if counts {
                self.bind(m.source(), t);
                self.learn = None;
            }
            return Vec::new();
        }
        let src = m.source();
        let level = m.level();
        let mut out = Vec::new();
        let mut act = |t: Target| {
            if t.is_continuous() {
                out.push(Event::Set(t, level));
            } else if level > 0.0 {
                out.push(Event::Press(t));
            } else {
                out.push(Event::Release(t));
            }
        };
        let mut hit = false;
        for b in self.bindings.iter().filter(|b| b.source == src) {
            act(b.target);
            hit = true;
        }
        // OSC answers its own addresses with no setup.
        if !hit {
            if let Message::Osc { address, .. } = m {
                if let Some(t) = Target::all(16).into_iter().find(|t| t.osc_address() == *address) {
                    act(t);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc(n: u8, v: u8) -> Message {
        Message::Cc { channel: 0, number: n, value: v }
    }

    #[test]
    fn midi_bytes_become_messages() {
        assert_eq!(Message::from_midi(&[0xB1, 7, 100]), Some(Message::Cc { channel: 1, number: 7, value: 100 }));
        assert_eq!(Message::from_midi(&[0x90, 36, 90]), Some(Message::NoteOn { channel: 0, number: 36, velocity: 90 }));
        assert_eq!(Message::from_midi(&[0x90, 36, 0]), Some(Message::NoteOff { channel: 0, number: 36 }));
        assert_eq!(Message::from_midi(&[0x80, 36, 64]), Some(Message::NoteOff { channel: 0, number: 36 }));
        assert_eq!(Message::from_midi(&[0xF8]), None, "clock ticks are ignored");
        assert_eq!(Message::from_midi(&[]), None);
    }

    #[test]
    fn learn_binds_the_next_control_to_the_chosen_target() {
        let mut m = ControlMap::default();
        m.learn = Some(Target::Master);
        assert!(m.handle(&cc(7, 64)).is_empty(), "the learning message does nothing else");
        assert_eq!(m.learn, None);
        assert_eq!(m.source_of(Target::Master), Some(&Source::Cc { channel: 0, number: 7 }));
        let ev = m.handle(&cc(7, 127));
        assert_eq!(ev, vec![Event::Set(Target::Master, 1.0)]);
    }

    #[test]
    fn a_pad_has_to_go_down_to_learn_and_up_releases() {
        let mut m = ControlMap::default();
        m.learn = Some(Target::Drop);
        m.handle(&Message::NoteOff { channel: 0, number: 40 });
        assert_eq!(m.learn, Some(Target::Drop), "a note off is not a press");
        m.handle(&Message::NoteOn { channel: 0, number: 40, velocity: 100 });
        assert_eq!(m.learn, None);
        assert_eq!(m.handle(&Message::NoteOn { channel: 0, number: 40, velocity: 100 }), vec![Event::Press(Target::Drop)]);
        assert_eq!(m.handle(&Message::NoteOff { channel: 0, number: 40 }), vec![Event::Release(Target::Drop)]);
    }

    #[test]
    fn binding_again_replaces_the_old_control_and_clear_removes_it() {
        let mut m = ControlMap::default();
        m.bind(Source::Cc { channel: 0, number: 1 }, Target::Left);
        m.bind(Source::Cc { channel: 0, number: 2 }, Target::Left);
        assert_eq!(m.bindings.len(), 1);
        m.bind(Source::Cc { channel: 0, number: 2 }, Target::Right);
        assert_eq!(m.bindings.len(), 1, "a control drives one thing");
        assert_eq!(m.source_of(Target::Right), Some(&Source::Cc { channel: 0, number: 2 }));
        m.clear(Target::Right);
        assert!(m.bindings.is_empty());
    }

    #[test]
    fn unbound_controls_do_nothing_and_osc_works_with_no_setup() {
        let mut m = ControlMap::default();
        assert!(m.handle(&cc(9, 9)).is_empty());
        let e = m.handle(&Message::Osc { address: "/kickline/centre".into(), value: 0.25 });
        assert_eq!(e, vec![Event::Set(Target::Centre, 0.25)]);
        let e = m.handle(&Message::Osc { address: "/kickline/column/3".into(), value: 1.0 });
        assert_eq!(e, vec![Event::Press(Target::Column(2))]);
        assert!(m.handle(&Message::Osc { address: "/other".into(), value: 1.0 }).is_empty());
    }

    #[test]
    fn the_setup_is_saved_without_the_learn_state_and_old_shows_open() {
        let mut m = ControlMap::default();
        m.bind(Source::Osc("/fader1".into()), Target::Master);
        m.midi_port = Some("APC mini".into());
        m.learn = Some(Target::Left);
        let json = serde_json::to_string(&m).unwrap();
        let back: ControlMap = serde_json::from_str(&json).unwrap();
        assert_eq!(back.learn, None);
        assert_eq!(back.bindings, m.bindings);
        assert_eq!(back.midi_port.as_deref(), Some("APC mini"));
        let old: ControlMap = serde_json::from_str("{\"bindings\":[]}").unwrap();
        assert_eq!(old, ControlMap::default());
    }
}
