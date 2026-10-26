use lopdf::{Dictionary, Document, Object, ObjectId};
use tracing::{debug, warn};

use crate::domain::level::CleanupPolicy;
use crate::error::AppResult;
use crate::pdf::object_graph::{
    catalog_id, child_dict_mut, dict_name_is, resolve, resolve_array, resolve_dict,
};

const MARKUP_ANNOTATION_SUBTYPES: [&[u8]; 17] = [
    b"Text",
    b"FreeText",
    b"Line",
    b"Square",
    b"Circle",
    b"Polygon",
    b"PolyLine",
    b"Highlight",
    b"Underline",
    b"Squiggly",
    b"StrikeOut",
    b"Stamp",
    b"Caret",
    b"Ink",
    b"Popup",
    b"Sound",
    b"Redact",
];
const FILE_ATTACHMENT_SUBTYPE: &[u8] = b"FileAttachment";
const WIDGET_SUBTYPE: &[u8] = b"Widget";
const MAX_ACTION_CHAIN_DEPTH: usize = 16;
const MAX_FIELD_TREE_DEPTH: usize = 64;

fn is_javascript_action(document: &Document, object: &Object, depth: usize) -> bool {
    if depth > MAX_ACTION_CHAIN_DEPTH {
        return false;
    }
    let Some(action) = resolve_dict(document, object) else {
        return false;
    };
    if dict_name_is(action, b"S", b"JavaScript") {
        return true;
    }
    match action.get(b"Next") {
        Ok(next) => match resolve(document, next) {
            Object::Array(items) => items
                .iter()
                .any(|item| is_javascript_action(document, item, depth + 1)),
            other => is_javascript_action(document, other, depth + 1),
        },
        Err(_) => false,
    }
}

fn additional_actions_contain_javascript(document: &Document, object: &Object) -> bool {
    resolve_dict(document, object).is_some_and(|actions| {
        actions
            .iter()
            .any(|(_, action)| is_javascript_action(document, action, 0))
    })
}

fn owner_has_javascript_entry(document: &Document, owner: &Dictionary, key: &[u8]) -> bool {
    let Ok(value) = owner.get(key) else {
        return false;
    };
    match key {
        b"AA" => additional_actions_contain_javascript(document, value),
        _ => is_javascript_action(document, value, 0),
    }
}

fn remove_javascript_entries(document: &mut Document, owner_id: ObjectId, keys: &[&[u8]]) {
    let doomed: Vec<Vec<u8>> = match document.get_dictionary(owner_id) {
        Ok(owner) => keys
            .iter()
            .filter(|key| owner_has_javascript_entry(document, owner, key))
            .map(|key| key.to_vec())
            .collect(),
        Err(_) => return,
    };
    if let Ok(owner) = document.get_dictionary_mut(owner_id) {
        for key in doomed {
            owner.remove(&key);
        }
    }
}

fn remove_keys(document: &mut Document, owner_id: ObjectId, keys: &[&[u8]]) {
    if let Ok(owner) = document.get_dictionary_mut(owner_id) {
        for key in keys {
            owner.remove(key);
        }
    }
}

fn remove_catalog_names_entries(document: &mut Document, catalog: ObjectId, entries: &[&[u8]]) {
    let Some(names) = child_dict_mut(document, catalog, b"Names") else {
        return;
    };
    for entry in entries {
        names.remove(entry);
    }
    let names_is_empty = names.is_empty();
    if names_is_empty {
        remove_keys(document, catalog, &[b"Names"]);
    }
}

fn annotation_subtype(document: &Document, annotation: &Object) -> Option<Vec<u8>> {
    let dict = resolve_dict(document, annotation)?;
    dict.get(b"Subtype")
        .ok()
        .and_then(|subtype| subtype.as_name().ok())
        .map(<[u8]>::to_vec)
}

fn rewrite_annotations(document: &mut Document, page_id: ObjectId, annotations: Vec<Object>) {
    let storage = document
        .get_dictionary(page_id)
        .ok()
        .and_then(|page| page.get(b"Annots").ok())
        .and_then(|value| value.as_reference().ok());
    if let (Some(array_id), false) = (storage, annotations.is_empty()) {
        document
            .objects
            .insert(array_id, Object::Array(annotations));
        return;
    }
    if let Ok(page) = document.get_dictionary_mut(page_id) {
        if annotations.is_empty() {
            page.remove(b"Annots");
        } else {
            page.set("Annots", Object::Array(annotations));
        }
    }
}

fn page_annotations(document: &Document, page_id: ObjectId) -> Vec<Object> {
    document
        .get_dictionary(page_id)
        .ok()
        .and_then(|page| page.get(b"Annots").ok())
        .and_then(|value| resolve_array(document, value))
        .cloned()
        .unwrap_or_default()
}

fn remove_annotations_with_subtypes(document: &mut Document, subtypes: &[&[u8]]) {
    if subtypes.is_empty() {
        return;
    }
    let pages: Vec<ObjectId> = document.page_iter().collect();
    for page_id in pages {
        let annotations = page_annotations(document, page_id);
        let original_len = annotations.len();
        let kept: Vec<Object> = annotations
            .into_iter()
            .filter(|annotation| {
                annotation_subtype(document, annotation)
                    .map_or(true, |subtype| !subtypes.contains(&subtype.as_slice()))
            })
            .collect();
        if kept.len() != original_len {
            rewrite_annotations(document, page_id, kept);
        }
    }
}

fn strip_javascript_from_annotations(document: &mut Document) {
    let pages: Vec<ObjectId> = document.page_iter().collect();
    for page_id in pages {
        for annotation in page_annotations(document, page_id) {
            if let Object::Reference(annotation_id) = annotation {
                remove_javascript_entries(document, annotation_id, &[b"A", b"AA"]);
            }
        }
    }
}

fn remove_javascript(document: &mut Document) {
    if let Some(catalog) = catalog_id(document) {
        remove_javascript_entries(document, catalog, &[b"OpenAction", b"AA"]);
        remove_catalog_names_entries(document, catalog, &[b"JavaScript"]);
    }
    let pages: Vec<ObjectId> = document.page_iter().collect();
    for page_id in pages {
        remove_javascript_entries(document, page_id, &[b"AA"]);
    }
    strip_javascript_from_annotations(document);
}

fn remove_thumbnails_and_piece_info(document: &mut Document) {
    if let Some(catalog) = catalog_id(document) {
        remove_keys(document, catalog, &[b"PieceInfo"]);
    }
    let pages: Vec<ObjectId> = document.page_iter().collect();
    for page_id in pages {
        remove_keys(document, page_id, &[b"Thumb", b"PieceInfo"]);
    }
}

fn remove_struct_tree(document: &mut Document) {
    if let Some(catalog) = catalog_id(document) {
        remove_keys(document, catalog, &[b"StructTreeRoot", b"MarkInfo"]);
    }
    let pages: Vec<ObjectId> = document.page_iter().collect();
    for page_id in pages {
        remove_keys(document, page_id, &[b"StructParents"]);
        for annotation in page_annotations(document, page_id) {
            if let Object::Reference(annotation_id) = annotation {
                remove_keys(document, annotation_id, &[b"StructParent"]);
            }
        }
    }
}

fn field_tree_has_signature(
    document: &Document,
    field: &Object,
    inherited_type: Option<Vec<u8>>,
    depth: usize,
) -> bool {
    if depth > MAX_FIELD_TREE_DEPTH {
        return false;
    }
    let Some(dict) = resolve_dict(document, field) else {
        return false;
    };
    let field_type = dict
        .get(b"FT")
        .ok()
        .and_then(|value| value.as_name().ok())
        .map(<[u8]>::to_vec)
        .or(inherited_type);
    if field_type.as_deref() == Some(b"Sig".as_slice()) {
        return true;
    }
    dict.get(b"Kids")
        .ok()
        .and_then(|kids| resolve_array(document, kids))
        .is_some_and(|kids| {
            kids.iter()
                .any(|kid| field_tree_has_signature(document, kid, field_type.clone(), depth + 1))
        })
}

fn acroform_has_signature(document: &Document) -> bool {
    let Some(catalog) = catalog_id(document).and_then(|id| document.get_dictionary(id).ok()) else {
        return false;
    };
    let Some(form) = catalog
        .get(b"AcroForm")
        .ok()
        .and_then(|value| resolve_dict(document, value))
    else {
        return false;
    };
    form.get(b"Fields")
        .ok()
        .and_then(|fields| resolve_array(document, fields))
        .is_some_and(|fields| {
            fields
                .iter()
                .any(|field| field_tree_has_signature(document, field, None, 0))
        })
}

fn should_remove_forms(document: &Document, policy: &CleanupPolicy) -> bool {
    if !policy.remove_forms {
        return false;
    }
    if acroform_has_signature(document) {
        warn!(reason = "signature field present", "Form removal skipped");
        return false;
    }
    true
}

fn removable_annotation_subtypes(policy: &CleanupPolicy, remove_forms: bool) -> Vec<&'static [u8]> {
    let mut subtypes: Vec<&'static [u8]> = Vec::new();
    if policy.remove_markup_annotations {
        subtypes.extend(MARKUP_ANNOTATION_SUBTYPES);
    }
    if policy.remove_embedded_files || policy.remove_markup_annotations {
        subtypes.push(FILE_ATTACHMENT_SUBTYPE);
    }
    if remove_forms {
        subtypes.push(WIDGET_SUBTYPE);
    }
    subtypes
}

fn remove_catalog_features(document: &mut Document, policy: &CleanupPolicy, remove_forms: bool) {
    let Some(catalog) = catalog_id(document) else {
        return;
    };
    if remove_forms {
        remove_keys(document, catalog, &[b"AcroForm"]);
    }
    if policy.remove_output_intents {
        remove_keys(document, catalog, &[b"OutputIntents"]);
    }
    if policy.remove_embedded_files {
        remove_catalog_names_entries(document, catalog, &[b"EmbeddedFiles"]);
    }
}

pub fn remove_unnecessary_objects(
    document: &mut Document,
    policy: &CleanupPolicy,
) -> AppResult<()> {
    remove_thumbnails_and_piece_info(document);
    remove_javascript(document);
    let remove_forms = should_remove_forms(document, policy);
    remove_annotations_with_subtypes(
        document,
        &removable_annotation_subtypes(policy, remove_forms),
    );
    remove_catalog_features(document, policy, remove_forms);
    if policy.remove_struct_tree {
        remove_struct_tree(document);
    }
    debug!("Unnecessary objects detached from the document");
    Ok(())
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;
    use crate::domain::level::CompressionLevel;
    use crate::test_support::{assert_no_dangling_references, PdfFixture};

    const ALL_LEVELS: [CompressionLevel; 4] = [
        CompressionLevel::Low,
        CompressionLevel::Medium,
        CompressionLevel::High,
        CompressionLevel::Extreme,
    ];

    fn run(document: &mut Document, level: CompressionLevel) {
        remove_unnecessary_objects(document, &level.preset().cleanup).unwrap();
    }

    fn page_id(document: &Document) -> ObjectId {
        *document.get_pages().get(&1).unwrap()
    }

    fn annotation_subtypes(document: &Document) -> Vec<String> {
        page_annotations(document, page_id(document))
            .iter()
            .filter_map(|annotation| annotation_subtype(document, annotation))
            .map(|subtype| String::from_utf8(subtype).unwrap())
            .collect()
    }

    fn catalog(document: &Document) -> &Dictionary {
        document.catalog().unwrap()
    }

    fn annotated_fixture() -> Document {
        PdfFixture::new()
            .page_with_content(b"q Q")
            .with_annotation("Link")
            .with_annotation("Text")
            .with_annotation("Highlight")
            .with_annotation("FileAttachment")
            .with_annotation("Screen")
            .with_acroform_widget()
            .build()
    }

    #[test]
    fn cleanup_low_keeps_markup_annotations() {
        let mut document = annotated_fixture();
        run(&mut document, CompressionLevel::Low);
        assert_eq!(
            annotation_subtypes(&document),
            vec![
                "Link",
                "Text",
                "Highlight",
                "FileAttachment",
                "Screen",
                "Widget"
            ]
        );
    }

    #[test]
    fn cleanup_high_removes_markup_annotations() {
        let mut document = annotated_fixture();
        run(&mut document, CompressionLevel::High);
        let subtypes = annotation_subtypes(&document);
        assert!(!subtypes.contains(&"Text".to_string()));
        assert!(!subtypes.contains(&"Highlight".to_string()));
        assert!(!subtypes.contains(&"FileAttachment".to_string()));
    }

    #[test]
    fn cleanup_high_keeps_link_and_widget_annotations() {
        let mut document = annotated_fixture();
        run(&mut document, CompressionLevel::High);
        assert_eq!(
            annotation_subtypes(&document),
            vec!["Link", "Screen", "Widget"]
        );
        assert!(catalog(&document).has(b"AcroForm"));
    }

    #[test]
    fn cleanup_extreme_removes_widgets_and_acroform() {
        let mut document = annotated_fixture();
        run(&mut document, CompressionLevel::Extreme);
        assert_eq!(annotation_subtypes(&document), vec!["Link", "Screen"]);
        assert!(!catalog(&document).has(b"AcroForm"));
    }

    #[test]
    fn cleanup_extreme_keeps_form_with_signature_field() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_acroform_widget()
            .with_signature_field()
            .build();
        run(&mut document, CompressionLevel::Extreme);
        assert!(catalog(&document).has(b"AcroForm"));
        assert_eq!(annotation_subtypes(&document), vec!["Widget", "Widget"]);
    }

    #[test]
    fn cleanup_detects_signature_field_inherited_through_kids() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_acroform_widget()
            .build();
        let kid = document.add_object(dictionary! { "T" => Object::string_literal("child") });
        let parent = document
            .add_object(dictionary! { "FT" => "Sig", "Kids" => vec![Object::Reference(kid)] });
        let catalog_object = catalog_id(&document).unwrap();
        if let Ok(Object::Dictionary(form)) = document
            .get_dictionary_mut(catalog_object)
            .unwrap()
            .get_mut(b"AcroForm")
        {
            if let Ok(Object::Array(fields)) = form.get_mut(b"Fields") {
                fields.push(Object::Reference(parent));
            }
        }
        run(&mut document, CompressionLevel::Extreme);
        assert!(catalog(&document).has(b"AcroForm"));
    }

    #[test]
    fn cleanup_medium_keeps_output_intents() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_output_intent()
            .build();
        run(&mut document, CompressionLevel::Medium);
        assert!(catalog(&document).has(b"OutputIntents"));
        run(&mut document, CompressionLevel::High);
        assert!(catalog(&document).has(b"OutputIntents"));
    }

    #[test]
    fn cleanup_extreme_removes_output_intents() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_output_intent()
            .build();
        run(&mut document, CompressionLevel::Extreme);
        assert!(!catalog(&document).has(b"OutputIntents"));
    }

    #[test]
    fn cleanup_never_removes_named_destinations() {
        for level in ALL_LEVELS {
            let mut document = PdfFixture::new()
                .page_with_content(b"q Q")
                .with_named_destination("chapter1")
                .build();
            run(&mut document, level);
            let names = catalog(&document).get(b"Names").unwrap().as_dict().unwrap();
            assert!(names.has(b"Dests"), "{level}");
        }
    }

    #[test]
    fn cleanup_removes_javascript_open_action_at_every_level() {
        for level in ALL_LEVELS {
            let mut document = PdfFixture::new()
                .page_with_content(b"q Q")
                .with_javascript_open_action()
                .build();
            run(&mut document, level);
            assert!(!catalog(&document).has(b"OpenAction"), "{level}");
        }
    }

    #[test]
    fn cleanup_keeps_non_javascript_open_action() {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let page = page_id(&document);
        let catalog_object = catalog_id(&document).unwrap();
        document.get_dictionary_mut(catalog_object).unwrap().set(
            "OpenAction",
            Object::Array(vec![Object::Reference(page), Object::Name(b"Fit".to_vec())]),
        );
        run(&mut document, CompressionLevel::Extreme);
        assert!(catalog(&document).has(b"OpenAction"));
    }

    #[test]
    fn cleanup_removes_javascript_names_tree_and_additional_actions() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_named_destination("d")
            .build();
        let catalog_object = catalog_id(&document).unwrap();
        let js_action = document
            .add_object(dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("x") });
        let chained = document.add_object(dictionary! {
            "S" => "GoTo",
            "Next" => vec![Object::Reference(js_action)],
        });
        let catalog_dict = document.get_dictionary_mut(catalog_object).unwrap();
        if let Ok(Object::Dictionary(names)) = catalog_dict.get_mut(b"Names") {
            names.set(
                "JavaScript",
                Object::Dictionary(dictionary! { "Names" => Vec::<Object>::new() }),
            );
        }
        catalog_dict.set("AA", Object::Dictionary(dictionary! { "WC" => chained }));
        run(&mut document, CompressionLevel::Low);
        let catalog_after = catalog(&document);
        assert!(!catalog_after.has(b"AA"));
        let names = catalog_after.get(b"Names").unwrap().as_dict().unwrap();
        assert!(!names.has(b"JavaScript"));
        assert!(names.has(b"Dests"));
    }

    #[test]
    fn cleanup_removes_javascript_annotation_actions_but_keeps_link_destination() {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let page = page_id(&document);
        let action = document
            .add_object(dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("x") });
        let annotation = document.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "A" => action,
            "Dest" => vec![Object::Reference(page), Object::Name(b"Fit".to_vec())],
        });
        document
            .get_dictionary_mut(page)
            .unwrap()
            .set("Annots", Object::Array(vec![Object::Reference(annotation)]));
        run(&mut document, CompressionLevel::Low);
        let annotation_dict = document.get_dictionary(annotation).unwrap();
        assert!(!annotation_dict.has(b"A"));
        assert!(annotation_dict.has(b"Dest"));
    }

    #[test]
    fn cleanup_removes_thumbnails_and_piece_info_at_every_level() {
        for level in ALL_LEVELS {
            let mut document = PdfFixture::new()
                .page_with_content(b"q Q")
                .page_with_content(b"BT ET")
                .with_thumbnail()
                .build();
            let first = page_id(&document);
            document
                .get_dictionary_mut(first)
                .unwrap()
                .set("PieceInfo", Object::Dictionary(Dictionary::new()));
            run(&mut document, level);
            for page in document.page_iter().collect::<Vec<_>>() {
                let dict = document.get_dictionary(page).unwrap();
                assert!(!dict.has(b"Thumb") && !dict.has(b"PieceInfo"), "{level}");
            }
        }
    }

    #[test]
    fn cleanup_removes_shared_thumbnail_reference_without_dangling_references() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .page_with_content(b"BT ET")
            .with_thumbnail()
            .build();
        run(&mut document, CompressionLevel::Low);
        document.prune_objects();
        assert_no_dangling_references(&document);
        assert!(!document.objects.values().any(
            |object| matches!(object, Object::Stream(stream) if stream.content == vec![7u8; 12])
        ));
    }

    #[test]
    fn cleanup_high_removes_struct_tree_and_embedded_files() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_struct_tree()
            .with_embedded_file()
            .build();
        run(&mut document, CompressionLevel::High);
        let catalog_after = catalog(&document);
        assert!(!catalog_after.has(b"StructTreeRoot") && !catalog_after.has(b"MarkInfo"));
        assert!(!catalog_after.has(b"Names"));
        assert!(!document
            .get_dictionary(page_id(&document))
            .unwrap()
            .has(b"StructParents"));
    }

    #[test]
    fn cleanup_medium_keeps_struct_tree_and_embedded_files() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_struct_tree()
            .with_embedded_file()
            .build();
        run(&mut document, CompressionLevel::Medium);
        let catalog_after = catalog(&document);
        assert!(catalog_after.has(b"StructTreeRoot") && catalog_after.has(b"MarkInfo"));
        assert!(catalog_after
            .get(b"Names")
            .unwrap()
            .as_dict()
            .unwrap()
            .has(b"EmbeddedFiles"));
    }

    #[test]
    fn cleanup_handles_annotations_stored_in_indirect_array() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_annotation("Text")
            .with_annotation("Link")
            .build();
        let page = page_id(&document);
        let entries = page_annotations(&document, page);
        let array_id = document.add_object(Object::Array(entries));
        document
            .get_dictionary_mut(page)
            .unwrap()
            .set("Annots", Object::Reference(array_id));
        run(&mut document, CompressionLevel::High);
        assert_eq!(annotation_subtypes(&document), vec!["Link"]);
        assert_eq!(
            document
                .get_dictionary(page)
                .unwrap()
                .get(b"Annots")
                .unwrap(),
            &Object::Reference(array_id)
        );
    }

    #[test]
    fn cleanup_drops_annots_key_when_every_annotation_removed() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_annotation("Text")
            .build();
        run(&mut document, CompressionLevel::High);
        assert!(!document
            .get_dictionary(page_id(&document))
            .unwrap()
            .has(b"Annots"));
    }

    #[test]
    fn cleanup_at_every_level_leaves_no_dangling_references_after_prune() {
        for level in ALL_LEVELS {
            let mut document = PdfFixture::new()
                .page_with_content(b"q Q")
                .with_annotation("Text")
                .with_annotation("Link")
                .with_acroform_widget()
                .with_output_intent()
                .with_struct_tree()
                .with_embedded_file()
                .with_named_destination("d")
                .with_thumbnail()
                .with_javascript_open_action()
                .build();
            run(&mut document, level);
            document.prune_objects();
            assert_no_dangling_references(&document);
        }
    }

    #[test]
    fn cleanup_on_document_without_catalog_features_is_a_no_op() {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let before = document.objects.len();
        run(&mut document, CompressionLevel::Extreme);
        assert_eq!(document.objects.len(), before);
    }
}
