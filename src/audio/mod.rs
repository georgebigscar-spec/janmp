//! Audio pipeline: decoders → channel mapping / resampling → ring buffer → output device.

mod ape;
pub mod cdda;
pub mod decoder;
pub mod engine;
pub mod eq;
pub mod ffmpeg;
pub mod formats;
pub mod midi;
pub mod replaygain;
mod output;
mod resample;
pub mod stream;
pub mod vis;
pub mod ytdl;

pub use engine::{PlaybackState, Player, PlayerEvent};
