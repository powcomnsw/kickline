//! Lumen core: frame buffers, generators, effects, blending and the
//! composition (layers x clips) that is rendered every tick.

pub mod audio;
pub mod blend;
pub mod color;
pub mod composition;
pub mod effect;
pub mod frame;
pub mod generator;

pub use audio::{AudioBand, AudioBands, AudioLink};
pub use blend::BlendMode;
pub use composition::{Clip, Composition, Layer};
pub use effect::Effect;
pub use frame::{Frame, Rgba};
pub use generator::Generator;
