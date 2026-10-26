use std::collections::BTreeSet;
use std::io::Write;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use image::ImageFormat;
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};

use super::image_fixture::{encode_as, gradient_rgb};

const DEFAULT_MEDIA_BOX: [i64; 4] = [0, 0, 612, 792];
const PAGE_RESOURCES_KEY: &[u8] = b"Resources";
const FIXTURE_FONT_NAME: &str = "F1";
pub const ICC_LOOKING_PROFILE: &[u8] =
    b"q   1 0 0 1 0 0   cm   BT   ET   Q   %   icc-looking-profile   q   1 0 0 1 0 0   cm   BT   ET   Q  ";
pub const SAMPLED_FUNCTION_SAMPLES: &[u8] =
    b"q   %   sampled   Q    \x00\x10\x20\x30   q   %   sampled   Q    \x00\x10\x20\x30   q   Q   ";

#[derive(Debug, Clone)]
pub enum FixtureImage {
    Jpeg { width: u32, height: u32 },
    FlateRgb { width: u32, height: u32 },
    FlateRgbWithPngPredictor { width: u32, height: u32 },
    FilterChain { filters: Vec<&'static str> },
}

pub struct PdfFixture {
    document: Document,
    catalog_id: ObjectId,
    pages_id: ObjectId,
    page_ids: Vec<ObjectId>,
    next_image_index: u32,
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).expect("fixture deflate write");
    encoder.finish().expect("fixture deflate finish")
}

fn flate_stream(mut dict: Dictionary, raw: &[u8]) -> Stream {
    dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
    Stream::new(dict, deflate(raw))
}

fn media_box_object() -> Object {
    Object::Array(
        DEFAULT_MEDIA_BOX
            .iter()
            .map(|v| Object::Integer(*v))
            .collect(),
    )
}

fn gradient_rgb_bytes(width: u32, height: u32) -> Vec<u8> {
    gradient_rgb(width, height).to_rgb8().into_raw()
}

fn predictor_encoded_rows(width: u32, height: u32) -> Vec<u8> {
    let row_bytes = (width * 3) as usize;
    let raw = gradient_rgb_bytes(width, height);
    let mut encoded = Vec::with_capacity(raw.len() + height as usize);
    for row in raw.chunks(row_bytes) {
        encoded.push(0);
        encoded.extend_from_slice(row);
    }
    encoded
}

fn rgb_image_dict(width: u32, height: u32) -> Dictionary {
    dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => i64::from(width),
        "Height" => i64::from(height),
        "ColorSpace" => "DeviceRGB",
        "BitsPerComponent" => 8,
    }
}

impl FixtureImage {
    fn into_stream(self) -> Stream {
        match self {
            Self::Jpeg { width, height } => {
                let jpeg = encode_as(&gradient_rgb(width, height), ImageFormat::Jpeg);
                let mut dict = rgb_image_dict(width, height);
                dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
                Stream::new(dict, jpeg)
            }
            Self::FlateRgb { width, height } => flate_stream(
                rgb_image_dict(width, height),
                &gradient_rgb_bytes(width, height),
            ),
            Self::FlateRgbWithPngPredictor { width, height } => {
                let mut dict = rgb_image_dict(width, height);
                dict.set(
                    "DecodeParms",
                    Object::Dictionary(dictionary! {
                        "Predictor" => 15,
                        "Colors" => 3,
                        "Columns" => i64::from(width),
                        "BitsPerComponent" => 8,
                    }),
                );
                flate_stream(dict, &predictor_encoded_rows(width, height))
            }
            Self::FilterChain { filters } => {
                let mut dict = rgb_image_dict(4, 4);
                let names = filters
                    .iter()
                    .map(|name| Object::Name(name.as_bytes().to_vec()))
                    .collect();
                dict.set("Filter", Object::Array(names));
                Stream::new(dict, b"opaque-encoded-bytes".to_vec())
            }
        }
    }
}

impl Default for PdfFixture {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfFixture {
    pub fn new() -> Self {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => Vec::<Object>::new(),
                "Count" => 0,
            }),
        );
        document.trailer.set("Root", catalog_id);
        Self {
            document,
            catalog_id,
            pages_id,
            page_ids: Vec::new(),
            next_image_index: 1,
        }
    }

    pub fn page_with_content(self, content: &[u8]) -> Self {
        self.page_with_content_streams(&[content])
    }

    pub fn page_with_content_streams(mut self, contents: &[&[u8]]) -> Self {
        let content_ids: Vec<Object> = contents
            .iter()
            .map(|content| {
                let stream = Stream::new(Dictionary::new(), content.to_vec());
                Object::Reference(self.document.add_object(stream))
            })
            .collect();
        let contents_object = match content_ids.as_slice() {
            [single] => single.clone(),
            _ => Object::Array(content_ids),
        };
        let page_id = self.document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => self.pages_id,
            "MediaBox" => media_box_object(),
            "Contents" => contents_object,
        });
        self.page_ids.push(page_id);
        self.refresh_kids();
        self
    }

    fn refresh_kids(&mut self) {
        let kids: Vec<Object> = self
            .page_ids
            .iter()
            .map(|id| Object::Reference(*id))
            .collect();
        let count = kids.len() as i64;
        if let Some(Object::Dictionary(pages)) = self.document.objects.get_mut(&self.pages_id) {
            pages.set("Kids", Object::Array(kids));
            pages.set("Count", Object::Integer(count));
        }
    }

    fn first_page_id(&self) -> ObjectId {
        *self
            .page_ids
            .first()
            .expect("fixture needs a page before this builder method")
    }

    pub fn with_flate_content(mut self) -> Self {
        let content_ids: Vec<ObjectId> = self
            .page_ids
            .iter()
            .flat_map(|page_id| self.document.get_page_contents(*page_id))
            .collect();
        for id in content_ids {
            if let Some(Object::Stream(stream)) = self.document.objects.get_mut(&id) {
                let raw = std::mem::take(&mut stream.content);
                *stream = flate_stream(stream.dict.clone(), &raw);
            }
        }
        self
    }

    pub fn with_inherited_font_resource(mut self) -> Self {
        let font_id = self.document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources = dictionary! {
            "Font" => dictionary! { FIXTURE_FONT_NAME => font_id },
        };
        if let Some(Object::Dictionary(pages)) = self.document.objects.get_mut(&self.pages_id) {
            pages.set(PAGE_RESOURCES_KEY, Object::Dictionary(resources));
        }
        self
    }

    fn page_resources_mut(&mut self) -> &mut Dictionary {
        let page_id = self.first_page_id();
        let page = self
            .document
            .get_dictionary_mut(page_id)
            .expect("first page dictionary");
        if !page.has(PAGE_RESOURCES_KEY) {
            page.set(PAGE_RESOURCES_KEY, Object::Dictionary(Dictionary::new()));
        }
        match page.get_mut(PAGE_RESOURCES_KEY) {
            Ok(Object::Dictionary(resources)) => resources,
            _ => panic!("fixture page resources must be a direct dictionary"),
        }
    }

    fn register_page_resource(&mut self, category: &str, name: &str, value: Object) {
        let resources = self.page_resources_mut();
        if !resources.has(category.as_bytes()) {
            resources.set(category, Object::Dictionary(Dictionary::new()));
        }
        if let Ok(Object::Dictionary(entries)) = resources.get_mut(category.as_bytes()) {
            entries.set(name, value);
        }
    }

    pub fn with_image(mut self, image: FixtureImage) -> Self {
        let image_id = self.document.add_object(image.into_stream());
        let name = format!("Im{}", self.next_image_index);
        self.next_image_index += 1;
        self.register_page_resource("XObject", &name, Object::Reference(image_id));
        self
    }

    pub fn with_form_xobject(mut self, content: &[u8]) -> Self {
        let form = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Form",
                "BBox" => media_box_object(),
            },
            content.to_vec(),
        );
        let form_id = self.document.add_object(form);
        self.register_page_resource("XObject", "Fm1", Object::Reference(form_id));
        self
    }

    fn append_page_annotation(&mut self, annotation_id: ObjectId) {
        let page_id = self.first_page_id();
        let page = self
            .document
            .get_dictionary_mut(page_id)
            .expect("first page dictionary");
        if !page.has(b"Annots") {
            page.set("Annots", Object::Array(Vec::new()));
        }
        if let Ok(Object::Array(annotations)) = page.get_mut(b"Annots") {
            annotations.push(Object::Reference(annotation_id));
        }
    }

    pub fn with_annotation(mut self, subtype: &str) -> Self {
        let annotation_id = self.document.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => subtype,
            "Rect" => vec![Object::Integer(0), Object::Integer(0), Object::Integer(10), Object::Integer(10)],
        });
        self.append_page_annotation(annotation_id);
        self
    }

    fn add_form_widget(&mut self, field_type: &str) {
        let widget_id = self.document.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Widget",
            "FT" => field_type,
            "T" => Object::string_literal("field"),
            "Rect" => vec![Object::Integer(0), Object::Integer(0), Object::Integer(10), Object::Integer(10)],
        });
        self.append_page_annotation(widget_id);
        let catalog_id = self.catalog_id;
        let catalog = self
            .document
            .get_dictionary_mut(catalog_id)
            .expect("catalog dictionary");
        if !catalog.has(b"AcroForm") {
            catalog.set(
                "AcroForm",
                Object::Dictionary(dictionary! { "Fields" => Vec::<Object>::new() }),
            );
        }
        if let Ok(Object::Dictionary(form)) = catalog.get_mut(b"AcroForm") {
            if let Ok(Object::Array(fields)) = form.get_mut(b"Fields") {
                fields.push(Object::Reference(widget_id));
            }
        }
    }

    pub fn with_acroform_widget(mut self) -> Self {
        self.add_form_widget("Tx");
        self
    }

    pub fn with_signature_field(mut self) -> Self {
        self.add_form_widget("Sig");
        self
    }

    fn catalog_names_mut(&mut self) -> &mut Dictionary {
        let catalog_id = self.catalog_id;
        let catalog = self
            .document
            .get_dictionary_mut(catalog_id)
            .expect("catalog dictionary");
        if !catalog.has(b"Names") {
            catalog.set("Names", Object::Dictionary(Dictionary::new()));
        }
        match catalog.get_mut(b"Names") {
            Ok(Object::Dictionary(names)) => names,
            _ => panic!("fixture catalog Names must be a direct dictionary"),
        }
    }

    pub fn with_named_destination(mut self, name: &str) -> Self {
        let page_id = self.first_page_id();
        let destination = Object::Array(vec![
            Object::Reference(page_id),
            Object::Name(b"Fit".to_vec()),
        ]);
        let tree = dictionary! {
            "Names" => vec![Object::string_literal(name), destination],
        };
        self.catalog_names_mut()
            .set("Dests", Object::Dictionary(tree));
        self
    }

    pub fn with_output_intent(mut self) -> Self {
        let profile_id = self.document.add_object(Stream::new(
            dictionary! { "N" => 3 },
            b"fixture-icc-profile".to_vec(),
        ));
        let intent_id = self.document.add_object(dictionary! {
            "Type" => "OutputIntent",
            "S" => "GTS_PDFX",
            "OutputConditionIdentifier" => Object::string_literal("sRGB"),
            "DestOutputProfile" => profile_id,
        });
        let catalog_id = self.catalog_id;
        if let Ok(catalog) = self.document.get_dictionary_mut(catalog_id) {
            catalog.set(
                "OutputIntents",
                Object::Array(vec![Object::Reference(intent_id)]),
            );
        }
        self
    }

    pub fn with_struct_tree(mut self) -> Self {
        let root_id = self
            .document
            .add_object(dictionary! { "Type" => "StructTreeRoot" });
        let catalog_id = self.catalog_id;
        if let Ok(catalog) = self.document.get_dictionary_mut(catalog_id) {
            catalog.set("StructTreeRoot", Object::Reference(root_id));
            catalog.set(
                "MarkInfo",
                Object::Dictionary(dictionary! { "Marked" => true }),
            );
        }
        let page_id = self.first_page_id();
        if let Ok(page) = self.document.get_dictionary_mut(page_id) {
            page.set("StructParents", Object::Integer(0));
        }
        self
    }

    pub fn with_embedded_file(mut self) -> Self {
        let file_id = self.document.add_object(Stream::new(
            dictionary! { "Type" => "EmbeddedFile" },
            b"attachment-bytes".to_vec(),
        ));
        let spec_id = self.document.add_object(dictionary! {
            "Type" => "Filespec",
            "F" => Object::string_literal("a.txt"),
            "EF" => dictionary! { "F" => file_id },
        });
        let tree = dictionary! {
            "Names" => vec![Object::string_literal("a.txt"), Object::Reference(spec_id)],
        };
        self.catalog_names_mut()
            .set("EmbeddedFiles", Object::Dictionary(tree));
        self
    }

    pub fn with_thumbnail(mut self) -> Self {
        let thumbnail_id = self
            .document
            .add_object(Stream::new(rgb_image_dict(2, 2), vec![7u8; 12]));
        for page_id in self.page_ids.clone() {
            if let Ok(page) = self.document.get_dictionary_mut(page_id) {
                page.set("Thumb", Object::Reference(thumbnail_id));
            }
        }
        self
    }

    pub fn with_javascript_open_action(mut self) -> Self {
        let action_id = self.document.add_object(dictionary! {
            "Type" => "Action",
            "S" => "JavaScript",
            "JS" => Object::string_literal("app.alert(1)"),
        });
        let catalog_id = self.catalog_id;
        if let Ok(catalog) = self.document.get_dictionary_mut(catalog_id) {
            catalog.set("OpenAction", Object::Reference(action_id));
        }
        self
    }

    pub fn with_info_and_xmp_metadata(mut self) -> Self {
        let info_id = self.document.add_object(dictionary! {
            "Title" => Object::string_literal("Fixture"),
            "Author" => Object::string_literal("Tester"),
        });
        self.document.trailer.set("Info", info_id);
        let xmp_id = self.document.add_object(Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            b"<x:xmpmeta/>".to_vec(),
        ));
        let catalog_id = self.catalog_id;
        if let Ok(catalog) = self.document.get_dictionary_mut(catalog_id) {
            catalog.set("Metadata", Object::Reference(xmp_id));
        }
        self
    }

    pub fn with_icc_color_space_stream(mut self) -> Self {
        let profile_id = self.document.add_object(Stream::new(
            dictionary! { "N" => 3 },
            ICC_LOOKING_PROFILE.to_vec(),
        ));
        let color_space = Object::Array(vec![
            Object::Name(b"ICCBased".to_vec()),
            Object::Reference(profile_id),
        ]);
        self.register_page_resource("ColorSpace", "Cs1", color_space);
        self
    }

    pub fn with_sampled_function_stream(mut self) -> Self {
        let function_id = self.document.add_object(Stream::new(
            dictionary! {
                "FunctionType" => 0,
                "Domain" => vec![Object::Integer(0), Object::Integer(1)],
                "Range" => vec![Object::Integer(0), Object::Integer(1)],
                "Size" => vec![Object::Integer(4)],
                "BitsPerSample" => 8,
            },
            SAMPLED_FUNCTION_SAMPLES.to_vec(),
        ));
        let shading = Object::Dictionary(dictionary! {
            "ShadingType" => 2,
            "ColorSpace" => "DeviceGray",
            "Function" => function_id,
            "Coords" => vec![Object::Integer(0), Object::Integer(0), Object::Integer(1), Object::Integer(0)],
        });
        self.register_page_resource("Shading", "Sh1", shading);
        self
    }

    pub fn with_identity_cid_font(mut self, font_program: &[u8]) -> Self {
        let font_file = flate_stream(
            dictionary! { "Length1" => font_program.len() as i64 },
            font_program,
        );
        let font_file_id = self.document.add_object(font_file);
        let descriptor_id = self.document.add_object(dictionary! {
            "Type" => "FontDescriptor",
            "FontName" => "TestFont",
            "Flags" => 4,
            "FontBBox" => vec![Object::Integer(-200), Object::Integer(-300), Object::Integer(1200), Object::Integer(1000)],
            "ItalicAngle" => 0,
            "Ascent" => 900,
            "Descent" => -200,
            "CapHeight" => 700,
            "StemV" => 80,
            "FontFile2" => font_file_id,
        });
        let descendant_id = self.document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "CIDFontType2",
            "BaseFont" => "TestFont",
            "CIDToGIDMap" => "Identity",
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::string_literal("Adobe"),
                "Ordering" => Object::string_literal("Identity"),
                "Supplement" => 0,
            },
            "DW" => 1000,
            "FontDescriptor" => descriptor_id,
        });
        let type0_id = self.document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => "TestFont",
            "Encoding" => "Identity-H",
            "DescendantFonts" => vec![Object::Reference(descendant_id)],
        });
        self.register_page_resource("Font", FIXTURE_FONT_NAME, Object::Reference(type0_id));
        self
    }

    pub fn build(self) -> Document {
        self.document
    }

    pub fn into_bytes(self) -> Vec<u8> {
        let mut document = self.document;
        let mut bytes = Vec::new();
        document
            .save_to(&mut bytes)
            .expect("fixture document must serialize");
        bytes
    }
}

fn collect_references(object: &Object, found: &mut BTreeSet<ObjectId>) {
    match object {
        Object::Reference(id) => {
            found.insert(*id);
        }
        Object::Array(items) => items
            .iter()
            .for_each(|item| collect_references(item, found)),
        Object::Dictionary(dict) => dict
            .iter()
            .for_each(|(_, value)| collect_references(value, found)),
        Object::Stream(stream) => stream
            .dict
            .iter()
            .for_each(|(_, value)| collect_references(value, found)),
        _ => {}
    }
}

pub fn assert_no_dangling_references(document: &Document) {
    let mut referenced = BTreeSet::new();
    document
        .trailer
        .iter()
        .for_each(|(_, value)| collect_references(value, &mut referenced));
    document
        .objects
        .values()
        .for_each(|object| collect_references(object, &mut referenced));
    let missing: Vec<ObjectId> = referenced
        .into_iter()
        .filter(|id| !document.objects.contains_key(id))
        .collect();
    assert!(missing.is_empty(), "dangling references: {missing:?}");
}

fn paeth_predictor(left: u8, up: u8, up_left: u8) -> u8 {
    let (a, b, c) = (i32::from(left), i32::from(up), i32::from(up_left));
    let estimate = a + b - c;
    let (distance_a, distance_b, distance_c) = (
        (estimate - a).abs(),
        (estimate - b).abs(),
        (estimate - c).abs(),
    );
    if distance_a <= distance_b && distance_a <= distance_c {
        left
    } else if distance_b <= distance_c {
        up
    } else {
        up_left
    }
}

fn unfilter_png_rows(filtered: &[u8], columns: usize, colors: usize) -> Vec<u8> {
    let row_bytes = columns * colors;
    let mut output: Vec<u8> = Vec::with_capacity(filtered.len());
    for (row_index, row) in filtered.chunks(row_bytes + 1).enumerate() {
        let (filter, data) = (row[0], &row[1..]);
        let previous_start = output.len().saturating_sub(row_bytes);
        for (index, byte) in data.iter().enumerate() {
            let left = if index >= colors {
                output[output.len() - colors]
            } else {
                0
            };
            let up = if row_index > 0 {
                output[previous_start + index]
            } else {
                0
            };
            let up_left = if row_index > 0 && index >= colors {
                output[previous_start + index - colors]
            } else {
                0
            };
            let predicted = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                _ => paeth_predictor(left, up, up_left),
            };
            output.push(byte.wrapping_add(predicted));
        }
    }
    output
}

pub fn decoded_image_pixels(stream: &Stream) -> Vec<u8> {
    let inflated = crate::codec::zlib::inflate_bounded(&stream.content, usize::MAX)
        .expect("image stream must inflate");
    let parms = match stream.dict.get(b"DecodeParms") {
        Ok(Object::Dictionary(parms)) => parms,
        _ => return inflated,
    };
    let number = |key: &[u8]| {
        parms
            .get(key)
            .ok()
            .and_then(|v| v.as_i64().ok())
            .unwrap_or(1)
    };
    unfilter_png_rows(
        &inflated,
        number(b"Columns") as usize,
        number(b"Colors") as usize,
    )
}

pub fn decoded_page_content(document: &Document, page_number: u32) -> Vec<u8> {
    let page_id = *document
        .get_pages()
        .get(&page_number)
        .expect("page number must exist");
    document
        .get_page_content(page_id)
        .expect("page content must decode")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_fixture_builds_loadable_document() {
        let document = PdfFixture::new().page_with_content(b"q Q").build();
        assert_eq!(document.get_pages().len(), 1);
    }

    #[test]
    fn pdf_fixture_round_trips_through_bytes() {
        let bytes = PdfFixture::new()
            .page_with_content(b"BT ET")
            .page_with_content(b"q Q")
            .into_bytes();
        let reloaded = Document::load_mem(&bytes).expect("fixture bytes must parse");
        assert_eq!(reloaded.get_pages().len(), 2);
        assert_eq!(decoded_page_content(&reloaded, 2), b"q Q");
    }

    #[test]
    #[should_panic(expected = "dangling references")]
    fn assert_no_dangling_references_detects_missing_object() {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        document.trailer.set("Info", Object::Reference((9999, 0)));
        assert_no_dangling_references(&document);
    }

    #[test]
    fn assert_no_dangling_references_accepts_complete_fixture() {
        let document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_image(FixtureImage::FlateRgb {
                width: 4,
                height: 4,
            })
            .with_info_and_xmp_metadata()
            .build();
        assert_no_dangling_references(&document);
    }

    #[test]
    fn with_flate_content_marks_streams_as_flate_and_decodes() {
        let document = PdfFixture::new()
            .page_with_content(b"BT /F1 12 Tf ET")
            .with_flate_content()
            .build();
        assert_eq!(decoded_page_content(&document, 1), b"BT /F1 12 Tf ET");
    }

    #[test]
    fn with_inherited_font_resource_puts_font_on_pages_node_only() {
        let document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_inherited_font_resource()
            .build();
        let page_id = *document.get_pages().get(&1).expect("page");
        assert!(!document
            .get_dictionary(page_id)
            .expect("page")
            .has(b"Resources"));
        let pages_id = document
            .get_dictionary(page_id)
            .and_then(|page| page.get(b"Parent"))
            .and_then(Object::as_reference)
            .expect("parent");
        let pages = document.get_dictionary(pages_id).expect("pages node");
        let resources = pages
            .get(b"Resources")
            .and_then(Object::as_dict)
            .expect("resources");
        assert!(resources.has(b"Font"));
    }

    #[test]
    fn with_image_registers_xobject_in_page_resources() {
        let document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_image(FixtureImage::Jpeg {
                width: 8,
                height: 8,
            })
            .build();
        let page_id = *document.get_pages().get(&1).expect("page");
        let (resources, _) = document.get_page_resources(page_id).expect("resources");
        let xobjects = resources
            .expect("resources")
            .get(b"XObject")
            .expect("xobject");
        assert!(matches!(xobjects, Object::Dictionary(d) if d.has(b"Im1")));
    }

    #[test]
    fn with_signature_field_adds_sig_widget_to_acroform_fields() {
        let document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_signature_field()
            .build();
        let catalog = document.catalog().expect("catalog");
        let form = catalog
            .get(b"AcroForm")
            .and_then(Object::as_dict)
            .expect("form");
        assert_eq!(
            form.get(b"Fields")
                .and_then(Object::as_array)
                .expect("fields")
                .len(),
            1
        );
    }
}
