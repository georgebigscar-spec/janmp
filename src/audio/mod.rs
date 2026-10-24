//! Audio pipeline: decoders → channel mapping / resampling → ring buffer → output device.

pub mod decoder;
pub mod engine;
pub mod formats;
mod output;
mod resample;
pub mod vis;

pub use engine::{PlaybackState, Player, PlayerEvent};
