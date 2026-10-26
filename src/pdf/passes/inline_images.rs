use std::collections::HashMap;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use tracing::{debug, warn};

use crate::domain::options::PdfOptions;
use crate::error::AppResult;
use crate::pdf::content_lexer::{
    decode_hex_string, decode_literal_string, parse_inline_image, tokenize, Token, TokenKind,
};
use crate::pdf::filters::{decode_plain_stream, store_as_flate, StreamEncoding};
use crate::pdf::object_graph::{inherited_page_resources, page_content_stream_ids, resolve_dict};

const INLINE_XOBJECT_PREFIX: &str = "ImInl";
const DEVICE_COLOR_SPACES: [&[u8]; 3] = [b"DeviceGray", b"DeviceRGB", b"DeviceCMYK"];

fn expand_key(abbreviation: &[u8]) -> &[u8] {
    match abbreviation {
        b"BPC" => b"BitsPerComponent",
        b"CS" => b"ColorSpace",
        b"D" => b"Decode",
        b"DP" => b"DecodeParms",
        b"F" => b"Filter",
        b"H" => b"Height",
        b"IM" => b"ImageMask",
        b"I" => b"Interpolate",
        b"W" => b"Width",
        other => other,
    }
}

fn expand_value_name(key: &[u8], name: &[u8]) -> Vec<u8> {
    let expanded: &[u8] = match (key, name) {
        (b"ColorSpace", b"G") => b"DeviceGray",
        (b"ColorSpace", b"RGB") => b"DeviceRGB",
        (b"ColorSpace", b"CMYK") => b"DeviceCMYK",
        (b"ColorSpace", b"I") => b"Indexed",
        (b"Filter", b"AHx") => b"ASCIIHexDecode",
        (b"Filter", b"A85") => b"ASCII85Decode",
        (b"Filter", b"LZW") => b"LZWDecode",
        (b"Filter", b"Fl") => b"FlateDecode",
        (b"Filter", b"RL") => b"RunLengthDecode",
        (b"Filter", b"CCF") => b"CCITTFaxDecode",
        (b"Filter", b"DCT") => b"DCTDecode",
        _ => name,
    };
    expanded.to_vec()
}

fn parse_number(raw: &[u8]) -> Object {
    let text = String::from_utf8_lossy(raw);
    match text.parse::<i64>() {
        Ok(integer) => Object::Integer(integer),
        Err(_) => Object::Real(text.parse::<f32>().unwrap_or(0.0)),
    }
}

struct ValueParser<'t, 'a> {
    tokens: &'t [Token<'a>],
    position: usize,
}

impl<'t, 'a> ValueParser<'t, 'a> {
    fn next_token(&mut self) -> Option<Token<'a>> {
        let token = self.tokens.get(self.position).copied();
        self.position += 1;
        token
    }

    fn parse_value(&mut self, key: &[u8]) -> Option<Object> {
        let token = self.next_token()?;
        match token.kind {
            TokenKind::Number => Some(parse_number(token.raw)),
            TokenKind::Name => Some(Object::Name(expand_value_name(key, &token.raw[1..]))),
            TokenKind::LiteralString => Some(Object::String(
                decode_literal_string(token.raw),
                StringFormat::Literal,
            )),
            TokenKind::HexString => Some(Object::String(
                decode_hex_string(token.raw),
                StringFormat::Hexadecimal,
            )),
            TokenKind::ArrayStart => self.parse_array(key),
            TokenKind::DictStart => self.parse_dictionary(),
            TokenKind::Keyword => match token.raw {
                b"true" => Some(Object::Boolean(true)),
                b"false" => Some(Object::Boolean(false)),
                b"null" => Some(Object::Null),
                _ => None,
            },
            _ => None,
        }
    }

    fn parse_array(&mut self, key: &[u8]) -> Option<Object> {
        let mut items = Vec::new();
        loop {
            if self.tokens.get(self.position)?.kind == TokenKind::ArrayEnd {
                self.position += 1;
                return Some(Object::Array(items));
            }
            items.push(self.parse_value(key)?);
        }
    }

    fn parse_dictionary(&mut self) -> Option<Object> {
        let mut dict = Dictionary::new();
        loop {
            let key_token = self.next_token()?;
            if key_token.kind == TokenKind::DictEnd {
                return Some(Object::Dictionary(dict));
            }
            if key_token.kind != TokenKind::Name {
                return None;
            }
            let key = key_token.raw[1..].to_vec();
            let value = self.parse_value(&key)?;
            dict.set(key, value);
        }
    }
}

fn parse_image_dictionary(tokens: &[Token<'_>]) -> Option<Vec<(Vec<u8>, Object)>> {
    let mut parser = ValueParser {
        tokens,
        position: 0,
    };
    let mut entries = Vec::new();
    while parser.position < tokens.len() {
        let key_token = parser.next_token()?;
        if key_token.kind != TokenKind::Name {
            return None;
        }
        let key = expand_key(&key_token.raw[1..]).to_vec();
        let value = parser.parse_value(&key)?;
        entries.push((key, value));
    }
    Some(entries)
}

fn uses_named_color_space(entries: &[(Vec<u8>, Object)]) -> bool {
    entries.iter().any(|(key, value)| {
        key == b"ColorSpace"
            && matches!(value, Object::Name(name) if !DEVICE_COLOR_SPACES.contains(&name.as_slice()))
    })
}

fn build_image_xobject(entries: Vec<(Vec<u8>, Object)>, data: &[u8]) -> Stream {
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Image".to_vec()));
    for (key, value) in entries {
        dict.set(key, value);
    }
    dict.set("Length", Object::Integer(data.len() as i64));
    Stream::new(dict, data.to_vec())
}

fn offset_in(data: &[u8], slice: &[u8]) -> usize {
    slice.as_ptr() as usize - data.as_ptr() as usize
}

struct ConvertedContent {
    content: Vec<u8>,
    images: Vec<(String, Stream)>,
}

fn convert_content(data: &[u8], first_index: u32) -> Option<ConvertedContent> {
    let tokens = tokenize(data).ok()?;
    let mut content = Vec::with_capacity(data.len());
    let mut images: Vec<(String, Stream)> = Vec::new();
    let mut cursor = 0usize;
    for token in tokens
        .iter()
        .filter(|token| token.kind == TokenKind::InlineImage)
    {
        let Ok(inline) = parse_inline_image(token.raw) else {
            continue;
        };
        let Some(entries) = parse_image_dictionary(&inline.dictionary) else {
            continue;
        };
        if uses_named_color_space(&entries) {
            continue;
        }
        let name = format!(
            "{INLINE_XOBJECT_PREFIX}{}",
            first_index + images.len() as u32
        );
        let start = offset_in(data, token.raw);
        content.extend_from_slice(&data[cursor..start]);
        content.extend_from_slice(format!("/{name} Do").as_bytes());
        cursor = start + token.raw.len();
        images.push((name, build_image_xobject(entries, inline.data)));
    }
    if images.is_empty() {
        return None;
    }
    content.extend_from_slice(&data[cursor..]);
    Some(ConvertedContent { content, images })
}

fn contains_inline_image(data: &[u8]) -> bool {
    tokenize(data).is_ok_and(|tokens| {
        tokens
            .iter()
            .any(|token| token.kind == TokenKind::InlineImage)
    })
}

fn page_resources_for_update(document: &mut Document, page_id: ObjectId) -> AppResult<ObjectId> {
    let own_resources = document
        .get_dictionary(page_id)?
        .get(b"Resources")
        .ok()
        .cloned();
    let resources_id = match own_resources {
        Some(Object::Reference(id)) => return Ok(id),
        Some(Object::Dictionary(dict)) => document.add_object(Object::Dictionary(dict)),
        _ => {
            let inherited = inherited_page_resources(document, page_id).unwrap_or_default();
            document.add_object(Object::Dictionary(inherited))
        }
    };
    document
        .get_dictionary_mut(page_id)?
        .set("Resources", Object::Reference(resources_id));
    Ok(resources_id)
}

enum XObjectLocation {
    Direct,
    Indirect(ObjectId),
    Missing,
}

fn xobject_dictionary_location(document: &Document, resources_id: ObjectId) -> XObjectLocation {
    let Ok(resources) = document.get_dictionary(resources_id) else {
        return XObjectLocation::Missing;
    };
    match resources.get(b"XObject") {
        Ok(Object::Reference(id)) => XObjectLocation::Indirect(*id),
        Ok(Object::Dictionary(_)) => XObjectLocation::Direct,
        _ => XObjectLocation::Missing,
    }
}

fn xobject_dictionary_mut(
    document: &mut Document,
    resources_id: ObjectId,
) -> AppResult<&mut Dictionary> {
    let location = xobject_dictionary_location(document, resources_id);
    match location {
        XObjectLocation::Indirect(id) => Ok(document.get_dictionary_mut(id)?),
        XObjectLocation::Direct | XObjectLocation::Missing => {
            let resources = document.get_dictionary_mut(resources_id)?;
            if matches!(location, XObjectLocation::Missing) {
                resources.set("XObject", Object::Dictionary(Dictionary::new()));
            }
            match resources.get_mut(b"XObject") {
                Ok(Object::Dictionary(dict)) => Ok(dict),
                _ => Err(lopdf::Error::DictKey.into()),
            }
        }
    }
}

fn next_inline_index(document: &Document, resources_id: ObjectId) -> u32 {
    let xobjects = document
        .get_dictionary(resources_id)
        .ok()
        .and_then(|resources| resources.get(b"XObject").ok())
        .and_then(|value| resolve_dict(document, value));
    let highest = xobjects
        .map(|dict| {
            dict.iter()
                .filter_map(|(key, _)| {
                    let name = String::from_utf8_lossy(key);
                    name.strip_prefix(INLINE_XOBJECT_PREFIX)?
                        .parse::<u32>()
                        .ok()
                })
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    highest + 1
}

fn page_reference_counts(document: &Document) -> HashMap<ObjectId, usize> {
    let mut counts = HashMap::new();
    for page_id in document.page_iter() {
        for stream_id in page_content_stream_ids(document, page_id) {
            *counts.entry(stream_id).or_insert(0) += 1;
        }
    }
    counts
}

fn decode_content_stream(
    document: &Document,
    stream_id: ObjectId,
    options: &PdfOptions,
) -> Option<Vec<u8>> {
    let Some(Object::Stream(stream)) = document.objects.get(&stream_id) else {
        return None;
    };
    if !StreamEncoding::from_dict(&stream.dict).is_plain_decodable() {
        debug!(object_id = ?stream_id, "Inline image scan skipped: unsupported filter chain");
        return None;
    }
    match decode_plain_stream(stream, options.limits.max_stream_bytes) {
        Ok(decoded) => Some(decoded),
        Err(error) => {
            warn!(object_id = ?stream_id, error = %error, "Inline image scan skipped: cannot decode stream");
            None
        }
    }
}

fn convert_page(
    document: &mut Document,
    page_id: ObjectId,
    shared_counts: &HashMap<ObjectId, usize>,
    options: &PdfOptions,
) -> AppResult<usize> {
    let candidates: Vec<(ObjectId, Vec<u8>)> = page_content_stream_ids(document, page_id)
        .into_iter()
        .filter(|id| shared_counts.get(id).copied().unwrap_or(0) == 1)
        .filter_map(|id| decode_content_stream(document, id, options).map(|data| (id, data)))
        .filter(|(_, data)| contains_inline_image(data))
        .collect();
    if candidates.is_empty() {
        return Ok(0);
    }
    let resources_id = page_resources_for_update(document, page_id)?;
    let mut next_index = next_inline_index(document, resources_id);
    let mut converted = 0usize;
    for (stream_id, data) in candidates {
        let Some(result) = convert_content(&data, next_index) else {
            continue;
        };
        let Some(Object::Stream(original)) = document.objects.get(&stream_id) else {
            continue;
        };
        let mut rewritten = original.clone();
        if let Err(error) = store_as_flate(
            &mut rewritten,
            &result.content,
            options.level.preset().deflate_level,
        ) {
            warn!(object_id = ?stream_id, error = %error, "Cannot re-encode content stream; inline images kept");
            continue;
        }
        let mut new_ids = Vec::new();
        for (name, image) in result.images {
            let image_id = document.add_object(Object::Stream(image));
            new_ids.push((name, image_id));
        }
        next_index += new_ids.len() as u32;
        converted += new_ids.len();
        let xobjects = xobject_dictionary_mut(document, resources_id)?;
        for (name, image_id) in new_ids {
            xobjects.set(name, Object::Reference(image_id));
        }
        document
            .objects
            .insert(stream_id, Object::Stream(rewritten));
    }
    Ok(converted)
}

pub fn convert_inline_images(document: &mut Document, options: &PdfOptions) -> AppResult<usize> {
    let shared_counts = page_reference_counts(document);
    let pages: Vec<ObjectId> = document.page_iter().collect();
    let mut total = 0usize;
    for page_id in pages {
        total += convert_page(document, page_id, &shared_counts, options)?;
    }
    debug!(converted = total, "Inline images converted to XObjects");
    Ok(total)
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;
    use crate::domain::level::CompressionLevel;
    use crate::test_support::{decoded_page_content, PdfFixture};

    const INLINE_RGB_IMAGE: &[u8] =
        b"q 10 0 0 10 0 0 cm BI /W 2 /H 2 /CS /RGB /BPC 8 ID \x01\x02\x03\x04\x05\x06\x07\x08\x09\x0A\x0B\x0C EI Q";

    fn options() -> PdfOptions {
        PdfOptions::for_level(CompressionLevel::Medium)
    }

    fn page_id(document: &Document) -> ObjectId {
        *document.get_pages().get(&1).unwrap()
    }

    fn page_xobjects(document: &Document) -> Dictionary {
        let resources = inherited_page_resources(document, page_id(document)).unwrap();
        resolve_dict(document, resources.get(b"XObject").unwrap())
            .unwrap()
            .clone()
    }

    fn content_stream_bytes(document: &Document) -> Vec<u8> {
        let id = document.get_page_contents(page_id(document))[0];
        document
            .get_object(id)
            .unwrap()
            .as_stream()
            .unwrap()
            .content
            .clone()
    }

    #[test]
    fn inline_images_converts_image_and_registers_xobject() {
        let mut document = PdfFixture::new()
            .page_with_content(INLINE_RGB_IMAGE)
            .build();
        assert_eq!(convert_inline_images(&mut document, &options()).unwrap(), 1);
        assert_eq!(
            decoded_page_content(&document, 1),
            b"q 10 0 0 10 0 0 cm /ImInl1 Do Q"
        );
        let xobjects = page_xobjects(&document);
        let image_id = xobjects.get(b"ImInl1").unwrap().as_reference().unwrap();
        let image = document.get_object(image_id).unwrap().as_stream().unwrap();
        assert_eq!(
            image.dict.get(b"Subtype").unwrap(),
            &Object::Name(b"Image".to_vec())
        );
        assert_eq!(image.dict.get(b"Width").unwrap(), &Object::Integer(2));
        assert_eq!(image.dict.get(b"Height").unwrap(), &Object::Integer(2));
        assert_eq!(
            image.dict.get(b"ColorSpace").unwrap(),
            &Object::Name(b"DeviceRGB".to_vec())
        );
        assert_eq!(
            image.content,
            b"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0A\x0B\x0C"
        );
    }

    #[test]
    fn inline_images_keeps_inherited_font_resources() {
        let mut document = PdfFixture::new()
            .page_with_content(INLINE_RGB_IMAGE)
            .with_inherited_font_resource()
            .build();
        convert_inline_images(&mut document, &options()).unwrap();
        let resources = inherited_page_resources(&document, page_id(&document)).unwrap();
        let fonts = resolve_dict(&document, resources.get(b"Font").unwrap()).unwrap();
        assert!(fonts.has(b"F1"));
        assert!(page_xobjects(&document).has(b"ImInl1"));
    }

    #[test]
    fn inline_images_registers_xobject_when_resources_is_indirect_with_direct_xobject_dict() {
        let mut document = PdfFixture::new()
            .page_with_content(INLINE_RGB_IMAGE)
            .build();
        let existing_image = document.add_object(Object::Null);
        let resources_id = document.add_object(Object::Dictionary(dictionary! {
            "XObject" => dictionary! { "Im1" => existing_image },
        }));
        document
            .get_dictionary_mut(page_id(&document))
            .unwrap()
            .set("Resources", Object::Reference(resources_id));
        convert_inline_images(&mut document, &options()).unwrap();
        let resources = document.get_dictionary(resources_id).unwrap();
        let xobjects = resources.get(b"XObject").unwrap().as_dict().unwrap();
        assert!(xobjects.has(b"Im1"));
        assert!(xobjects.has(b"ImInl1"));
    }

    #[test]
    fn inline_images_registers_xobject_when_xobject_dict_is_indirect() {
        let mut document = PdfFixture::new()
            .page_with_content(INLINE_RGB_IMAGE)
            .build();
        let xobject_dict_id = document.add_object(Object::Dictionary(Dictionary::new()));
        document
            .get_dictionary_mut(page_id(&document))
            .unwrap()
            .set(
                "Resources",
                Object::Dictionary(dictionary! { "XObject" => xobject_dict_id }),
            );
        convert_inline_images(&mut document, &options()).unwrap();
        let xobjects = document.get_dictionary(xobject_dict_id).unwrap();
        assert!(xobjects.has(b"ImInl1"));
    }

    #[test]
    fn inline_images_numbers_new_xobjects_above_existing_inline_names() {
        let mut document = PdfFixture::new()
            .page_with_content(INLINE_RGB_IMAGE)
            .build();
        let existing_image = document.add_object(Object::Null);
        document
            .get_dictionary_mut(page_id(&document))
            .unwrap()
            .set(
                "Resources",
                Object::Dictionary(dictionary! {
                    "XObject" => dictionary! { "ImInl7" => existing_image },
                }),
            );
        convert_inline_images(&mut document, &options()).unwrap();
        assert!(page_xobjects(&document).has(b"ImInl8"));
        assert_eq!(
            decoded_page_content(&document, 1),
            b"q 10 0 0 10 0 0 cm /ImInl8 Do Q"
        );
    }

    #[test]
    fn inline_images_skips_stream_with_unsupported_filter_chain() {
        let mut document = PdfFixture::new()
            .page_with_content(INLINE_RGB_IMAGE)
            .build();
        let content_id = document.get_page_contents(page_id(&document))[0];
        if let Some(Object::Stream(stream)) = document.objects.get_mut(&content_id) {
            stream.dict.set(
                "Filter",
                Object::Array(vec![
                    Object::Name(b"ASCII85Decode".to_vec()),
                    Object::Name(b"FlateDecode".to_vec()),
                ]),
            );
        }
        let before = content_stream_bytes(&document);
        assert_eq!(convert_inline_images(&mut document, &options()).unwrap(), 0);
        assert_eq!(content_stream_bytes(&document), before);
    }

    #[test]
    fn inline_images_expands_abbreviated_filter_array() {
        let content = b"BI /W 1 /H 1 /CS /G /BPC 8 /F [/AHx /Fl] ID 00 EI";
        let mut document = PdfFixture::new().page_with_content(content).build();
        convert_inline_images(&mut document, &options()).unwrap();
        let xobjects = page_xobjects(&document);
        let image_id = xobjects.get(b"ImInl1").unwrap().as_reference().unwrap();
        let image = document.get_object(image_id).unwrap().as_stream().unwrap();
        assert_eq!(
            image.dict.get(b"Filter").unwrap(),
            &Object::Array(vec![
                Object::Name(b"ASCIIHexDecode".to_vec()),
                Object::Name(b"FlateDecode".to_vec())
            ])
        );
    }

    #[test]
    fn inline_images_expands_indexed_color_space_array() {
        let content = b"BI /W 1 /H 1 /BPC 8 /CS [/I /RGB 1 <000000FFFFFF>] ID \x00 EI";
        let mut document = PdfFixture::new().page_with_content(content).build();
        assert_eq!(convert_inline_images(&mut document, &options()).unwrap(), 1);
        let xobjects = page_xobjects(&document);
        let image_id = xobjects.get(b"ImInl1").unwrap().as_reference().unwrap();
        let image = document.get_object(image_id).unwrap().as_stream().unwrap();
        let color_space = image.dict.get(b"ColorSpace").unwrap().as_array().unwrap();
        assert_eq!(color_space[0], Object::Name(b"Indexed".to_vec()));
        assert_eq!(color_space[1], Object::Name(b"DeviceRGB".to_vec()));
    }

    #[test]
    fn inline_images_ignores_bi_inside_literal_string() {
        let content = b"BT (BI ID EI) Tj ET";
        let mut document = PdfFixture::new().page_with_content(content).build();
        let before = content_stream_bytes(&document);
        assert_eq!(convert_inline_images(&mut document, &options()).unwrap(), 0);
        assert_eq!(content_stream_bytes(&document), before);
    }

    #[test]
    fn inline_images_skips_named_resource_color_space() {
        let content = b"BI /W 1 /H 1 /BPC 8 /CS /Cs1 ID \x00 EI";
        let mut document = PdfFixture::new().page_with_content(content).build();
        let before = content_stream_bytes(&document);
        assert_eq!(convert_inline_images(&mut document, &options()).unwrap(), 0);
        assert_eq!(content_stream_bytes(&document), before);
    }

    #[test]
    fn inline_images_skips_content_stream_shared_by_two_pages() {
        let mut document = PdfFixture::new()
            .page_with_content(INLINE_RGB_IMAGE)
            .page_with_content(b"q Q")
            .build();
        let first = *document.get_pages().get(&1).unwrap();
        let second = *document.get_pages().get(&2).unwrap();
        let shared = document.get_page_contents(first)[0];
        document
            .get_dictionary_mut(second)
            .unwrap()
            .set("Contents", Object::Reference(shared));
        let before = content_stream_bytes(&document);
        assert_eq!(convert_inline_images(&mut document, &options()).unwrap(), 0);
        assert_eq!(content_stream_bytes(&document), before);
    }

    #[test]
    fn inline_images_converts_multiple_images_in_one_stream_with_unique_names() {
        let content =
            b"BI /W 1 /H 1 /CS /G /BPC 8 ID \x01 EI q BI /W 1 /H 1 /CS /G /BPC 8 ID \x02 EI Q";
        let mut document = PdfFixture::new().page_with_content(content).build();
        assert_eq!(convert_inline_images(&mut document, &options()).unwrap(), 2);
        assert_eq!(
            decoded_page_content(&document, 1),
            b"/ImInl1 Do q /ImInl2 Do Q"
        );
    }

    #[test]
    fn inline_images_rewrites_flate_content_stream_and_preserves_other_operators() {
        let mut document = PdfFixture::new()
            .page_with_content(INLINE_RGB_IMAGE)
            .with_flate_content()
            .build();
        convert_inline_images(&mut document, &options()).unwrap();
        assert_eq!(
            decoded_page_content(&document, 1),
            b"q 10 0 0 10 0 0 cm /ImInl1 Do Q"
        );
    }

    #[test]
    fn inline_images_leaves_document_without_inline_images_untouched() {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let object_count = document.objects.len();
        assert_eq!(convert_inline_images(&mut document, &options()).unwrap(), 0);
        assert_eq!(document.objects.len(), object_count);
    }
}
