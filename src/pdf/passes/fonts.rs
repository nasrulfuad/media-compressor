use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use subsetter::GlyphRemapper;
use tracing::{debug, warn};

use crate::codec::zlib::deflate;
use crate::domain::options::PdfOptions;
use crate::error::AppResult;
use crate::pdf::filters::{decode_plain_stream, replace_with_flate_bytes};
use crate::pdf::font_usage::collect_font_usage;
use crate::pdf::object_graph::{catalog_id, dict_name_is, resolve_array, resolve_dict};

const SUBSET_TAG_LENGTH: usize = 6;
const ALPHABET_SIZE: u64 = 26;
const CID_MAP_ENTRY_BYTES: usize = 2;
const NOTDEF_GLYPH: u16 = 0;
const IDENTITY_ENCODINGS: [&[u8]; 2] = [b"Identity-H", b"Identity-V"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontTarget {
    pub type0_id: ObjectId,
    pub descendant_id: ObjectId,
    pub descriptor_id: ObjectId,
    pub font_file_id: ObjectId,
}

pub fn is_subset_tagged(name: &[u8]) -> bool {
    name.len() > SUBSET_TAG_LENGTH
        && name[SUBSET_TAG_LENGTH] == b'+'
        && name[..SUBSET_TAG_LENGTH].iter().all(u8::is_ascii_uppercase)
}

pub fn subset_tag(glyphs: &[u16]) -> String {
    let mut hasher = DefaultHasher::new();
    glyphs.hash(&mut hasher);
    let mut value = hasher.finish();
    (0..SUBSET_TAG_LENGTH)
        .map(|_| {
            let letter = b'A' + (value % ALPHABET_SIZE) as u8;
            value /= ALPHABET_SIZE;
            letter as char
        })
        .collect()
}

pub fn build_cid_to_gid_map(glyphs: &[u16], remapper: &GlyphRemapper) -> Vec<u8> {
    let max_cid = glyphs.iter().copied().max().unwrap_or(NOTDEF_GLYPH);
    let mut map = vec![0u8; (usize::from(max_cid) + 1) * CID_MAP_ENTRY_BYTES];
    for cid in glyphs {
        let new_gid = remapper.get(*cid).unwrap_or(NOTDEF_GLYPH);
        let offset = usize::from(*cid) * CID_MAP_ENTRY_BYTES;
        map[offset..offset + CID_MAP_ENTRY_BYTES].copy_from_slice(&new_gid.to_be_bytes());
    }
    map
}

fn acroform_default_font_ids(document: &Document) -> HashSet<ObjectId> {
    let Some(catalog) = catalog_id(document).and_then(|id| document.get_dictionary(id).ok()) else {
        return HashSet::new();
    };
    let fonts = catalog
        .get(b"AcroForm")
        .ok()
        .and_then(|form| resolve_dict(document, form))
        .and_then(|form| form.get(b"DR").ok())
        .and_then(|resources| resolve_dict(document, resources))
        .and_then(|resources| resources.get(b"Font").ok())
        .and_then(|fonts| resolve_dict(document, fonts));
    fonts
        .map(|dict| {
            dict.iter()
                .filter_map(|(_, value)| value.as_reference().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn name_bytes<'a>(dict: &'a Dictionary, key: &[u8]) -> Option<&'a [u8]> {
    dict.get(key).ok().and_then(|value| value.as_name().ok())
}

fn single_descendant(document: &Document, type0: &Dictionary) -> Option<ObjectId> {
    let descendants = type0
        .get(b"DescendantFonts")
        .ok()
        .and_then(|value| resolve_array(document, value))?;
    match descendants.as_slice() {
        [Object::Reference(id)] => Some(*id),
        _ => None,
    }
}

fn has_identity_cid_map(descendant: &Dictionary) -> bool {
    match descendant.get(b"CIDToGIDMap") {
        Err(_) => true,
        Ok(value) => matches!(value, Object::Name(name) if name == b"Identity"),
    }
}

fn font_file_target(
    document: &Document,
    type0_id: ObjectId,
    descendant_id: ObjectId,
) -> Option<FontTarget> {
    let descendant = document.get_dictionary(descendant_id).ok()?;
    if !dict_name_is(descendant, b"Subtype", b"CIDFontType2") || !has_identity_cid_map(descendant) {
        return None;
    }
    let descriptor_id = descendant
        .get(b"FontDescriptor")
        .ok()?
        .as_reference()
        .ok()?;
    let descriptor = document.get_dictionary(descriptor_id).ok()?;
    let font_file_id = descriptor.get(b"FontFile2").ok()?.as_reference().ok()?;
    matches!(document.objects.get(&font_file_id), Some(Object::Stream(_))).then_some(FontTarget {
        type0_id,
        descendant_id,
        descriptor_id,
        font_file_id,
    })
}

pub fn eligible_target(
    document: &Document,
    type0_id: ObjectId,
    excluded: &HashSet<ObjectId>,
) -> Option<FontTarget> {
    let type0 = document.get_dictionary(type0_id).ok()?;
    if !dict_name_is(type0, b"Subtype", b"Type0") || excluded.contains(&type0_id) {
        return None;
    }
    let encoding = name_bytes(type0, b"Encoding")?;
    if !IDENTITY_ENCODINGS.contains(&encoding) {
        return None;
    }
    if name_bytes(type0, b"BaseFont").is_some_and(is_subset_tagged) {
        return None;
    }
    let descendant_id = single_descendant(document, type0)?;
    font_file_target(document, type0_id, descendant_id)
}

fn find_targets(document: &Document) -> Vec<FontTarget> {
    let excluded = acroform_default_font_ids(document);
    let targets: Vec<FontTarget> = document
        .objects
        .keys()
        .filter_map(|id| eligible_target(document, *id, &excluded))
        .collect();
    let mut file_counts: HashMap<ObjectId, usize> = HashMap::new();
    for target in &targets {
        *file_counts.entry(target.font_file_id).or_insert(0) += 1;
    }
    targets
        .into_iter()
        .filter(|target| {
            let unique = file_counts.get(&target.font_file_id) == Some(&1);
            if !unique {
                debug!(font = ?target.type0_id, reason = "font program shared", "Font subsetting skipped");
            }
            unique
        })
        .collect()
}

struct SubsetPlan {
    subset_program: Vec<u8>,
    stored_program: Vec<u8>,
    cid_map_stored: Vec<u8>,
    tag: String,
    old_stored_size: usize,
}

fn glyphs_to_keep(used: &BTreeSet<u16>) -> Vec<u16> {
    let mut glyphs: BTreeSet<u16> = used.clone();
    glyphs.insert(NOTDEF_GLYPH);
    glyphs.into_iter().collect()
}

fn plan_subset(
    document: &Document,
    target: &FontTarget,
    used: &BTreeSet<u16>,
    options: &PdfOptions,
) -> AppResult<Option<SubsetPlan>> {
    let Some(Object::Stream(font_file)) = document.objects.get(&target.font_file_id) else {
        return Ok(None);
    };
    let program = decode_plain_stream(font_file, options.limits.max_stream_bytes)?;
    let glyphs = glyphs_to_keep(used);
    let remapper = GlyphRemapper::new_from_glyphs_sorted(&glyphs);
    let subset_program = match subsetter::subset(&program, 0, &remapper) {
        Ok(subset) => subset,
        Err(error) => {
            warn!(font = ?target.type0_id, error = %error, "Font subsetting failed; font left untouched");
            return Ok(None);
        }
    };
    let level = options.level.preset().deflate_level;
    let stored_program = deflate(&subset_program, level)?;
    let cid_map_stored = deflate(&build_cid_to_gid_map(&glyphs, &remapper), level)?;
    let plan = SubsetPlan {
        subset_program,
        stored_program,
        cid_map_stored,
        tag: subset_tag(&glyphs),
        old_stored_size: font_file.content.len(),
    };
    let new_size = plan.stored_program.len() + plan.cid_map_stored.len();
    Ok((new_size < plan.old_stored_size).then_some(plan))
}

fn tagged_name(tag: &str, original: &[u8]) -> Object {
    let mut name = format!("{tag}+").into_bytes();
    name.extend_from_slice(original);
    Object::Name(name)
}

fn apply_tag(document: &mut Document, id: ObjectId, key: &[u8], tag: &str) {
    let Ok(dict) = document.get_dictionary_mut(id) else {
        return;
    };
    let Some(original) = name_bytes(dict, key).map(<[u8]>::to_vec) else {
        return;
    };
    if !is_subset_tagged(&original) {
        dict.set(key.to_vec(), tagged_name(tag, &original));
    }
}

fn apply_plan(document: &mut Document, target: &FontTarget, plan: SubsetPlan) {
    if let Some(Object::Stream(font_file)) = document.objects.get_mut(&target.font_file_id) {
        replace_with_flate_bytes(font_file, plan.stored_program);
        font_file
            .dict
            .set("Length1", Object::Integer(plan.subset_program.len() as i64));
    }
    let mut cid_map = Stream::new(Dictionary::new(), Vec::new());
    replace_with_flate_bytes(&mut cid_map, plan.cid_map_stored);
    let cid_map_id = document.add_object(Object::Stream(cid_map));
    if let Ok(descendant) = document.get_dictionary_mut(target.descendant_id) {
        descendant.set("CIDToGIDMap", Object::Reference(cid_map_id));
    }
    apply_tag(document, target.type0_id, b"BaseFont", &plan.tag);
    apply_tag(document, target.descendant_id, b"BaseFont", &plan.tag);
    apply_tag(document, target.descriptor_id, b"FontName", &plan.tag);
}

pub fn subset_fonts(document: &mut Document, options: &PdfOptions) -> AppResult<i64> {
    let usage = collect_font_usage(document);
    if !usage.is_complete {
        warn!(
            "Font usage could not be determined for every content stream; font subsetting skipped"
        );
        return Ok(0);
    }
    let mut total_saved = 0i64;
    for target in find_targets(document) {
        let Some(used) = usage.cids_by_font.get(&target.type0_id) else {
            debug!(font = ?target.type0_id, reason = "no recorded usage", "Font subsetting skipped");
            continue;
        };
        let plan = match plan_subset(document, &target, used, options) {
            Ok(Some(plan)) => plan,
            Ok(None) => continue,
            Err(error) => {
                warn!(font = ?target.type0_id, error = %error, "Font subsetting skipped");
                continue;
            }
        };
        total_saved += plan.old_stored_size as i64
            - (plan.stored_program.len() + plan.cid_map_stored.len()) as i64;
        apply_plan(document, &target, plan);
    }
    Ok(total_saved)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use lopdf::dictionary;

    use super::*;
    use crate::domain::level::CompressionLevel;
    use crate::test_support::PdfFixture;

    const TEST_FONT_PATH: &str = "tests/fixtures/fonts/test-font.ttf";
    const FAKE_PROGRAM: &[u8] = b"not-a-real-font-program";

    fn options() -> PdfOptions {
        PdfOptions::for_level(CompressionLevel::Medium)
    }

    fn type0_id(document: &Document) -> ObjectId {
        document
            .objects
            .iter()
            .find_map(|(id, object)| match object {
                Object::Dictionary(dict) if dict_name_is(dict, b"Subtype", b"Type0") => Some(*id),
                _ => None,
            })
            .expect("type0 font")
    }

    fn cid_document(program: &[u8], content: &[u8]) -> Document {
        PdfFixture::new()
            .page_with_content(content)
            .with_identity_cid_font(program)
            .build()
    }

    fn target_of(document: &Document) -> Option<FontTarget> {
        eligible_target(document, type0_id(document), &HashSet::new())
    }

    #[test]
    fn subset_tag_is_six_uppercase_letters_and_deterministic() {
        let tag = subset_tag(&[0, 3, 9]);
        assert_eq!(tag.len(), 6);
        assert!(tag.chars().all(|letter| letter.is_ascii_uppercase()));
        assert_eq!(tag, subset_tag(&[0, 3, 9]));
        assert_ne!(tag, subset_tag(&[0, 3, 10]));
    }

    #[test]
    fn is_subset_tagged_requires_six_uppercase_letters_and_plus() {
        assert!(is_subset_tagged(b"ABCDEF+Arial"));
        assert!(!is_subset_tagged(b"Arial"));
        assert!(!is_subset_tagged(b"abcdef+Arial"));
        assert!(!is_subset_tagged(b"ABCDE+Arial"));
        assert!(!is_subset_tagged(b"ABCDEF"));
    }

    #[test]
    fn build_cid_to_gid_map_maps_old_glyphs_to_new_glyphs() {
        let glyphs = [0u16, 5, 9];
        let remapper = GlyphRemapper::new_from_glyphs_sorted(&glyphs);
        let map = build_cid_to_gid_map(&glyphs, &remapper);
        assert_eq!(map.len(), 20);
        assert_eq!(&map[10..12], &[0, 1]);
        assert_eq!(&map[18..20], &[0, 2]);
        assert_eq!(&map[2..4], &[0, 0]);
    }

    #[test]
    fn font_eligibility_accepts_identity_cid_truetype_font() {
        let document = cid_document(FAKE_PROGRAM, b"BT /F1 12 Tf <0024> Tj ET");
        let target = target_of(&document).expect("eligible");
        assert_eq!(target.type0_id, type0_id(&document));
    }

    #[test]
    fn font_eligibility_rejects_simple_truetype() {
        let mut document = cid_document(FAKE_PROGRAM, b"q Q");
        let id = type0_id(&document);
        document
            .get_dictionary_mut(id)
            .unwrap()
            .set("Subtype", Object::Name(b"TrueType".to_vec()));
        assert!(eligible_target(&document, id, &HashSet::new()).is_none());
    }

    #[test]
    fn font_eligibility_rejects_non_identity_encoding() {
        let mut document = cid_document(FAKE_PROGRAM, b"q Q");
        let id = type0_id(&document);
        document
            .get_dictionary_mut(id)
            .unwrap()
            .set("Encoding", Object::Name(b"UniJIS-UCS2-H".to_vec()));
        assert!(target_of(&document).is_none());
    }

    #[test]
    fn font_eligibility_rejects_non_identity_cid_to_gid_map() {
        let mut document = cid_document(FAKE_PROGRAM, b"q Q");
        let target = target_of(&document).unwrap();
        document
            .get_dictionary_mut(target.descendant_id)
            .unwrap()
            .set("CIDToGIDMap", Object::Reference((999, 0)));
        assert!(target_of(&document).is_none());
    }

    #[test]
    fn font_eligibility_rejects_already_subset_tagged_font() {
        let mut document = cid_document(FAKE_PROGRAM, b"q Q");
        let id = type0_id(&document);
        document
            .get_dictionary_mut(id)
            .unwrap()
            .set("BaseFont", Object::Name(b"ABCDEF+Test".to_vec()));
        assert!(target_of(&document).is_none());
    }

    #[test]
    fn font_eligibility_rejects_cff_descendant() {
        let mut document = cid_document(FAKE_PROGRAM, b"q Q");
        let target = target_of(&document).unwrap();
        document
            .get_dictionary_mut(target.descendant_id)
            .unwrap()
            .set("Subtype", Object::Name(b"CIDFontType0".to_vec()));
        assert!(target_of(&document).is_none());
    }

    #[test]
    fn font_eligibility_rejects_font_in_acroform_default_resources() {
        let mut document = cid_document(FAKE_PROGRAM, b"q Q");
        let id = type0_id(&document);
        let catalog = catalog_id(&document).unwrap();
        document.get_dictionary_mut(catalog).unwrap().set(
            "AcroForm",
            Object::Dictionary(dictionary! {
                "Fields" => Vec::<Object>::new(),
                "DR" => dictionary! { "Font" => dictionary! { "F1" => id } },
            }),
        );
        assert!(acroform_default_font_ids(&document).contains(&id));
        assert!(find_targets(&document).is_empty());
    }

    #[test]
    fn find_targets_skips_fonts_sharing_one_font_program() {
        let mut document = cid_document(FAKE_PROGRAM, b"q Q");
        let first = target_of(&document).unwrap();
        let second_descendant =
            document.add_object(document.get_object(first.descendant_id).unwrap().clone());
        let second_type0 = document.add_object(Object::Dictionary({
            let mut dict = document.get_dictionary(first.type0_id).unwrap().clone();
            dict.set(
                "DescendantFonts",
                Object::Array(vec![Object::Reference(second_descendant)]),
            );
            dict
        }));
        assert!(eligible_target(&document, second_type0, &HashSet::new()).is_some());
        assert!(find_targets(&document).is_empty());
    }

    #[test]
    fn subset_fonts_skips_everything_when_usage_is_incomplete() {
        let mut document = cid_document(FAKE_PROGRAM, b"BT /F1 12 Tf (unterminated Tj ET");
        let before = document.objects.len();
        assert_eq!(subset_fonts(&mut document, &options()).unwrap(), 0);
        assert_eq!(document.objects.len(), before);
    }

    #[test]
    fn subset_fonts_leaves_unused_font_untouched() {
        let mut document = cid_document(FAKE_PROGRAM, b"q Q");
        let before = document.objects.len();
        assert_eq!(subset_fonts(&mut document, &options()).unwrap(), 0);
        assert_eq!(document.objects.len(), before);
    }

    #[test]
    fn subset_fonts_leaves_font_untouched_when_program_is_not_a_font() {
        let mut document = cid_document(FAKE_PROGRAM, b"BT /F1 12 Tf <0024> Tj ET");
        let target = target_of(&document).unwrap();
        let before = document.get_object(target.font_file_id).unwrap().clone();
        assert_eq!(subset_fonts(&mut document, &options()).unwrap(), 0);
        assert_eq!(document.get_object(target.font_file_id).unwrap(), &before);
    }

    #[test]
    fn subset_fonts_on_document_without_fonts_returns_zero() {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        assert_eq!(subset_fonts(&mut document, &options()).unwrap(), 0);
    }

    #[test]
    #[ignore = "requires tests/fixtures/fonts/test-font.ttf"]
    fn subset_fonts_shrinks_font_and_keeps_used_glyph_outlines() {
        assert!(
            Path::new(TEST_FONT_PATH).exists(),
            "{TEST_FONT_PATH} is missing"
        );
        let program = std::fs::read(TEST_FONT_PATH).unwrap();
        let mut document = cid_document(&program, b"BT /F1 12 Tf <002400250026> Tj ET");
        let target = target_of(&document).unwrap();
        let old_stored = match document.get_object(target.font_file_id).unwrap() {
            Object::Stream(stream) => stream.content.len(),
            _ => unreachable!(),
        };
        let saved = subset_fonts(&mut document, &options()).unwrap();
        assert!(saved > 0, "saved {saved}");
        let font_file = document
            .get_object(target.font_file_id)
            .unwrap()
            .as_stream()
            .unwrap();
        assert!(font_file.content.len() < old_stored);
        let inflated = decode_plain_stream(font_file, usize::MAX).unwrap();
        assert_eq!(
            font_file.dict.get(b"Length1").unwrap(),
            &Object::Integer(inflated.len() as i64)
        );
        let descendant = document.get_dictionary(target.descendant_id).unwrap();
        let map_id = descendant
            .get(b"CIDToGIDMap")
            .unwrap()
            .as_reference()
            .unwrap();
        let map = decode_plain_stream(
            document.get_object(map_id).unwrap().as_stream().unwrap(),
            usize::MAX,
        )
        .unwrap();
        let entries: Vec<u16> = [0x24usize, 0x25, 0x26]
            .iter()
            .map(|cid| u16::from_be_bytes([map[cid * 2], map[cid * 2 + 1]]))
            .collect();
        assert!(entries.iter().all(|gid| *gid != 0));
        let distinct: BTreeSet<u16> = entries.iter().copied().collect();
        assert_eq!(distinct.len(), 3);
        let base_font = document
            .get_dictionary(target.type0_id)
            .unwrap()
            .get(b"BaseFont")
            .unwrap()
            .as_name()
            .unwrap();
        assert!(is_subset_tagged(base_font));
    }
}
