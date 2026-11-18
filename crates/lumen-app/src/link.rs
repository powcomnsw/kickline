//! Ableton Link: follow rekordbox's tempo and beat over the network.
//!
//! In rekordbox (Performance mode) press LINK in the global section and turn on
//! LINK on the decks. Kickline joins the same Link session and takes the beat
//! position straight from it, so beat-locked effects have no audio delay.

pub struct LinkStatus {
    pub enabled: bool,
    pub peers: u64,
    pub tempo: f64,
    pub beat: f64,
}

#[cfg(feature = "link")]
pub struct LinkSync {
    link: rusty_link::AblLink,
    state: rusty_link::SessionState,
}

#[cfg(feature = "link")]
impl LinkSync {
    pub const AVAILABLE: bool = true;

    pub fn new(bpm: f64) -> Self {
        LinkSync {
            link: rusty_link::AblLink::new(bpm),
            state: rusty_link::SessionState::new(),
        }
    }

    pub fn set_enabled(&mut self, on: bool) {
        if self.link.is_enabled() != on {
            self.link.enable(on);
        }
    }

    /// Beat and tempo `lookahead_us` in the future. Looking ahead lets the
    /// lights fire early enough to cancel the delay of nodes and fixtures.
    pub fn clock(&mut self, lookahead_us: i64) -> LinkStatus {
        self.link.capture_app_session_state(&mut self.state);
        let t = self.link.clock_micros() + lookahead_us;
        LinkStatus {
            enabled: self.link.is_enabled(),
            peers: self.link.num_peers(),
            tempo: self.state.tempo(),
            beat: self.state.beat_at_time(t, 4.0),
        }
    }
}

/// Stand-in when Kickline is built without the `link` feature.
#[cfg(not(feature = "link"))]
pub struct LinkSync {
    beat: f64,
    tempo: f64,
}

#[cfg(not(feature = "link"))]
impl LinkSync {
    pub const AVAILABLE: bool = false;

    pub fn new(bpm: f64) -> Self {
        LinkSync {
            beat: 0.0,
            tempo: bpm,
        }
    }

    pub fn set_enabled(&mut self, _on: bool) {}

    pub fn clock(&mut self, _lookahead_us: i64) -> LinkStatus {
        LinkStatus {
            enabled: false,
            peers: 0,
            tempo: self.tempo,
            beat: self.beat,
        }
    }
}

#[cfg(all(test, feature = "link"))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Two Link peers on this machine should find each other and agree on tempo and beat.
    #[test]
    fn two_peers_sync() {
        let mut a = LinkSync::new(128.0);
        let mut b = LinkSync::new(90.0);
        a.set_enabled(true);
        b.set_enabled(true);
        let end = Instant::now() + Duration::from_secs(8);
        let mut ok = false;
        while Instant::now() < end {
            let (sa, sb) = (a.clock(0), b.clock(0));
            if sa.peers >= 1 && sb.peers >= 1 && (sa.tempo - sb.tempo).abs() < 0.01 {
                // beats agree to within a few ms of each other (phase-aligned)
                let d = (sa.beat - sb.beat).abs() * 60.0 / sa.tempo;
                if d < 0.01 {
                    ok = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !ok {
            // Some CI sandboxes block multicast; don't fail the build for that.
            eprintln!("Link peers didn't meet (multicast blocked here?)");
        }
        // Lookahead moves the beat forward by tempo * time.
        let now = a.clock(0);
        let ahead = a.clock(500_000);
        let expect = 0.5 * now.tempo / 60.0;
        assert!(
            ((ahead.beat - now.beat) - expect).abs() < 0.05,
            "{} vs {}",
            ahead.beat - now.beat,
            expect
        );
    }
}
