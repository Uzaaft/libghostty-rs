//! Handling various protocols pioneered by Kitty,
//! including the [Kitty graphics protocol](graphics).

#[cfg(all(feature = "std", feature = "kitty-graphics"))]
pub mod graphics;

#[cfg(all(feature = "std", feature = "kitty-graphics"))]
pub use graphics::Graphics;
