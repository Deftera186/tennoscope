//! Windows desktop duplication, isolated from the application's unsafe-free capture policy.
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(any(windows, test))]
#[allow(unsafe_code)]
mod mapped;
#[cfg(any(windows, test))]
mod pixels;
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

#[cfg(windows)]
pub use windows::{Capture, CaptureError};

/// A window in physical desktop coordinates, including negative monitor origins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// An upright monitor image and its physical desktop geometry.
pub struct CapturedFrame {
    pub image: image::RgbaImage,
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: u32,
    pub height: u32,
}
