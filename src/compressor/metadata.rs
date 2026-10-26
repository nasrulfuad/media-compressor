use lopdf::{Document, Object, ObjectId};
use tracing::debug;

use crate::compressor::pdf_utils::estimate_object_size;

/// Strip document metadata to reduce file size.
///
/// Removes the `/Info` dictionary referenced from the trailer and any
/// XMP metadata streams (streams whose `/Type` is `/Metadata`). The
/// return value is an estimate of the bytes removed.
pub fn strip_metadata(doc: &mut Document) -> u64 {
    let mut bytes_removed: u64 = 0;

    // ── Remove /Info dictionary ───────────────────────────────────────────────
    if let Ok(Object::Reference(info_id)) = doc.trailer.get(b"Info").cloned() {
        if let Some(obj) = doc.objects.remove(&info_id) {
            bytes_removed += estimate_object_size(&obj);
            debug!(object_id = ?info_id, "Removed /Info dictionary");
        }
        doc.trailer.remove(b"Info");
    }

    // ── Remove XMP metadata streams ───────────────────────────────────────────
    let xmp_ids: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter(|(_, obj)| is_xmp_stream(obj))
        .map(|(id, _)| *id)
        .collect();

    for id in xmp_ids {
        if let Some(obj) = doc.objects.remove(&id) {
            bytes_removed += estimate_object_size(&obj);
            debug!(object_id = ?id, "Removed XMP metadata stream");
        }
    }

    bytes_removed
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn is_xmp_stream(obj: &Object) -> bool {
    match obj {
        Object::Stream(s) => {
            matches!(s.dict.get(b"Type"), Ok(Object::Name(n)) if n == b"Metadata")
        }
        _ => false,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Dictionary, Object, Stream};

    fn make_xmp_stream() -> Object {
        let mut dict = Dictionary::new();
        dict.set("Type", Object::Name(b"Metadata".to_vec()));
        dict.set("Subtype", Object::Name(b"XML".to_vec()));
        Object::Stream(Stream {
            dict,
            content: b"<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>".to_vec(),
            allows_compression: false,
            start_position: None,
        })
    }

    #[test]
    fn is_xmp_stream_detects_metadata_stream() {
        let obj = make_xmp_stream();
        assert!(is_xmp_stream(&obj));
    }

    #[test]
    fn is_xmp_stream_rejects_non_metadata() {
        let obj = Object::Integer(42);
        assert!(!is_xmp_stream(&obj));
    }

    #[test]
    fn estimate_size_stream() {
        let xmp = make_xmp_stream();
        let size = estimate_object_size(&xmp);
        assert!(size > 0);
    }

    #[test]
    fn strip_metadata_removes_xmp_stream() {
        let mut doc = Document::new();

        // Insert a fake XMP stream object.
        let xmp_id = doc.add_object(make_xmp_stream());

        assert!(doc.objects.contains_key(&xmp_id));
        strip_metadata(&mut doc);
        assert!(
            !doc.objects.contains_key(&xmp_id),
            "XMP stream should have been removed"
        );
    }
}
