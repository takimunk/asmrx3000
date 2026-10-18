//! Sound generators. Each produces a mono signal; the engine places it in space.

mod bird;
mod water;
mod wind;

pub use bird::Bird;
pub use water::BubbleStream;
pub use wind::WindSource;
