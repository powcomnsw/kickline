//! Kickline core: frame buffers, generators, effects, blending and the
//! composition (layers x clips) that is rendered every tick.

pub mod audio;
pub mod blend;
pub mod color;
pub mod composition;
pub mod effect;
pub mod frame;
pub mod generator;
pub mod palette;
pub mod director;
pub mod scenes;
pub mod sections;
pub mod strip;

pub use audio::{AudioBand, AudioBands, AudioLink};
pub use blend::BlendMode;
pub use composition::{Clip, Composition, Layer};
pub use effect::Effect;
pub use frame::{Frame, Rgba};
pub use generator::Generator;
pub use palette::{Paint, Palette, Palettes};
pub use strip::{StripClip, StripEffect, StripPixel};
pub use composition::{StripPass, StripStage};
pub use sections::{LayerLook, Looks, Section, SectionDetector, SectionTuning};
pub use director::{Director, DirectorSettings, Genre};
