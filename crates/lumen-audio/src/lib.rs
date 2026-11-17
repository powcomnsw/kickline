//! Audio analysis for lighting: splits the music into frequency bands and
//! detects kick / snare / hi-hat hits using a filter bank (no FFT), so the
//! response is immediate and follows the same low/mid/high split DJ
//! software uses for its coloured waveforms.

pub mod analyzer;
pub mod demo;
pub mod filter;
pub mod input;
pub mod mac_tap;

pub use analyzer::{Analyzer, KickResponse, Settings, Wake};
pub use demo::DemoBeat;
pub use input::{default_input_name, host_name, list_devices, AudioFeed, AudioInput, DeviceInfo, FeedKind, StreamHealth};
pub use mac_tap::AppInfo;
