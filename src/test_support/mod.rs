pub mod image_fixture;
pub mod pdf_fixture;

pub use image_fixture::{encode_as, gradient_rgb, gradient_rgb16, gradient_rgba_with_transparency};
pub use pdf_fixture::{
    assert_no_dangling_references, decoded_page_content, FixtureImage, PdfFixture,
    ICC_LOOKING_PROFILE, SAMPLED_FUNCTION_SAMPLES,
};
