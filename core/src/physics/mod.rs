//! Physical models of outdoor sound propagation. Init-time / control-rate code in f64; the
//! audio path only sees their results through `shaping` filters, delays and gains.

pub mod air;
pub mod diffraction;
pub mod directivity;
pub mod ground;
pub mod scattering;
