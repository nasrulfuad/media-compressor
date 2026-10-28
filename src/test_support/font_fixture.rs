const TRUETYPE_VERSION: u32 = 0x0001_0000;
const HEAD_MAGIC_NUMBER: u32 = 0x5F0F_3CF5;
const HEAD_TABLE_BYTES: usize = 54;
const HEAD_MAGIC_OFFSET: usize = 12;
const HEAD_INDEX_TO_LOC_FORMAT_OFFSET: usize = 50;
const HHEA_TABLE_BYTES: usize = 36;
const HHEA_NUMBER_OF_METRICS_OFFSET: usize = 34;
const MAXP_VERSION_ONE_TAIL_BYTES: usize = 26;
const TABLE_DIRECTORY_HEADER_BYTES: usize = 12;
const TABLE_RECORD_BYTES: usize = 16;
const TABLE_ALIGNMENT: usize = 4;
const SIMPLE_GLYPH_CONTOURS: i16 = 1;
const GLYPH_OUTLINE_BYTES: usize = 64;
const ADVANCE_WIDTH: u16 = 500;
const SHORT_LOCA_DIVISOR: usize = 2;
const LCG_MULTIPLIER: u32 = 1_103_515_245;
const LCG_INCREMENT: u32 = 12_345;
const LCG_OUTPUT_SHIFT: u32 = 16;

fn push_u16(buffer: &mut Vec<u8>, value: u16) {
    buffer.extend_from_slice(&value.to_be_bytes());
}

fn push_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&value.to_be_bytes());
}

pub fn fixture_glyph_outline(glyph_id: u16) -> Vec<u8> {
    let mut outline = SIMPLE_GLYPH_CONTOURS.to_be_bytes().to_vec();
    let mut state = u32::from(glyph_id).wrapping_add(1);
    while outline.len() < GLYPH_OUTLINE_BYTES {
        state = state
            .wrapping_mul(LCG_MULTIPLIER)
            .wrapping_add(LCG_INCREMENT);
        outline.push((state >> LCG_OUTPUT_SHIFT) as u8);
    }
    outline
}

fn head_table() -> Vec<u8> {
    let mut head = vec![0u8; HEAD_TABLE_BYTES];
    head[..4].copy_from_slice(&TRUETYPE_VERSION.to_be_bytes());
    head[HEAD_MAGIC_OFFSET..HEAD_MAGIC_OFFSET + 4]
        .copy_from_slice(&HEAD_MAGIC_NUMBER.to_be_bytes());
    head
}

fn hhea_table(glyph_count: u16) -> Vec<u8> {
    let mut hhea = vec![0u8; HHEA_TABLE_BYTES];
    hhea[..4].copy_from_slice(&TRUETYPE_VERSION.to_be_bytes());
    hhea[HHEA_NUMBER_OF_METRICS_OFFSET..].copy_from_slice(&glyph_count.to_be_bytes());
    hhea
}

fn hmtx_table(glyph_count: u16) -> Vec<u8> {
    let mut hmtx = Vec::new();
    for _ in 0..glyph_count {
        push_u16(&mut hmtx, ADVANCE_WIDTH);
        push_u16(&mut hmtx, 0);
    }
    hmtx
}

fn maxp_table(glyph_count: u16) -> Vec<u8> {
    let mut maxp = Vec::new();
    push_u32(&mut maxp, TRUETYPE_VERSION);
    push_u16(&mut maxp, glyph_count);
    maxp.extend_from_slice(&[0u8; MAXP_VERSION_ONE_TAIL_BYTES]);
    maxp
}

fn glyf_and_loca_tables(glyph_count: u16) -> (Vec<u8>, Vec<u8>) {
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for glyph_id in 0..glyph_count {
        push_u16(&mut loca, (glyf.len() / SHORT_LOCA_DIVISOR) as u16);
        glyf.extend(fixture_glyph_outline(glyph_id));
    }
    push_u16(&mut loca, (glyf.len() / SHORT_LOCA_DIVISOR) as u16);
    (glyf, loca)
}

fn assemble_font(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let table_count = tables.len() as u16;
    let entry_selector = table_count.ilog2() as u16;
    let search_range = (1u16 << entry_selector) * TABLE_RECORD_BYTES as u16;
    let mut font = Vec::new();
    push_u32(&mut font, TRUETYPE_VERSION);
    push_u16(&mut font, table_count);
    push_u16(&mut font, search_range);
    push_u16(&mut font, entry_selector);
    push_u16(
        &mut font,
        table_count * TABLE_RECORD_BYTES as u16 - search_range,
    );
    let mut offset = TABLE_DIRECTORY_HEADER_BYTES + tables.len() * TABLE_RECORD_BYTES;
    for (tag, data) in tables {
        font.extend_from_slice(*tag);
        push_u32(&mut font, 0);
        push_u32(&mut font, offset as u32);
        push_u32(&mut font, data.len() as u32);
        offset += data.len().next_multiple_of(TABLE_ALIGNMENT);
    }
    for (_, data) in tables {
        font.extend_from_slice(data);
        font.resize(font.len().next_multiple_of(TABLE_ALIGNMENT), 0);
    }
    font
}

pub fn truetype_font(glyph_count: u16) -> Vec<u8> {
    let (glyf, loca) = glyf_and_loca_tables(glyph_count);
    assemble_font(&[
        (b"glyf", glyf),
        (b"head", head_table()),
        (b"hhea", hhea_table(glyph_count)),
        (b"hmtx", hmtx_table(glyph_count)),
        (b"loca", loca),
        (b"maxp", maxp_table(glyph_count)),
    ])
}

fn read_u16(data: &[u8], offset: usize) -> usize {
    usize::from(u16::from_be_bytes([data[offset], data[offset + 1]]))
}

fn read_u32(data: &[u8], offset: usize) -> usize {
    u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize
}

fn table_bytes<'a>(font: &'a [u8], tag: &[u8; 4]) -> &'a [u8] {
    let table_count = read_u16(font, 4);
    (0..table_count)
        .map(|index| TABLE_DIRECTORY_HEADER_BYTES + index * TABLE_RECORD_BYTES)
        .find(|record| &font[*record..*record + 4] == tag)
        .map(|record| {
            let offset = read_u32(font, record + 8);
            &font[offset..offset + read_u32(font, record + 12)]
        })
        .expect("font table must exist")
}

fn loca_offset(loca: &[u8], uses_long_offsets: bool, glyph_id: usize) -> usize {
    if uses_long_offsets {
        read_u32(loca, glyph_id * 4)
    } else {
        read_u16(loca, glyph_id * 2) * SHORT_LOCA_DIVISOR
    }
}

pub fn glyph_count(font: &[u8]) -> u16 {
    read_u16(table_bytes(font, b"maxp"), 4) as u16
}

pub fn glyph_outline(font: &[u8], glyph_id: u16) -> Vec<u8> {
    let uses_long_offsets =
        read_u16(table_bytes(font, b"head"), HEAD_INDEX_TO_LOC_FORMAT_OFFSET) != 0;
    let loca = table_bytes(font, b"loca");
    let glyph_id = usize::from(glyph_id);
    let start = loca_offset(loca, uses_long_offsets, glyph_id);
    let end = loca_offset(loca, uses_long_offsets, glyph_id + 1);
    table_bytes(font, b"glyf")[start..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truetype_font_starts_with_truetype_version() {
        assert_eq!(&truetype_font(4)[..4], &[0, 1, 0, 0]);
    }

    #[test]
    fn truetype_font_reports_requested_glyph_count() {
        assert_eq!(glyph_count(&truetype_font(7)), 7);
    }

    #[test]
    fn glyph_outline_returns_the_fixture_outline_of_each_glyph() {
        let font = truetype_font(5);
        assert_eq!(glyph_outline(&font, 0), fixture_glyph_outline(0));
        assert_eq!(glyph_outline(&font, 4), fixture_glyph_outline(4));
    }

    #[test]
    fn fixture_glyph_outlines_are_distinct_simple_glyphs() {
        let first = fixture_glyph_outline(1);
        let second = fixture_glyph_outline(2);
        assert_eq!(first.len(), GLYPH_OUTLINE_BYTES);
        assert_eq!(&first[..2], &SIMPLE_GLYPH_CONTOURS.to_be_bytes());
        assert_ne!(first, second);
    }

    #[test]
    fn truetype_font_is_accepted_by_the_subsetter() {
        let font = truetype_font(10);
        let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&[0, 3]);
        let subset = subsetter::subset(&font, 0, &remapper).unwrap();
        assert_eq!(glyph_count(&subset), 2);
        assert_eq!(glyph_outline(&subset, 1), fixture_glyph_outline(3));
    }
}
