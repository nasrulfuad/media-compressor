pub mod cleanup;
pub mod dedup;
pub mod fonts;
pub mod images;
pub mod inline_images;
pub mod merge_content;
pub mod metadata;
pub mod streams;

pub const MIN_COMPRESSIBLE_SIZE: usize = 64;
