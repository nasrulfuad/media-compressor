use lopdf::{Document, Object, ObjectId, Stream};
use tracing::{debug, warn};

use crate::codec::zlib::deflate;
use crate::domain::options::PdfOptions;
use crate::error::AppResult;
use crate::pdf::content_minifier::minify_content;
use crate::pdf::filters::{decode_plain_stream, replace_with_flate_bytes, StreamEncoding};
use crate::pdf::object_graph::{content_stream_ids, dict_name_is, is_image_stream_dict};

use super::MIN_COMPRESSIBLE_SIZE;

const SKIPPED_TYPES: [&[u8]; 3] = [b"XRef", b"ObjStm", b"Metadata"];

fn is_skipped_stream(stream: &Stream) -> bool {
    is_image_stream_dict(&stream.dict)
        || SKIPPED_TYPES
            .iter()
            .any(|skipped| dict_name_is(&stream.dict, b"Type", skipped))
}

fn log_unsupported_encoding(id: ObjectId, encoding: &StreamEncoding) {
    let unsupported = encoding.unsupported_filter_names();
    if unsupported.is_empty() {
        debug!(object_id = ?id, filters = ?encoding.filters, "Stream left untouched: unsupported filter chain");
    } else {
        warn!(object_id = ?id, filters = ?unsupported, "Unsupported filter, stream left untouched");
    }
}

fn recompress_stream(
    id: ObjectId,
    stream: &mut Stream,
    is_content: bool,
    options: &PdfOptions,
) -> AppResult<Option<i64>> {
    let original_size = stream.content.len();
    if original_size < MIN_COMPRESSIBLE_SIZE || is_skipped_stream(stream) {
        return Ok(None);
    }
    let encoding = StreamEncoding::from_dict(&stream.dict);
    if !encoding.is_plain_decodable() {
        log_unsupported_encoding(id, &encoding);
        return Ok(None);
    }
    let decoded = match decode_plain_stream(stream, options.limits.max_stream_bytes) {
        Ok(decoded) => decoded,
        Err(error) => {
            warn!(object_id = ?id, error = %error, "Cannot decode stream; leaving untouched");
            return Ok(None);
        }
    };
    let payload = if is_content {
        minify_content(&decoded).unwrap_or(decoded)
    } else {
        decoded
    };
    let compressed = deflate(&payload, options.level.preset().deflate_level)?;
    if compressed.len() >= original_size {
        return Ok(None);
    }
    let saved = (original_size - compressed.len()) as i64;
    replace_with_flate_bytes(stream, compressed);
    Ok(Some(saved))
}

pub fn compress_streams(document: &mut Document, options: &PdfOptions) -> AppResult<i64> {
    let content_ids = content_stream_ids(document);
    let ids: Vec<ObjectId> = document.objects.keys().copied().collect();
    let mut total_saved = 0i64;
    for id in ids {
        let Some(Object::Stream(stream)) = document.objects.get_mut(&id) else {
            continue;
        };
        let is_content = content_ids.contains(&id);
        match recompress_stream(id, stream, is_content, options)? {
            Some(saved) => {
                total_saved += saved;
                debug!(object_id = ?id, bytes_saved = saved, "Stream re-compressed");
            }
            None => debug!(object_id = ?id, "Stream skipped"),
        }
    }
    Ok(total_saved)
}

#[cfg(test)]
mod tests {
    use lopdf::{dictionary, Dictionary};

    use super::*;
    use crate::codec::zlib::inflate_bounded;
    use crate::domain::level::CompressionLevel;
    use crate::pdf::content_lexer::tokenize;
    use crate::test_support::{
        decoded_page_content, FixtureImage, PdfFixture, ICC_LOOKING_PROFILE,
        SAMPLED_FUNCTION_SAMPLES,
    };

    fn options() -> PdfOptions {
        PdfOptions::for_level(CompressionLevel::Medium)
    }

    fn decoded(stream: &Stream) -> Vec<u8> {
        decode_plain_stream(stream, usize::MAX).unwrap()
    }

    fn find_stream<F: Fn(&Stream) -> bool>(
        document: &Document,
        predicate: F,
    ) -> (ObjectId, Stream) {
        document
            .objects
            .iter()
            .find_map(|(id, object)| match object {
                Object::Stream(stream) if predicate(stream) => Some((*id, stream.clone())),
                _ => None,
            })
            .expect("matching stream")
    }

    fn token_pairs(data: &[u8]) -> Vec<(String, Vec<u8>)> {
        tokenize(data)
            .unwrap()
            .into_iter()
            .map(|token| (format!("{:?}", token.kind), token.raw.to_vec()))
            .collect()
    }

    fn verbose_content() -> Vec<u8> {
        b"BT   /F1   12   Tf   (Discount 50%)   Tj   ET   % trailing comment\n".repeat(6)
    }

    #[test]
    fn compress_streams_keeps_percent_inside_string_literal() {
        let content = verbose_content();
        let mut document = PdfFixture::new().page_with_content(&content).build();
        let saved = compress_streams(&mut document, &options()).unwrap();
        assert!(saved > 0);
        let rewritten = decoded_page_content(&document, 1);
        assert!(rewritten.len() < content.len());
        let expected_tokens: Vec<_> = token_pairs(&content);
        assert_eq!(token_pairs(&rewritten), expected_tokens);
        assert!(rewritten
            .windows(14)
            .any(|window| window == b"(Discount 50%)"));
    }

    #[test]
    fn compress_streams_marks_stream_as_flate_and_updates_length() {
        let mut document = PdfFixture::new()
            .page_with_content(&verbose_content())
            .build();
        compress_streams(&mut document, &options()).unwrap();
        let page_id = *document.get_pages().get(&1).unwrap();
        let content_id = document.get_page_contents(page_id)[0];
        let stream = document
            .get_object(content_id)
            .unwrap()
            .as_stream()
            .unwrap();
        assert_eq!(
            stream.dict.get(b"Filter").unwrap(),
            &Object::Name(b"FlateDecode".to_vec())
        );
        assert_eq!(
            stream.dict.get(b"Length").unwrap(),
            &Object::Integer(stream.content.len() as i64)
        );
    }

    #[test]
    fn compress_streams_leaves_flate_ascii_hex_chain_untouched() {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let page_id = *document.get_pages().get(&1).unwrap();
        let content_id = document.get_page_contents(page_id)[0];
        let payload = b"3 0 0 1 0 0 cm   Q   ".repeat(20);
        let mut chained = Stream::new(
            dictionary! {
                "Filter" => vec![Object::Name(b"ASCIIHexDecode".to_vec()), Object::Name(b"FlateDecode".to_vec())],
            },
            payload,
        );
        chained
            .dict
            .set("Length", Object::Integer(chained.content.len() as i64));
        let before = chained.clone();
        document.objects.insert(content_id, Object::Stream(chained));
        compress_streams(&mut document, &options()).unwrap();
        assert_eq!(
            document
                .get_object(content_id)
                .unwrap()
                .as_stream()
                .unwrap(),
            &before
        );
    }

    #[test]
    fn compress_streams_leaves_predictor_stream_untouched() {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let page_id = *document.get_pages().get(&1).unwrap();
        let content_id = document.get_page_contents(page_id)[0];
        let compressed = deflate(&vec![9u8; 2048], 1).unwrap();
        let mut dict = Dictionary::new();
        dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
        dict.set(
            "DecodeParms",
            Object::Dictionary(dictionary! { "Predictor" => 15, "Columns" => 8 }),
        );
        let before = Stream::new(dict, compressed);
        document
            .objects
            .insert(content_id, Object::Stream(before.clone()));
        compress_streams(&mut document, &options()).unwrap();
        assert_eq!(
            document
                .get_object(content_id)
                .unwrap()
                .as_stream()
                .unwrap(),
            &before
        );
    }

    #[test]
    fn compress_streams_never_touches_icc_profile_stream() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_icc_color_space_stream()
            .build();
        compress_streams(&mut document, &options()).unwrap();
        let (_, profile) = find_stream(&document, |stream| stream.dict.has(b"N"));
        assert_eq!(decoded(&profile), ICC_LOOKING_PROFILE);
    }

    #[test]
    fn compress_streams_never_touches_sampled_function_stream() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_sampled_function_stream()
            .build();
        compress_streams(&mut document, &options()).unwrap();
        let (_, function) = find_stream(&document, |stream| stream.dict.has(b"FunctionType"));
        assert_eq!(decoded(&function), SAMPLED_FUNCTION_SAMPLES);
    }

    #[test]
    fn compress_streams_minifies_form_xobject_content() {
        let form_content = verbose_content();
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_form_xobject(&form_content)
            .build();
        compress_streams(&mut document, &options()).unwrap();
        let (_, form) = find_stream(&document, |stream| {
            crate::pdf::object_graph::is_form_stream_dict(&stream.dict)
        });
        assert!(decoded(&form).len() < form_content.len());
    }

    #[test]
    fn compress_streams_skips_image_streams() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_image(FixtureImage::FlateRgb {
                width: 16,
                height: 16,
            })
            .build();
        let (id, before) = find_stream(&document, |stream| is_image_stream_dict(&stream.dict));
        compress_streams(&mut document, &options()).unwrap();
        assert_eq!(
            document.get_object(id).unwrap().as_stream().unwrap(),
            &before
        );
    }

    #[test]
    fn compress_streams_leaves_bomb_stream_untouched() {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let page_id = *document.get_pages().get(&1).unwrap();
        let content_id = document.get_page_contents(page_id)[0];
        let bomb = deflate(&vec![0u8; 8 * 1024 * 1024], 9).unwrap();
        let mut dict = Dictionary::new();
        dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
        let before = Stream::new(dict, bomb);
        document
            .objects
            .insert(content_id, Object::Stream(before.clone()));
        let mut limited = options();
        limited.limits.max_stream_bytes = 1024 * 1024;
        compress_streams(&mut document, &limited).unwrap();
        assert_eq!(
            document
                .get_object(content_id)
                .unwrap()
                .as_stream()
                .unwrap(),
            &before
        );
        assert!(inflate_bounded(&before.content, usize::MAX).is_ok());
    }

    #[test]
    fn compress_streams_leaves_tiny_streams_untouched() {
        let mut document = PdfFixture::new().page_with_content(b"q  Q").build();
        let saved = compress_streams(&mut document, &options()).unwrap();
        assert_eq!(saved, 0);
        assert_eq!(decoded_page_content(&document, 1), b"q  Q");
    }
}
