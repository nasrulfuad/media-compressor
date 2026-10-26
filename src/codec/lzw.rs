use crate::error::{AppError, AppResult};

pub const LZW_CLEAR_CODE: u16 = 256;
pub const LZW_END_OF_DATA_CODE: u16 = 257;
pub const LZW_FIRST_DYNAMIC_CODE: u16 = 258;
pub const LZW_MAX_CODE_WIDTH: u32 = 12;
const LZW_INITIAL_CODE_WIDTH: u32 = 9;
const LZW_MAX_TABLE_ENTRIES: usize = (1 << LZW_MAX_CODE_WIDTH) - LZW_FIRST_DYNAMIC_CODE as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LzwSettings {
    pub early_change: bool,
    pub max_output_bytes: usize,
}

struct BitReader<'a> {
    data: &'a [u8],
    bit_position: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_position: 0,
        }
    }

    fn read(&mut self, width: u32) -> Option<u16> {
        let end = self.bit_position + width as usize;
        if end > self.data.len() * 8 {
            return None;
        }
        let mut value: u32 = 0;
        for position in self.bit_position..end {
            let byte = self.data[position / 8];
            let bit = (byte >> (7 - position % 8)) & 1;
            value = (value << 1) | u32::from(bit);
        }
        self.bit_position = end;
        Some(value as u16)
    }
}

struct Dictionary {
    entries: Vec<Vec<u8>>,
}

impl Dictionary {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn next_code(&self) -> usize {
        usize::from(LZW_FIRST_DYNAMIC_CODE) + self.entries.len()
    }

    fn is_full(&self) -> bool {
        self.entries.len() >= LZW_MAX_TABLE_ENTRIES
    }

    fn lookup(&self, code: u16) -> Option<Vec<u8>> {
        if code < LZW_CLEAR_CODE {
            return Some(vec![code as u8]);
        }
        let index = usize::from(code).checked_sub(usize::from(LZW_FIRST_DYNAMIC_CODE))?;
        self.entries.get(index).cloned()
    }

    fn add(&mut self, entry: Vec<u8>) {
        if !self.is_full() {
            self.entries.push(entry);
        }
    }
}

fn code_width_after_add(next_code: usize, early_change: bool, current_width: u32) -> u32 {
    let threshold = next_code + usize::from(early_change);
    if current_width < LZW_MAX_CODE_WIDTH && threshold >= (1usize << current_width) {
        current_width + 1
    } else {
        current_width
    }
}

fn invalid(message: &str) -> AppError {
    AppError::Decompress(format!("invalid LZW data: {message}"))
}

pub fn lzw_decode(data: &[u8], settings: LzwSettings) -> AppResult<Vec<u8>> {
    let mut reader = BitReader::new(data);
    let mut dictionary = Dictionary::new();
    let mut output: Vec<u8> = Vec::new();
    let mut width = LZW_INITIAL_CODE_WIDTH;
    let mut previous: Option<Vec<u8>> = None;

    while let Some(code) = reader.read(width) {
        if code == LZW_CLEAR_CODE {
            dictionary = Dictionary::new();
            width = LZW_INITIAL_CODE_WIDTH;
            previous = None;
            continue;
        }
        if code == LZW_END_OF_DATA_CODE {
            break;
        }
        let entry = match (&previous, dictionary.lookup(code)) {
            (_, Some(entry)) => entry,
            (Some(previous_entry), None) if usize::from(code) == dictionary.next_code() => {
                let mut entry = previous_entry.clone();
                entry.push(previous_entry[0]);
                entry
            }
            (None, None) => return Err(invalid("first code after clear is not a literal")),
            (Some(_), None) => return Err(invalid("code outside dictionary")),
        };
        if output.len() + entry.len() > settings.max_output_bytes {
            return Err(AppError::DecompressionLimitExceeded {
                limit: settings.max_output_bytes,
            });
        }
        output.extend_from_slice(&entry);
        if let Some(previous_entry) = &previous {
            let mut new_entry = previous_entry.clone();
            new_entry.push(entry[0]);
            dictionary.add(new_entry);
            width = code_width_after_add(dictionary.next_code(), settings.early_change, width);
        }
        previous = Some(entry);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const UNBOUNDED: usize = usize::MAX;

    fn settings(early_change: bool) -> LzwSettings {
        LzwSettings {
            early_change,
            max_output_bytes: UNBOUNDED,
        }
    }

    struct BitWriter {
        bytes: Vec<u8>,
        bit_count: usize,
    }

    impl BitWriter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                bit_count: 0,
            }
        }

        fn write(&mut self, value: u16, width: u32) {
            for shift in (0..width).rev() {
                if self.bit_count % 8 == 0 {
                    self.bytes.push(0);
                }
                let bit = ((value >> shift) & 1) as u8;
                let last = self.bytes.len() - 1;
                self.bytes[last] |= bit << (7 - self.bit_count % 8);
                self.bit_count += 1;
            }
        }
    }

    const ENCODER_RESET_THRESHOLD: usize = 4000;

    fn lzw_encode_for_test(data: &[u8], early_change: bool) -> Vec<u8> {
        let mut writer = BitWriter::new();
        let mut table: HashMap<Vec<u8>, u16> = HashMap::new();
        let mut next_code = usize::from(LZW_FIRST_DYNAMIC_CODE);
        let mut width = LZW_INITIAL_CODE_WIDTH;
        let mut emitted_since_clear = 0usize;
        writer.write(LZW_CLEAR_CODE, width);
        let mut current: Vec<u8> = Vec::new();

        let emit = |writer: &mut BitWriter, code: u16, width: &mut u32, emitted: &mut usize| {
            writer.write(code, *width);
            *emitted += 1;
            let decoder_next = usize::from(LZW_FIRST_DYNAMIC_CODE) + emitted.saturating_sub(1);
            *width = code_width_after_add(decoder_next, early_change, *width);
        };

        for &byte in data {
            let mut candidate = current.clone();
            candidate.push(byte);
            let known = candidate.len() == 1 || table.contains_key(&candidate);
            if known {
                current = candidate;
                continue;
            }
            let code = if current.len() == 1 {
                u16::from(current[0])
            } else {
                table[&current]
            };
            emit(&mut writer, code, &mut width, &mut emitted_since_clear);
            table.insert(candidate, next_code as u16);
            next_code += 1;
            current = vec![byte];
            if next_code >= ENCODER_RESET_THRESHOLD {
                writer.write(LZW_CLEAR_CODE, width);
                table.clear();
                next_code = usize::from(LZW_FIRST_DYNAMIC_CODE);
                width = LZW_INITIAL_CODE_WIDTH;
                emitted_since_clear = 0;
            }
        }
        if !current.is_empty() {
            let code = if current.len() == 1 {
                u16::from(current[0])
            } else {
                table[&current]
            };
            emit(&mut writer, code, &mut width, &mut emitted_since_clear);
        }
        writer.write(LZW_END_OF_DATA_CODE, width);
        writer.bytes
    }

    fn pseudo_random_bytes(count: usize, alphabet: u32) -> Vec<u8> {
        let mut state: u32 = 0x2545_F491;
        (0..count)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 16) % alphabet) as u8
            })
            .collect()
    }

    #[test]
    fn lzw_decode_matches_pdf_spec_example() {
        let encoded = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        let decoded = lzw_decode(&encoded, settings(true)).unwrap();
        assert_eq!(
            decoded,
            [0x2D, 0x2D, 0x2D, 0x2D, 0x2D, 0x41, 0x2D, 0x2D, 0x2D, 0x42]
        );
    }

    #[test]
    fn lzw_decode_round_trips_across_all_code_widths() {
        let data = pseudo_random_bytes(20_000, 5);
        for early_change in [true, false] {
            let encoded = lzw_encode_for_test(&data, early_change);
            let decoded = lzw_decode(&encoded, settings(early_change)).unwrap();
            assert_eq!(decoded, data, "early_change={early_change}");
        }
    }

    #[test]
    fn lzw_decode_resets_dictionary_on_clear_code() {
        let first = pseudo_random_bytes(6_000, 4);
        let encoded = lzw_encode_for_test(&first, true);
        let decoded = lzw_decode(&encoded, settings(true)).unwrap();
        assert_eq!(decoded.len(), first.len());
    }

    #[test]
    fn lzw_decode_rejects_invalid_first_code() {
        let mut writer = BitWriter::new();
        writer.write(LZW_CLEAR_CODE, LZW_INITIAL_CODE_WIDTH);
        writer.write(300, LZW_INITIAL_CODE_WIDTH);
        let error = lzw_decode(&writer.bytes, settings(true)).unwrap_err();
        assert!(matches!(error, AppError::Decompress(_)));
    }

    #[test]
    fn lzw_decode_stops_at_output_limit() {
        let data = vec![9u8; 5_000];
        let encoded = lzw_encode_for_test(&data, true);
        let limited = LzwSettings {
            early_change: true,
            max_output_bytes: 100,
        };
        let error = lzw_decode(&encoded, limited).unwrap_err();
        assert!(matches!(
            error,
            AppError::DecompressionLimitExceeded { limit: 100 }
        ));
    }

    #[test]
    fn lzw_decode_of_empty_input_is_empty() {
        assert_eq!(lzw_decode(&[], settings(true)).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn lzw_decode_stops_at_end_of_data_code() {
        let mut writer = BitWriter::new();
        writer.write(LZW_CLEAR_CODE, LZW_INITIAL_CODE_WIDTH);
        writer.write(u16::from(b'A'), LZW_INITIAL_CODE_WIDTH);
        writer.write(LZW_END_OF_DATA_CODE, LZW_INITIAL_CODE_WIDTH);
        writer.write(u16::from(b'B'), LZW_INITIAL_CODE_WIDTH);
        assert_eq!(lzw_decode(&writer.bytes, settings(true)).unwrap(), b"A");
    }
}
