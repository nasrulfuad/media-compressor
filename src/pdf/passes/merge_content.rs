use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use tracing::{debug, warn};

use crate::codec::zlib::deflate;
use crate::domain::options::PdfOptions;
use crate::error::AppResult;
use crate::pdf::filters::{decode_plain_stream, replace_with_flate_bytes};
use crate::pdf::object_graph::page_content_stream_ids;

const CONTENT_SEPARATOR: u8 = b'\n';

struct MergeCandidate {
    page_id: ObjectId,
    stream_ids: Vec<ObjectId>,
}

fn merge_candidates(document: &Document) -> Vec<MergeCandidate> {
    document
        .page_iter()
        .filter_map(|page_id| {
            let is_array = matches!(
                document
                    .get_dictionary(page_id)
                    .ok()?
                    .get(b"Contents")
                    .ok()?,
                Object::Array(_)
            );
            let stream_ids = page_content_stream_ids(document, page_id);
            (is_array && stream_ids.len() > 1).then_some(MergeCandidate {
                page_id,
                stream_ids,
            })
        })
        .collect()
}

struct MergedContent {
    payload: Vec<u8>,
    old_stored_size: usize,
}

fn merge_payloads(
    document: &Document,
    candidate: &MergeCandidate,
    max_stream_bytes: usize,
) -> Option<MergedContent> {
    let mut payload = Vec::new();
    let mut old_stored_size = 0usize;
    for stream_id in &candidate.stream_ids {
        let Some(Object::Stream(stream)) = document.objects.get(stream_id) else {
            return None;
        };
        old_stored_size += stream.content.len();
        match decode_plain_stream(stream, max_stream_bytes) {
            Ok(decoded) => {
                if !payload.is_empty() {
                    payload.push(CONTENT_SEPARATOR);
                }
                payload.extend_from_slice(&decoded);
            }
            Err(error) => {
                warn!(stream_id = ?stream_id, error = %error, "Cannot decode content stream for merge; skipping page");
                return None;
            }
        }
    }
    (!payload.is_empty()).then_some(MergedContent {
        payload,
        old_stored_size,
    })
}

pub fn merge_page_content_streams(document: &mut Document, options: &PdfOptions) -> AppResult<i64> {
    let mut total_saved = 0i64;
    for candidate in merge_candidates(document) {
        let Some(merged) = merge_payloads(document, &candidate, options.limits.max_stream_bytes)
        else {
            continue;
        };
        let compressed = deflate(&merged.payload, options.level.preset().deflate_level)?;
        if compressed.len() >= merged.old_stored_size {
            continue;
        }
        let saved = (merged.old_stored_size - compressed.len()) as i64;
        let mut merged_stream = Stream::new(Dictionary::new(), Vec::new());
        replace_with_flate_bytes(&mut merged_stream, compressed);
        let merged_id = document.add_object(Object::Stream(merged_stream));
        if let Ok(page) = document.get_dictionary_mut(candidate.page_id) {
            page.set("Contents", Object::Reference(merged_id));
        }
        total_saved += saved;
        debug!(page_id = ?candidate.page_id, streams_merged = candidate.stream_ids.len(), bytes_saved = saved, "Content streams merged");
    }
    Ok(total_saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::level::CompressionLevel;
    use crate::test_support::{decoded_page_content, PdfFixture};

    fn options() -> PdfOptions {
        PdfOptions::for_level(CompressionLevel::Medium)
    }

    fn page_content_object(document: &Document) -> Object {
        let page_id = *document.get_pages().get(&1).unwrap();
        document
            .get_dictionary(page_id)
            .unwrap()
            .get(b"Contents")
            .unwrap()
            .clone()
    }

    #[test]
    fn merge_page_content_streams_joins_streams_with_newline_and_single_reference() {
        let first = b"q 1 0 0 1 10 10 cm 0 0 100 100 re f Q ".repeat(10);
        let second = b"BT /F1 12 Tf (Hello) Tj ET ".repeat(10);
        let mut document = PdfFixture::new()
            .page_with_content_streams(&[&first, &second])
            .build();
        let saved = merge_page_content_streams(&mut document, &options()).unwrap();
        assert!(saved > 0);
        assert!(matches!(
            page_content_object(&document),
            Object::Reference(_)
        ));
        let mut expected = first.clone();
        expected.push(b'\n');
        expected.extend_from_slice(&second);
        assert_eq!(decoded_page_content(&document, 1), expected);
    }

    #[test]
    fn merge_page_content_streams_ignores_single_stream_pages() {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let before = page_content_object(&document);
        assert_eq!(
            merge_page_content_streams(&mut document, &options()).unwrap(),
            0
        );
        assert_eq!(page_content_object(&document), before);
    }

    #[test]
    fn merge_page_content_streams_skips_page_with_undecodable_stream() {
        let first = b"q Q ".repeat(40);
        let mut document = PdfFixture::new()
            .page_with_content_streams(&[&first, &first])
            .build();
        let page_id = *document.get_pages().get(&1).unwrap();
        let second_id = document.get_page_contents(page_id)[1];
        if let Some(Object::Stream(stream)) = document.objects.get_mut(&second_id) {
            stream
                .dict
                .set("Filter", Object::Name(b"ASCII85Decode".to_vec()));
        }
        let before = page_content_object(&document);
        assert_eq!(
            merge_page_content_streams(&mut document, &options()).unwrap(),
            0
        );
        assert_eq!(page_content_object(&document), before);
    }

    #[test]
    fn merge_page_content_streams_keeps_original_when_merge_is_not_smaller() {
        let mut document = PdfFixture::new()
            .page_with_content_streams(&[b"q", b"Q"])
            .build();
        let before = page_content_object(&document);
        assert_eq!(
            merge_page_content_streams(&mut document, &options()).unwrap(),
            0
        );
        assert_eq!(page_content_object(&document), before);
    }
}
