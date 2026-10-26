use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Number,
    Name,
    LiteralString,
    HexString,
    ArrayStart,
    ArrayEnd,
    DictStart,
    DictEnd,
    Keyword,
    InlineImage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: TokenKind,
    pub raw: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineImage<'a> {
    pub dictionary: Vec<Token<'a>>,
    pub data: &'a [u8],
}

const INLINE_IMAGE_BEGIN: &[u8] = b"BI";
const INLINE_IMAGE_DATA: &[u8] = b"ID";
const INLINE_IMAGE_END: &[u8] = b"EI";
const BITS_PER_BYTE: usize = 8;

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, 0x00 | 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn is_regular(byte: u8) -> bool {
    !is_whitespace(byte) && !is_delimiter(byte)
}

fn invalid(message: &str) -> AppError {
    AppError::InvalidContentStream(message.to_string())
}

fn is_number(raw: &[u8]) -> bool {
    let digits = match raw.first() {
        Some(b'+') | Some(b'-') => &raw[1..],
        _ => raw,
    };
    let dot_count = digits.iter().filter(|byte| **byte == b'.').count();
    let digit_count = digits.iter().filter(|byte| byte.is_ascii_digit()).count();
    dot_count <= 1 && digit_count > 0 && digit_count + dot_count == digits.len()
}

struct Lexer<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Lexer<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    fn peek(&self, offset: usize) -> Option<u8> {
        self.data.get(self.position + offset).copied()
    }

    fn skip_whitespace_and_comments(&mut self) {
        while let Some(byte) = self.peek(0) {
            if is_whitespace(byte) {
                self.position += 1;
            } else if byte == b'%' {
                while let Some(comment_byte) = self.peek(0) {
                    if comment_byte == b'\r' || comment_byte == b'\n' {
                        break;
                    }
                    self.position += 1;
                }
            } else {
                break;
            }
        }
    }

    fn token(&self, kind: TokenKind, start: usize) -> Token<'a> {
        Token {
            kind,
            raw: &self.data[start..self.position],
        }
    }

    fn read_literal_string(&mut self) -> AppResult<Token<'a>> {
        let start = self.position;
        let mut depth = 0usize;
        while let Some(byte) = self.peek(0) {
            self.position += 1;
            match byte {
                b'\\' => self.position += 1,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(self.token(TokenKind::LiteralString, start));
                    }
                }
                _ => {}
            }
        }
        Err(invalid("unterminated literal string"))
    }

    fn read_hex_string(&mut self) -> AppResult<Token<'a>> {
        let start = self.position;
        self.position += 1;
        while let Some(byte) = self.peek(0) {
            self.position += 1;
            if byte == b'>' {
                return Ok(self.token(TokenKind::HexString, start));
            }
            if !byte.is_ascii_hexdigit() && !is_whitespace(byte) {
                return Err(invalid("non-hexadecimal byte inside hex string"));
            }
        }
        Err(invalid("unterminated hex string"))
    }

    fn read_regular_run(&mut self) -> Token<'a> {
        let start = self.position;
        while self.peek(0).is_some_and(is_regular) {
            self.position += 1;
        }
        let raw = &self.data[start..self.position];
        let kind = if is_number(raw) {
            TokenKind::Number
        } else {
            TokenKind::Keyword
        };
        Token { kind, raw }
    }

    fn read_name(&mut self) -> Token<'a> {
        let start = self.position;
        self.position += 1;
        while self.peek(0).is_some_and(is_regular) {
            self.position += 1;
        }
        self.token(TokenKind::Name, start)
    }

    fn single_byte(&mut self, kind: TokenKind) -> Token<'a> {
        let start = self.position;
        self.position += 1;
        self.token(kind, start)
    }

    fn next_plain_token(&mut self) -> AppResult<Option<Token<'a>>> {
        self.skip_whitespace_and_comments();
        let Some(byte) = self.peek(0) else {
            return Ok(None);
        };
        let token = match byte {
            b'(' => self.read_literal_string()?,
            b'<' if self.peek(1) == Some(b'<') => {
                let start = self.position;
                self.position += 2;
                self.token(TokenKind::DictStart, start)
            }
            b'<' => self.read_hex_string()?,
            b'>' if self.peek(1) == Some(b'>') => {
                let start = self.position;
                self.position += 2;
                self.token(TokenKind::DictEnd, start)
            }
            b'>' | b')' => return Err(invalid("unexpected closing delimiter")),
            b'[' => self.single_byte(TokenKind::ArrayStart),
            b']' => self.single_byte(TokenKind::ArrayEnd),
            b'{' | b'}' => self.single_byte(TokenKind::Keyword),
            b'/' => self.read_name(),
            _ => self.read_regular_run(),
        };
        Ok(Some(token))
    }

    fn next_token(&mut self) -> AppResult<Option<Token<'a>>> {
        let Some(token) = self.next_plain_token()? else {
            return Ok(None);
        };
        if token.kind == TokenKind::Keyword && token.raw == INLINE_IMAGE_BEGIN {
            let start = self.position - INLINE_IMAGE_BEGIN.len();
            self.read_inline_image()?;
            return Ok(Some(self.token(TokenKind::InlineImage, start)));
        }
        Ok(Some(token))
    }

    fn read_inline_image(&mut self) -> AppResult<InlineImage<'a>> {
        let mut dictionary = Vec::new();
        loop {
            let token = self
                .next_plain_token()?
                .ok_or_else(|| invalid("inline image without ID marker"))?;
            if token.kind == TokenKind::Keyword && token.raw == INLINE_IMAGE_DATA {
                break;
            }
            dictionary.push(token);
        }
        if self.peek(0).is_some_and(is_whitespace) {
            self.position += 1;
        }
        let data_start = self.position;
        let data_end = self.find_inline_data_end(&dictionary, data_start)?;
        let image = InlineImage {
            dictionary,
            data: &self.data[data_start..data_end],
        };
        self.position = data_end;
        self.skip_end_marker()?;
        Ok(image)
    }

    fn skip_end_marker(&mut self) -> AppResult<()> {
        while self.peek(0).is_some_and(is_whitespace) {
            self.position += 1;
        }
        if self.data[self.position..].starts_with(INLINE_IMAGE_END) {
            self.position += INLINE_IMAGE_END.len();
            return Ok(());
        }
        Err(invalid("inline image without EI marker"))
    }

    fn find_inline_data_end(
        &self,
        dictionary: &[Token<'a>],
        data_start: usize,
    ) -> AppResult<usize> {
        if let Some(length) = expected_unfiltered_length(dictionary) {
            let candidate_end = data_start.saturating_add(length);
            if self.is_end_marker_after(candidate_end) {
                return Ok(candidate_end);
            }
        }
        self.scan_for_end_marker(data_start)
    }

    fn is_end_marker_after(&self, position: usize) -> bool {
        let Some(rest) = self.data.get(position..) else {
            return false;
        };
        let after_whitespace = rest
            .iter()
            .position(|byte| !is_whitespace(*byte))
            .unwrap_or(rest.len());
        rest[after_whitespace..].starts_with(INLINE_IMAGE_END)
            && self.is_boundary_after_end_marker(position + after_whitespace)
    }

    fn is_boundary_after_end_marker(&self, marker_start: usize) -> bool {
        match self.data.get(marker_start + INLINE_IMAGE_END.len()) {
            None => true,
            Some(byte) => is_whitespace(*byte) || is_delimiter(*byte),
        }
    }

    fn scan_for_end_marker(&self, data_start: usize) -> AppResult<usize> {
        let mut index = data_start;
        while index <= self.data.len() {
            let at_boundary = index == data_start
                || self
                    .data
                    .get(index - 1)
                    .is_some_and(|byte| is_whitespace(*byte));
            if at_boundary
                && self.data[index..].starts_with(INLINE_IMAGE_END)
                && self.is_boundary_after_end_marker(index)
            {
                let data_end = if index > data_start { index - 1 } else { index };
                return Ok(data_end);
            }
            index += 1;
        }
        Err(invalid("inline image without EI marker"))
    }
}

fn dictionary_value<'a>(dictionary: &[Token<'a>], keys: &[&[u8]]) -> Option<Vec<Token<'a>>> {
    let mut index = 0;
    while index < dictionary.len() {
        let key = dictionary[index];
        if key.kind == TokenKind::Name && keys.iter().any(|candidate| &key.raw[1..] == *candidate) {
            return value_tokens(&dictionary[index + 1..]);
        }
        index += 1;
    }
    None
}

fn value_tokens<'a>(tokens: &[Token<'a>]) -> Option<Vec<Token<'a>>> {
    let first = *tokens.first()?;
    if first.kind != TokenKind::ArrayStart {
        return Some(vec![first]);
    }
    let end = tokens
        .iter()
        .position(|token| token.kind == TokenKind::ArrayEnd)?;
    Some(tokens[..=end].to_vec())
}

fn parse_integer(tokens: &[Token<'_>]) -> Option<usize> {
    let token = tokens.first()?;
    std::str::from_utf8(token.raw).ok()?.parse().ok()
}

fn component_count(value: &[Token<'_>]) -> Option<usize> {
    let first = value.first()?;
    match first.raw {
        b"/G" | b"/DeviceGray" | b"/CalGray" => Some(1),
        b"/RGB" | b"/DeviceRGB" | b"/CalRGB" => Some(3),
        b"/CMYK" | b"/DeviceCMYK" => Some(4),
        b"[" => match value.get(1).map(|token| token.raw) {
            Some(b"/I") | Some(b"/Indexed") => Some(1),
            _ => None,
        },
        _ => None,
    }
}

fn is_image_mask(dictionary: &[Token<'_>]) -> bool {
    dictionary_value(dictionary, &[b"IM", b"ImageMask"])
        .and_then(|value| value.first().map(|token| token.raw == b"true"))
        .unwrap_or(false)
}

fn expected_unfiltered_length(dictionary: &[Token<'_>]) -> Option<usize> {
    if dictionary_value(dictionary, &[b"F", b"Filter"]).is_some() {
        return None;
    }
    let width = parse_integer(&dictionary_value(dictionary, &[b"W", b"Width"])?)?;
    let height = parse_integer(&dictionary_value(dictionary, &[b"H", b"Height"])?)?;
    let (components, bits) = if is_image_mask(dictionary) {
        (1, 1)
    } else {
        let components = component_count(&dictionary_value(dictionary, &[b"CS", b"ColorSpace"])?)?;
        let bits = parse_integer(&dictionary_value(
            dictionary,
            &[b"BPC", b"BitsPerComponent"],
        )?)?;
        (components, bits)
    };
    let row_bits = width.checked_mul(components)?.checked_mul(bits)?;
    row_bits.div_ceil(BITS_PER_BYTE).checked_mul(height)
}

pub fn tokenize(data: &[u8]) -> AppResult<Vec<Token<'_>>> {
    let mut lexer = Lexer::new(data);
    let mut tokens = Vec::new();
    while let Some(token) = lexer.next_token()? {
        tokens.push(token);
    }
    Ok(tokens)
}

pub fn parse_inline_image(raw: &[u8]) -> AppResult<InlineImage<'_>> {
    let mut lexer = Lexer::new(raw);
    lexer
        .next_plain_token()?
        .filter(|token| token.kind == TokenKind::Keyword && token.raw == INLINE_IMAGE_BEGIN)
        .ok_or_else(|| invalid("inline image must start with BI"))?;
    lexer.read_inline_image()
}

fn octal_value(bytes: &[u8]) -> (u8, usize) {
    let digits = bytes
        .iter()
        .take(3)
        .take_while(|byte| (b'0'..=b'7').contains(byte))
        .count();
    let value = bytes[..digits].iter().fold(0u32, |accumulator, digit| {
        accumulator * 8 + u32::from(digit - b'0')
    });
    ((value & 0xFF) as u8, digits)
}

fn escape_replacement(byte: u8) -> Option<u8> {
    match byte {
        b'n' => Some(b'\n'),
        b'r' => Some(b'\r'),
        b't' => Some(b'\t'),
        b'b' => Some(0x08),
        b'f' => Some(0x0C),
        b'(' | b')' | b'\\' => Some(byte),
        _ => None,
    }
}

pub fn decode_literal_string(raw: &[u8]) -> Vec<u8> {
    let inner = raw.strip_prefix(b"(").unwrap_or(raw);
    let inner = inner.strip_suffix(b")").unwrap_or(inner);
    let mut decoded = Vec::with_capacity(inner.len());
    let mut index = 0;
    while index < inner.len() {
        let byte = inner[index];
        index += 1;
        if byte != b'\\' {
            decoded.push(byte);
            continue;
        }
        let Some(&escaped) = inner.get(index) else {
            break;
        };
        if (b'0'..=b'7').contains(&escaped) {
            let (value, consumed) = octal_value(&inner[index..]);
            decoded.push(value);
            index += consumed;
        } else if escaped == b'\r' {
            index += 1;
            if inner.get(index) == Some(&b'\n') {
                index += 1;
            }
        } else if escaped == b'\n' {
            index += 1;
        } else {
            decoded.push(escape_replacement(escaped).unwrap_or(escaped));
            index += 1;
        }
    }
    decoded
}

fn hex_digit_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => byte - b'A' + 10,
    }
}

pub fn decode_hex_string(raw: &[u8]) -> Vec<u8> {
    let inner = raw.strip_prefix(b"<").unwrap_or(raw);
    let inner = inner.strip_suffix(b">").unwrap_or(inner);
    let digits: Vec<u8> = inner
        .iter()
        .filter(|byte| byte.is_ascii_hexdigit())
        .map(|byte| hex_digit_value(*byte))
        .collect();
    digits
        .chunks(2)
        .map(|pair| (pair[0] << 4) | pair.get(1).copied().unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds_and_raw(data: &[u8]) -> Vec<(TokenKind, String)> {
        tokenize(data)
            .unwrap()
            .into_iter()
            .map(|token| (token.kind, String::from_utf8_lossy(token.raw).into_owned()))
            .collect()
    }

    fn token(kind: TokenKind, raw: &str) -> (TokenKind, String) {
        (kind, raw.to_string())
    }

    #[test]
    fn tokenize_keeps_percent_inside_literal_string() {
        assert_eq!(
            kinds_and_raw(b"(50%) Tj"),
            vec![
                token(TokenKind::LiteralString, "(50%)"),
                token(TokenKind::Keyword, "Tj")
            ]
        );
    }

    #[test]
    fn tokenize_skips_comment_until_end_of_line() {
        assert_eq!(
            kinds_and_raw(b"q % comment (not a string\nQ"),
            vec![
                token(TokenKind::Keyword, "q"),
                token(TokenKind::Keyword, "Q")
            ]
        );
    }

    #[test]
    fn tokenize_comment_ending_at_carriage_return_resumes_after_it() {
        assert_eq!(
            kinds_and_raw(b"q %c\rQ"),
            vec![
                token(TokenKind::Keyword, "q"),
                token(TokenKind::Keyword, "Q")
            ]
        );
    }

    #[test]
    fn tokenize_handles_nested_and_escaped_parentheses() {
        assert_eq!(
            kinds_and_raw(b"(a(b)c\\)d) Tj"),
            vec![
                token(TokenKind::LiteralString, "(a(b)c\\)d)"),
                token(TokenKind::Keyword, "Tj")
            ]
        );
    }

    #[test]
    fn tokenize_distinguishes_dict_start_from_hex_string() {
        assert_eq!(
            kinds_and_raw(b"<< /A <48 65> >>"),
            vec![
                token(TokenKind::DictStart, "<<"),
                token(TokenKind::Name, "/A"),
                token(TokenKind::HexString, "<48 65>"),
                token(TokenKind::DictEnd, ">>"),
            ]
        );
    }

    #[test]
    fn tokenize_reads_names_numbers_and_keywords() {
        assert_eq!(
            kinds_and_raw(b"/F1 12 Tf 1.5 -0.25 .5 TL"),
            vec![
                token(TokenKind::Name, "/F1"),
                token(TokenKind::Number, "12"),
                token(TokenKind::Keyword, "Tf"),
                token(TokenKind::Number, "1.5"),
                token(TokenKind::Number, "-0.25"),
                token(TokenKind::Number, ".5"),
                token(TokenKind::Keyword, "TL"),
            ]
        );
    }

    #[test]
    fn tokenize_reads_arrays_and_braces() {
        assert_eq!(
            kinds_and_raw(b"[(a) 2]{x}"),
            vec![
                token(TokenKind::ArrayStart, "["),
                token(TokenKind::LiteralString, "(a)"),
                token(TokenKind::Number, "2"),
                token(TokenKind::ArrayEnd, "]"),
                token(TokenKind::Keyword, "{"),
                token(TokenKind::Keyword, "x"),
                token(TokenKind::Keyword, "}"),
            ]
        );
    }

    #[test]
    fn tokenize_treats_sign_only_and_double_dot_runs_as_keywords() {
        assert_eq!(
            kinds_and_raw(b"- 1.2.3 +"),
            vec![
                token(TokenKind::Keyword, "-"),
                token(TokenKind::Keyword, "1.2.3"),
                token(TokenKind::Keyword, "+"),
            ]
        );
    }

    #[test]
    fn tokenize_emits_inline_image_as_single_token_with_binary_data() {
        let mut stream = b"q BI /W 2 /H 2 /CS /G /BPC 8 /F /Fl ID ".to_vec();
        stream.extend_from_slice(&[0x00, 0x0A, b'E', b'I', 0xFF]);
        stream.extend_from_slice(b"\nEI Q");
        let tokens = tokenize(&stream).unwrap();
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[1].kind, TokenKind::InlineImage);
        assert!(tokens[1].raw.starts_with(b"BI"));
        assert!(tokens[1].raw.ends_with(b"EI"));
        assert_eq!(tokens[2].raw, b"Q");
        let image = parse_inline_image(tokens[1].raw).unwrap();
        assert_eq!(image.data, &[0x00, 0x0A, b'E', b'I', 0xFF]);
    }

    #[test]
    fn tokenize_uses_expected_length_for_unfiltered_inline_image() {
        let mut stream = b"BI /W 4 /H 1 /CS /G /BPC 8 ID ".to_vec();
        stream.extend_from_slice(b" EI ");
        stream.extend_from_slice(b"\nEI Q");
        let tokens = tokenize(&stream).unwrap();
        assert_eq!(tokens.len(), 2);
        let image = parse_inline_image(tokens[0].raw).unwrap();
        assert_eq!(image.data, b" EI ");
    }

    #[test]
    fn tokenize_inline_image_mask_uses_one_bit_per_pixel() {
        let mut stream = b"BI /W 9 /H 2 /IM true ID ".to_vec();
        stream.extend_from_slice(&[0xFF, 0x80, 0xFF, 0x80]);
        stream.extend_from_slice(b" EI");
        let tokens = tokenize(&stream).unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(parse_inline_image(tokens[0].raw).unwrap().data.len(), 4);
    }

    #[test]
    fn tokenize_rejects_unterminated_literal_string() {
        let error = tokenize(b"(never closed").unwrap_err();
        assert!(matches!(error, AppError::InvalidContentStream(_)));
    }

    #[test]
    fn tokenize_rejects_unterminated_hex_string() {
        assert!(tokenize(b"<4865").is_err());
    }

    #[test]
    fn tokenize_rejects_non_hex_byte_in_hex_string() {
        assert!(tokenize(b"<48zz>").is_err());
    }

    #[test]
    fn tokenize_rejects_stray_closing_delimiters() {
        assert!(tokenize(b"a ) b").is_err());
        assert!(tokenize(b"a > b").is_err());
    }

    #[test]
    fn tokenize_rejects_inline_image_without_end_marker() {
        let error = tokenize(b"BI /W 1 /H 1 /F /Fl ID some data without terminator").unwrap_err();
        assert!(matches!(error, AppError::InvalidContentStream(_)));
    }

    #[test]
    fn tokenize_rejects_inline_image_without_data_marker() {
        assert!(tokenize(b"BI /W 1 /H 1").is_err());
    }

    #[test]
    fn tokenize_empty_and_whitespace_only_input_has_no_tokens() {
        assert!(tokenize(b"").unwrap().is_empty());
        assert!(tokenize(b" \r\n\t").unwrap().is_empty());
    }

    #[test]
    fn parse_inline_image_returns_dictionary_tokens() {
        let tokens = tokenize(b"BI /W 1 /H 1 /CS /G /BPC 8 ID \x7f EI").unwrap();
        let image = parse_inline_image(tokens[0].raw).unwrap();
        let raws: Vec<&[u8]> = image.dictionary.iter().map(|token| token.raw).collect();
        assert_eq!(
            raws,
            vec![&b"/W"[..], b"1", b"/H", b"1", b"/CS", b"/G", b"/BPC", b"8"]
        );
        assert_eq!(image.data, b"\x7f");
    }

    #[test]
    fn parse_inline_image_rejects_input_not_starting_with_bi() {
        assert!(parse_inline_image(b"q Q").is_err());
    }

    #[test]
    fn decode_literal_string_handles_all_escape_forms() {
        let raw = b"(a\\n\\r\\t\\b\\f\\(\\)\\\\\\101\\7\\0053\\\nx\\\r\nz\\q)";
        let expected: Vec<u8> = vec![
            b'a', b'\n', b'\r', b'\t', 0x08, 0x0C, b'(', b')', b'\\', b'A', 0x07, 0x05, b'3', b'x',
            b'z', b'q',
        ];
        assert_eq!(decode_literal_string(raw), expected);
    }

    #[test]
    fn decode_literal_string_keeps_nested_parentheses() {
        assert_eq!(decode_literal_string(b"(a(b)c)"), b"a(b)c");
    }

    #[test]
    fn decode_hex_string_pads_odd_length_with_zero() {
        assert_eq!(decode_hex_string(b"<48 65 6C 6C 6F 7>"), b"Hello\x70");
    }

    #[test]
    fn decode_hex_string_accepts_lowercase_and_empty() {
        assert_eq!(decode_hex_string(b"<4a4B>"), b"JK");
        assert_eq!(decode_hex_string(b"<>"), b"");
    }
}
