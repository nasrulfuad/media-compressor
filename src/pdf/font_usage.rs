use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId};
use tracing::debug;

use super::content_lexer::{decode_hex_string, decode_literal_string, tokenize, Token, TokenKind};
use super::filters::decode_plain_stream;
use super::object_graph::{
    inherited_page_resources, is_form_stream_dict, resolve, resolve_array, resolve_dict,
};

const MAX_FORM_DEPTH: usize = 16;
const CID_BYTE_WIDTH: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FontRef {
    Referenced(ObjectId),
    Inline,
    Unresolved,
}

#[derive(Debug, Default)]
pub struct FontUsage {
    pub cids_by_font: BTreeMap<ObjectId, BTreeSet<u16>>,
    pub is_complete: bool,
}

#[derive(Default, Clone)]
struct ScopeResources {
    fonts: HashMap<Vec<u8>, FontRef>,
    xobjects: HashMap<Vec<u8>, ObjectId>,
}

fn font_ref_of(value: &Object) -> FontRef {
    match value {
        Object::Reference(id) => FontRef::Referenced(*id),
        _ => FontRef::Inline,
    }
}

fn scope_from_resources(document: &Document, resources: &Dictionary) -> ScopeResources {
    let mut scope = ScopeResources::default();
    if let Some(fonts) = resources
        .get(b"Font")
        .ok()
        .and_then(|value| resolve_dict(document, value))
    {
        for (name, value) in fonts.iter() {
            scope.fonts.insert(name.clone(), font_ref_of(value));
        }
    }
    if let Some(xobjects) = resources
        .get(b"XObject")
        .ok()
        .and_then(|value| resolve_dict(document, value))
    {
        for (name, value) in xobjects.iter() {
            if let Object::Reference(id) = value {
                scope.xobjects.insert(name.clone(), *id);
            }
        }
    }
    scope
}

fn stream_scope(
    document: &Document,
    stream_dict: &Dictionary,
    parent: &ScopeResources,
) -> ScopeResources {
    stream_dict
        .get(b"Resources")
        .ok()
        .and_then(|value| resolve_dict(document, value))
        .map(|resources| scope_from_resources(document, resources))
        .unwrap_or_else(|| parent.clone())
}

pub fn cids_from_string_bytes(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(CID_BYTE_WIDTH)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect()
}

fn string_bytes(token: &Token<'_>) -> Option<Vec<u8>> {
    match token.kind {
        TokenKind::LiteralString => Some(decode_literal_string(token.raw)),
        TokenKind::HexString => Some(decode_hex_string(token.raw)),
        _ => None,
    }
}

struct Walker<'a> {
    document: &'a Document,
    usage: FontUsage,
    visited: HashSet<ObjectId>,
}

impl<'a> Walker<'a> {
    fn record(&mut self, font: FontRef, tokens: &[Token<'_>]) {
        let FontRef::Referenced(font_id) = font else {
            if font == FontRef::Unresolved {
                self.usage.is_complete = false;
            }
            return;
        };
        let entry = self.usage.cids_by_font.entry(font_id).or_default();
        for token in tokens {
            if let Some(bytes) = string_bytes(token) {
                entry.extend(cids_from_string_bytes(&bytes));
            }
        }
    }

    fn walk_content(
        &mut self,
        content: &[u8],
        scope: &ScopeResources,
        initial_font: FontRef,
        depth: usize,
    ) {
        let Ok(tokens) = tokenize(content) else {
            self.usage.is_complete = false;
            return;
        };
        let mut operands: Vec<Token<'_>> = Vec::new();
        let mut font = initial_font;
        for token in tokens {
            if token.kind != TokenKind::Keyword {
                operands.push(token);
                continue;
            }
            match token.raw {
                b"Tf" => font = self.font_from_operands(&operands, scope),
                b"Tj" | b"'" | b"\"" | b"TJ" => self.record(font, &operands),
                b"Do" => self.follow_xobject(&operands, scope, font, depth),
                _ => {}
            }
            operands.clear();
        }
    }

    fn font_from_operands(&self, operands: &[Token<'_>], scope: &ScopeResources) -> FontRef {
        operands
            .iter()
            .find(|token| token.kind == TokenKind::Name)
            .and_then(|name| scope.fonts.get(&name.raw[1..]).copied())
            .unwrap_or(FontRef::Unresolved)
    }

    fn follow_xobject(
        &mut self,
        operands: &[Token<'_>],
        scope: &ScopeResources,
        font: FontRef,
        depth: usize,
    ) {
        let Some(name) = operands.iter().find(|token| token.kind == TokenKind::Name) else {
            return;
        };
        if let Some(id) = scope.xobjects.get(&name.raw[1..]) {
            self.walk_stream(*id, scope, font, depth + 1);
        }
    }

    fn walk_stream(&mut self, id: ObjectId, parent: &ScopeResources, font: FontRef, depth: usize) {
        if depth > MAX_FORM_DEPTH || !self.visited.insert(id) {
            return;
        }
        let Some(Object::Stream(stream)) = self.document.objects.get(&id) else {
            return;
        };
        if !is_form_stream_dict(&stream.dict) && !stream.dict.has(b"PatternType") {
            return;
        }
        let scope = stream_scope(self.document, &stream.dict, parent);
        match decode_plain_stream(stream, usize::MAX) {
            Ok(content) => self.walk_content(&content, &scope, font, depth),
            Err(error) => {
                debug!(object_id = ?id, error = %error, "Form stream not decodable for font usage scan");
                self.usage.is_complete = false;
            }
        }
    }

    fn walk_page(&mut self, page_id: ObjectId) {
        let resources = inherited_page_resources(self.document, page_id).unwrap_or_default();
        let scope = scope_from_resources(self.document, &resources);
        for content_id in self.document.get_page_contents(page_id) {
            let Some(Object::Stream(stream)) = self.document.objects.get(&content_id) else {
                continue;
            };
            match decode_plain_stream(stream, usize::MAX) {
                Ok(content) => self.walk_content(&content, &scope, FontRef::Unresolved, 0),
                Err(_) => self.usage.is_complete = false,
            }
        }
        self.walk_annotations(page_id, &scope);
    }

    fn walk_annotations(&mut self, page_id: ObjectId, page_scope: &ScopeResources) {
        let Some(annotations) = self
            .document
            .get_dictionary(page_id)
            .ok()
            .and_then(|page| page.get(b"Annots").ok())
            .and_then(|value| resolve_array(self.document, value))
        else {
            return;
        };
        let appearance_ids: Vec<ObjectId> = annotations
            .iter()
            .filter_map(|annotation| resolve_dict(self.document, annotation))
            .flat_map(|annotation| appearance_stream_ids(self.document, annotation))
            .collect();
        for id in appearance_ids {
            self.walk_stream(id, page_scope, FontRef::Unresolved, 0);
        }
    }

    fn walk_remaining_streams(&mut self) {
        let ids: Vec<ObjectId> = self.document.objects.keys().copied().collect();
        for id in ids {
            self.walk_stream(id, &ScopeResources::default(), FontRef::Unresolved, 0);
        }
    }
}

fn collect_stream_ids(document: &Document, value: &Object, found: &mut Vec<ObjectId>) {
    match value {
        Object::Reference(id) => match resolve(document, value) {
            Object::Stream(_) => found.push(*id),
            Object::Dictionary(states) => {
                for (_, state) in states.iter() {
                    collect_stream_ids(document, state, found);
                }
            }
            _ => {}
        },
        Object::Dictionary(states) => {
            for (_, state) in states.iter() {
                collect_stream_ids(document, state, found);
            }
        }
        _ => {}
    }
}

fn appearance_stream_ids(document: &Document, annotation: &Dictionary) -> Vec<ObjectId> {
    let mut found = Vec::new();
    let Some(appearance) = annotation
        .get(b"AP")
        .ok()
        .and_then(|value| resolve_dict(document, value))
    else {
        return found;
    };
    for key in [b"N".as_slice(), b"R", b"D"] {
        if let Ok(value) = appearance.get(key) {
            collect_stream_ids(document, value, &mut found);
        }
    }
    found
}

pub fn collect_font_usage(document: &Document) -> FontUsage {
    let mut walker = Walker {
        document,
        usage: FontUsage {
            cids_by_font: BTreeMap::new(),
            is_complete: true,
        },
        visited: HashSet::new(),
    };
    let pages: Vec<ObjectId> = document.page_iter().collect();
    for page_id in pages {
        walker.walk_page(page_id);
    }
    walker.walk_remaining_streams();
    walker.usage
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;
    use crate::test_support::PdfFixture;

    fn font_ids(document: &Document) -> Vec<ObjectId> {
        document
            .objects
            .iter()
            .filter(|(_, object)| matches!(object, Object::Dictionary(d) if d.type_is(b"Font")))
            .map(|(id, _)| *id)
            .collect()
    }

    fn fixture_with_font(content: &[u8]) -> (Document, ObjectId) {
        let document = PdfFixture::new()
            .page_with_content(content)
            .with_inherited_font_resource()
            .build();
        let font = font_ids(&document)[0];
        (document, font)
    }

    fn used(document: &Document, font: ObjectId) -> Vec<u16> {
        collect_font_usage(document)
            .cids_by_font
            .get(&font)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    #[test]
    fn collect_used_cids_reads_tj_and_tj_array_strings() {
        let (document, font) = fixture_with_font(
            b"BT /F1 12 Tf <00240025> Tj [<0026> -20 (\\000\\047)] TJ (\\000\\050) ' ET",
        );
        assert_eq!(used(&document, font), vec![0x24, 0x25, 0x26, 0x27, 0x28]);
    }

    #[test]
    fn collect_used_cids_reads_double_quote_operator_string() {
        let (document, font) = fixture_with_font(b"BT /F1 12 Tf 1 2 <0030> \" ET");
        assert_eq!(used(&document, font), vec![0x30]);
    }

    #[test]
    fn collect_used_cids_is_not_truncated_by_comments() {
        let (document, font) =
            fixture_with_font(b"BT /F1 12 Tf <0024> Tj % comment (with paren\n <0025> Tj ET");
        assert_eq!(used(&document, font), vec![0x24, 0x25]);
        assert!(collect_font_usage(&document).is_complete);
    }

    #[test]
    fn collect_used_cids_ignores_trailing_odd_byte() {
        let (document, font) = fixture_with_font(b"BT /F1 12 Tf <002400> Tj ET");
        assert_eq!(used(&document, font), vec![0x24]);
    }

    #[test]
    fn collect_used_cids_follows_form_xobjects_once() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q /Fm1 Do Q /Fm1 Do")
            .with_form_xobject(b"BT /F1 12 Tf <0041> Tj ET")
            .with_inherited_font_resource()
            .build();
        let font = font_ids(&document)[0];
        let form_id = document
            .objects
            .iter()
            .find_map(|(id, object)| match object {
                Object::Stream(s) if is_form_stream_dict(&s.dict) => Some(*id),
                _ => None,
            })
            .unwrap();
        if let Some(Object::Stream(form)) = document.objects.get_mut(&form_id) {
            form.dict.set(
                "Resources",
                Object::Dictionary(dictionary! {
                    "Font" => dictionary! { "F1" => font },
                }),
            );
        }
        assert_eq!(used(&document, font), vec![0x41]);
    }

    #[test]
    fn collect_used_cids_includes_annotation_appearance_streams() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_inherited_font_resource()
            .with_annotation("Widget")
            .build();
        let font = font_ids(&document)[0];
        let appearance = document.add_object(lopdf::Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Form",
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            b"BT /F1 9 Tf <0051> Tj ET".to_vec(),
        ));
        let page_id = *document.get_pages().get(&1).unwrap();
        let annotation_id = document
            .get_dictionary(page_id)
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .as_reference()
            .unwrap();
        document
            .get_dictionary_mut(annotation_id)
            .unwrap()
            .set("AP", Object::Dictionary(dictionary! { "N" => appearance }));
        assert_eq!(used(&document, font), vec![0x51]);
    }

    #[test]
    fn collect_used_cids_reads_unreachable_form_streams() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_inherited_font_resource()
            .build();
        let font = font_ids(&document)[0];
        document.add_object(lopdf::Stream::new(
            dictionary! {
                "Subtype" => "Form",
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            b"BT /F1 9 Tf <0061> Tj ET".to_vec(),
        ));
        assert_eq!(used(&document, font), vec![0x61]);
    }

    #[test]
    fn collect_font_usage_is_incomplete_when_content_cannot_be_tokenized() {
        let (document, _) = fixture_with_font(b"BT /F1 12 Tf (unterminated Tj ET");
        assert!(!collect_font_usage(&document).is_complete);
    }

    #[test]
    fn collect_font_usage_is_incomplete_when_text_is_shown_without_font() {
        let (document, _) = fixture_with_font(b"BT <0024> Tj ET");
        assert!(!collect_font_usage(&document).is_complete);
    }

    #[test]
    fn collect_font_usage_is_incomplete_when_content_filter_is_unsupported() {
        let (mut document, _) = fixture_with_font(b"BT /F1 12 Tf <0024> Tj ET");
        let page_id = *document.get_pages().get(&1).unwrap();
        let content_id = document.get_page_contents(page_id)[0];
        if let Some(Object::Stream(stream)) = document.objects.get_mut(&content_id) {
            stream
                .dict
                .set("Filter", Object::Name(b"ASCII85Decode".to_vec()));
        }
        assert!(!collect_font_usage(&document).is_complete);
    }

    #[test]
    fn collect_font_usage_ignores_text_in_inline_font_dictionaries() {
        let mut document = PdfFixture::new()
            .page_with_content(b"BT /F9 12 Tf <0024> Tj ET")
            .build();
        let page_id = *document.get_pages().get(&1).unwrap();
        document.get_dictionary_mut(page_id).unwrap().set(
            "Resources",
            Object::Dictionary(dictionary! {
                "Font" => dictionary! { "F9" => dictionary! { "Subtype" => "Type1" } },
            }),
        );
        let usage = collect_font_usage(&document);
        assert!(usage.is_complete);
        assert!(usage.cids_by_font.is_empty());
    }

    #[test]
    fn cids_from_string_bytes_pairs_big_endian() {
        assert_eq!(
            cids_from_string_bytes(&[0x01, 0x02, 0xFF, 0xFE]),
            vec![0x0102, 0xFFFE]
        );
        assert!(cids_from_string_bytes(&[0x01]).is_empty());
    }
}
