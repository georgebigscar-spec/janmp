//! Audio pipeline: decoders → channel mapping / resampling → ring buffer → output device.

pub mod decoder;
pub mod engine;
pub mod ffmpeg;
pub mod formats;
pub mod midi;
mod output;
mod resample;
pub mod stream;
pub mod vis;

pub use engine::{PlaybackState, Player, PlayerEvent};
