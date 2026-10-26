use lopdf::{Dictionary, Object, Stream};

use crate::codec::lzw::{lzw_decode, LzwSettings};
use crate::codec::zlib::{deflate, inflate_bounded};
use crate::error::{AppError, AppResult};

const MIN_PREDICTOR_VALUE: i64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamFilter {
    Flate,
    Lzw { early_change: bool },
    Dct,
    Other(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamEncoding {
    pub filters: Vec<StreamFilter>,
    pub has_predictor: bool,
}

fn filter_names(dict: &Dictionary) -> Vec<Vec<u8>> {
    match dict.get(b"Filter") {
        Ok(Object::Name(name)) => vec![name.clone()],
        Ok(Object::Array(items)) => items
            .iter()
            .map(|item| match item {
                Object::Name(name) => name.clone(),
                _ => b"Unresolved".to_vec(),
            })
            .collect(),
        Ok(_) => vec![b"Unresolved".to_vec()],
        Err(_) => Vec::new(),
    }
}

fn decode_parms(dict: &Dictionary, filter_count: usize) -> Vec<Option<&Dictionary>> {
    let parms = dict.get(b"DecodeParms").or_else(|_| dict.get(b"DP"));
    match parms {
        Ok(Object::Dictionary(single)) => {
            let mut all = vec![None; filter_count];
            if let Some(first) = all.first_mut() {
                *first = Some(single);
            }
            all
        }
        Ok(Object::Array(items)) => (0..filter_count)
            .map(|index| match items.get(index) {
                Some(Object::Dictionary(parms)) => Some(parms),
                _ => None,
            })
            .collect(),
        _ => vec![None; filter_count],
    }
}

fn parm_integer(parms: Option<&Dictionary>, key: &[u8]) -> Option<i64> {
    parms
        .and_then(|dict| dict.get(key).ok())
        .and_then(|value| value.as_i64().ok())
}

fn classify(name: &[u8], parms: Option<&Dictionary>) -> StreamFilter {
    match name {
        b"FlateDecode" | b"Fl" => StreamFilter::Flate,
        b"LZWDecode" | b"LZW" => StreamFilter::Lzw {
            early_change: parm_integer(parms, b"EarlyChange") != Some(0),
        },
        b"DCTDecode" | b"DCT" => StreamFilter::Dct,
        other => StreamFilter::Other(other.to_vec()),
    }
}

impl StreamEncoding {
    pub fn from_dict(dict: &Dictionary) -> Self {
        let names = filter_names(dict);
        let parms = decode_parms(dict, names.len());
        let has_predictor = parms.iter().any(|parms| {
            parm_integer(*parms, b"Predictor").is_some_and(|p| p >= MIN_PREDICTOR_VALUE)
        });
        let filters = names
            .iter()
            .zip(parms.iter())
            .map(|(name, parms)| classify(name, *parms))
            .collect();
        Self {
            filters,
            has_predictor,
        }
    }

    pub fn is_plain_decodable(&self) -> bool {
        let known_single = matches!(
            self.filters.as_slice(),
            [] | [StreamFilter::Flate] | [StreamFilter::Lzw { .. }]
        );
        known_single && !self.has_predictor
    }

    pub fn is_unfiltered(&self) -> bool {
        self.filters.is_empty()
    }

    pub fn is_dct_only(&self) -> bool {
        self.filters == [StreamFilter::Dct]
    }

    pub fn is_flate_or_unfiltered(&self) -> bool {
        matches!(self.filters.as_slice(), [] | [StreamFilter::Flate])
    }

    pub fn unsupported_filter_names(&self) -> Vec<String> {
        self.filters
            .iter()
            .filter_map(|filter| match filter {
                StreamFilter::Other(name) => Some(String::from_utf8_lossy(name).into_owned()),
                _ => None,
            })
            .collect()
    }
}

pub fn decode_plain_stream(stream: &Stream, max_bytes: usize) -> AppResult<Vec<u8>> {
    let encoding = StreamEncoding::from_dict(&stream.dict);
    if !encoding.is_plain_decodable() {
        return Err(AppError::Decompress("unsupported filter chain".to_string()));
    }
    match encoding.filters.first() {
        None => {
            if stream.content.len() > max_bytes {
                return Err(AppError::DecompressionLimitExceeded { limit: max_bytes });
            }
            Ok(stream.content.clone())
        }
        Some(StreamFilter::Flate) => inflate_bounded(&stream.content, max_bytes),
        Some(StreamFilter::Lzw { early_change }) => lzw_decode(
            &stream.content,
            LzwSettings {
                early_change: *early_change,
                max_output_bytes: max_bytes,
            },
        ),
        Some(_) => Err(AppError::Decompress("unsupported filter chain".to_string())),
    }
}

pub fn replace_with_flate_bytes(stream: &mut Stream, flate_bytes: Vec<u8>) {
    let length = flate_bytes.len() as i64;
    stream.content = flate_bytes;
    stream.dict.set("Length", Object::Integer(length));
    stream
        .dict
        .set("Filter", Object::Name(b"FlateDecode".to_vec()));
    stream.dict.remove(b"DecodeParms");
    stream.dict.remove(b"DP");
}

pub fn store_as_flate(stream: &mut Stream, raw: &[u8], deflate_level: u32) -> AppResult<()> {
    replace_with_flate_bytes(stream, deflate(raw, deflate_level)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;

    fn name(value: &str) -> Object {
        Object::Name(value.as_bytes().to_vec())
    }

    fn dict_with_filter(filter: Object) -> Dictionary {
        let mut dict = Dictionary::new();
        dict.set("Filter", filter);
        dict
    }

    #[test]
    fn from_dict_parses_single_name() {
        let encoding = StreamEncoding::from_dict(&dict_with_filter(name("FlateDecode")));
        assert_eq!(encoding.filters, vec![StreamFilter::Flate]);
        assert!(!encoding.has_predictor);
    }

    #[test]
    fn from_dict_without_filter_is_unfiltered() {
        let encoding = StreamEncoding::from_dict(&Dictionary::new());
        assert!(encoding.is_unfiltered());
        assert!(encoding.is_plain_decodable());
    }

    #[test]
    fn from_dict_parses_filter_array_and_parms_array() {
        let mut dict = dict_with_filter(Object::Array(vec![
            name("ASCII85Decode"),
            name("LZWDecode"),
        ]));
        dict.set(
            "DecodeParms",
            Object::Array(vec![
                Object::Null,
                Object::Dictionary(dictionary! { "EarlyChange" => 0 }),
            ]),
        );
        let encoding = StreamEncoding::from_dict(&dict);
        assert_eq!(
            encoding.filters,
            vec![
                StreamFilter::Other(b"ASCII85Decode".to_vec()),
                StreamFilter::Lzw {
                    early_change: false
                }
            ]
        );
    }

    #[test]
    fn from_dict_expands_abbreviations() {
        let dict = dict_with_filter(Object::Array(vec![name("Fl"), name("LZW"), name("DCT")]));
        let encoding = StreamEncoding::from_dict(&dict);
        assert_eq!(
            encoding.filters,
            vec![
                StreamFilter::Flate,
                StreamFilter::Lzw { early_change: true },
                StreamFilter::Dct
            ]
        );
    }

    #[test]
    fn from_dict_detects_png_predictor() {
        let mut dict = dict_with_filter(name("FlateDecode"));
        dict.set(
            "DecodeParms",
            Object::Dictionary(dictionary! { "Predictor" => 15 }),
        );
        assert!(StreamEncoding::from_dict(&dict).has_predictor);
    }

    #[test]
    fn from_dict_ignores_predictor_one() {
        let mut dict = dict_with_filter(name("FlateDecode"));
        dict.set(
            "DecodeParms",
            Object::Dictionary(dictionary! { "Predictor" => 1 }),
        );
        assert!(!StreamEncoding::from_dict(&dict).has_predictor);
    }

    #[test]
    fn from_dict_marks_unresolved_filter_reference_as_other() {
        let dict = dict_with_filter(Object::Reference((5, 0)));
        let encoding = StreamEncoding::from_dict(&dict);
        assert!(!encoding.is_plain_decodable());
    }

    #[test]
    fn is_plain_decodable_accepts_single_flate_and_lzw() {
        for filter in ["FlateDecode", "LZWDecode"] {
            let encoding = StreamEncoding::from_dict(&dict_with_filter(name(filter)));
            assert!(encoding.is_plain_decodable(), "{filter}");
        }
    }

    #[test]
    fn is_plain_decodable_rejects_filter_chain() {
        let dict = dict_with_filter(Object::Array(vec![
            name("ASCIIHexDecode"),
            name("FlateDecode"),
        ]));
        assert!(!StreamEncoding::from_dict(&dict).is_plain_decodable());
    }

    #[test]
    fn is_plain_decodable_rejects_png_predictor() {
        let mut dict = dict_with_filter(name("FlateDecode"));
        dict.set(
            "DecodeParms",
            Object::Dictionary(dictionary! { "Predictor" => 12 }),
        );
        assert!(!StreamEncoding::from_dict(&dict).is_plain_decodable());
    }

    #[test]
    fn is_plain_decodable_rejects_dct() {
        let encoding = StreamEncoding::from_dict(&dict_with_filter(name("DCTDecode")));
        assert!(!encoding.is_plain_decodable());
        assert!(encoding.is_dct_only());
    }

    #[test]
    fn unsupported_filter_names_lists_only_unknown_filters() {
        let dict = dict_with_filter(Object::Array(vec![
            name("JBIG2Decode"),
            name("FlateDecode"),
        ]));
        assert_eq!(
            StreamEncoding::from_dict(&dict).unsupported_filter_names(),
            vec!["JBIG2Decode".to_string()]
        );
    }

    #[test]
    fn store_as_flate_removes_decode_parms_and_sets_length() {
        let mut dict = dict_with_filter(name("LZWDecode"));
        dict.set(
            "DecodeParms",
            Object::Dictionary(dictionary! { "EarlyChange" => 0 }),
        );
        let mut stream = Stream::new(dict, vec![1, 2, 3]);
        store_as_flate(&mut stream, b"hello hello hello", 9).unwrap();
        assert_eq!(
            stream.dict.get(b"Filter").unwrap(),
            &Object::Name(b"FlateDecode".to_vec())
        );
        assert!(!stream.dict.has(b"DecodeParms"));
        assert_eq!(
            stream.dict.get(b"Length").unwrap(),
            &Object::Integer(stream.content.len() as i64)
        );
        assert_eq!(
            decode_plain_stream(&stream, 1024).unwrap(),
            b"hello hello hello"
        );
    }

    #[test]
    fn decode_plain_stream_returns_unfiltered_bytes() {
        let stream = Stream::new(Dictionary::new(), b"plain".to_vec());
        assert_eq!(decode_plain_stream(&stream, 100).unwrap(), b"plain");
    }

    #[test]
    fn decode_plain_stream_enforces_limit_on_unfiltered_content() {
        let stream = Stream::new(Dictionary::new(), vec![0u8; 50]);
        let error = decode_plain_stream(&stream, 10).unwrap_err();
        assert!(matches!(
            error,
            AppError::DecompressionLimitExceeded { limit: 10 }
        ));
    }

    #[test]
    fn decode_plain_stream_rejects_unsupported_chain() {
        let stream = Stream::new(dict_with_filter(name("ASCII85Decode")), b"abc".to_vec());
        let error = decode_plain_stream(&stream, 100).unwrap_err();
        assert!(
            matches!(error, AppError::Decompress(message) if message == "unsupported filter chain")
        );
    }

    #[test]
    fn decode_plain_stream_decodes_flate_with_limit() {
        let compressed = deflate(&vec![1u8; 4096], 6).unwrap();
        let stream = Stream::new(dict_with_filter(name("FlateDecode")), compressed);
        assert_eq!(decode_plain_stream(&stream, 4096).unwrap(), vec![1u8; 4096]);
        let error = decode_plain_stream(&stream, 100).unwrap_err();
        assert!(matches!(
            error,
            AppError::DecompressionLimitExceeded { limit: 100 }
        ));
    }

    #[test]
    fn decode_plain_stream_decodes_lzw() {
        let encoded = vec![0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        let stream = Stream::new(dict_with_filter(name("LZWDecode")), encoded);
        assert_eq!(
            decode_plain_stream(&stream, 100).unwrap(),
            vec![0x2D, 0x2D, 0x2D, 0x2D, 0x2D, 0x41, 0x2D, 0x2D, 0x2D, 0x42]
        );
    }
}
