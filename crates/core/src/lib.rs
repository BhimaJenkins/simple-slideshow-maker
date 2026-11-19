//! Core of Simple Slideshow Maker: the project model, auto-fit timing, the
//! frame renderer and the exporters. No UI code lives here, so it can be
//! reused by the desktop and mobile front ends (and tested headlessly).

pub mod export;
pub mod media;
pub mod model;
pub mod render;
pub mod text;
pub mod timing;

pub use model::*;
