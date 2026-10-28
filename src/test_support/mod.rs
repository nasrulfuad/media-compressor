pub mod font_fixture;
pub mod image_fixture;
pub mod pdf_fixture;

pub use font_fixture::{fixture_glyph_outline, glyph_count, glyph_outline, truetype_font};
pub use image_fixture::{encode_as, gradient_rgb, gradient_rgb16, gradient_rgba_with_transparency};
pub use pdf_fixture::{
    assert_no_dangling_references, decoded_image_pixels, decoded_page_content, FixtureImage,
    PdfFixture, ICC_LOOKING_PROFILE, SAMPLED_FUNCTION_SAMPLES,
};
