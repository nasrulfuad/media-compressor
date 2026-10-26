#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod app;
pub mod cli;
pub mod codec;
pub mod detection;
pub mod domain;
pub mod error;
pub mod fs;
pub mod pdf;
pub mod presentation;
pub mod raster;

#[cfg(test)]
pub(crate) mod test_support;
