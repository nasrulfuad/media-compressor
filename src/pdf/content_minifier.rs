use super::content_lexer::{tokenize, Token, TokenKind};

fn is_self_delimiting(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::LiteralString
            | TokenKind::HexString
            | TokenKind::ArrayStart
            | TokenKind::ArrayEnd
            | TokenKind::DictStart
            | TokenKind::DictEnd
    )
}

fn needs_separator(previous: &Token<'_>, next: &Token<'_>) -> bool {
    !(is_self_delimiting(previous.kind)
        || is_self_delimiting(next.kind)
        || next.kind == TokenKind::Name)
}

fn join_tokens(tokens: &[Token<'_>]) -> Vec<u8> {
    let mut output = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if index > 0 && needs_separator(&tokens[index - 1], token) {
            output.push(b' ');
        }
        output.extend_from_slice(token.raw);
    }
    output
}

fn same_token_sequence(left: &[Token<'_>], right: &[Token<'_>]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(a, b)| a.kind == b.kind && a.raw == b.raw)
}

pub fn minify_content(data: &[u8]) -> Option<Vec<u8>> {
    let tokens = tokenize(data).ok()?;
    if tokens
        .iter()
        .any(|token| token.kind == TokenKind::InlineImage)
    {
        return None;
    }
    let output = join_tokens(&tokens);
    if output.len() >= data.len() {
        return None;
    }
    let reparsed = tokenize(&output).ok()?;
    same_token_sequence(&tokens, &reparsed).then_some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minify_content_preserves_spaces_inside_literal_string() {
        let minified = minify_content(b"BT   (A   B)   Tj   ET").unwrap();
        assert_eq!(minified, b"BT(A   B)Tj ET");
    }

    #[test]
    fn minify_content_preserves_percent_inside_literal_string() {
        let minified = minify_content(b"BT   (Discount 50%)   Tj   ET").unwrap();
        assert_eq!(minified, b"BT(Discount 50%)Tj ET");
    }

    #[test]
    fn minify_content_removes_comments_and_collapses_whitespace() {
        let input = b"q  % save state\n  1 0 0 1 10 20 cm\r\n\r\n  /F1 12 Tf\n  Q";
        let minified = minify_content(input).unwrap();
        assert_eq!(minified, b"q 1 0 0 1 10 20 cm/F1 12 Tf Q");
    }

    #[test]
    fn minify_content_returns_none_for_inline_images() {
        let input = b"q   BI /W 1 /H 1 /CS /G /BPC 8 ID \x80 EI   Q";
        assert_eq!(minify_content(input), None);
    }

    #[test]
    fn minify_content_returns_none_when_not_smaller() {
        assert_eq!(minify_content(b"q 1 0 0 1 0 0 cm Q"), None);
    }

    #[test]
    fn minify_content_returns_none_for_unparseable_input() {
        assert_eq!(minify_content(b"BT (unterminated   Tj   ET   "), None);
    }

    #[test]
    fn minify_content_keeps_array_and_dictionary_operands() {
        let input = b"BT   [ (A)  -120  (B) ]   TJ   /Span   << /MCID  0 >>   BDC   ET";
        let minified = minify_content(input).unwrap();
        assert_eq!(minified, b"BT[(A)-120(B)]TJ/Span<</MCID 0>>BDC ET");
    }

    #[test]
    fn minify_content_output_tokenizes_to_same_tokens() {
        let streams: [&[u8]; 10] = [
            b"q   1 0 0 1 0 0   cm   Q   ",
            b"BT   /F1   12   Tf   (Hello)   Tj   ET   ",
            b"   0.5   g   0   0   100   100   re   f   ",
            b"[ 3 2 ]   0   d   10   w   0 0 m   50 50 l   S   ",
            b"BT   <48656C6C6F>   Tj   ET   % trailing",
            b"/Artifact   BMC   q   Q   EMC   ",
            b"BT [ (a) 10 (b) ] TJ   ET   % x",
            b"q % c1\n % c2\n Q   Q   ",
            b"  (a\\)b)   '   ",
            b"/GS1   gs   1.5   .5   -.25   +3   m   h   ",
        ];
        for stream in streams {
            let minified = minify_content(stream).expect("stream must shrink");
            let before = tokenize(stream).unwrap();
            let after = tokenize(&minified).unwrap();
            let before_pairs: Vec<_> = before.iter().map(|t| (t.kind, t.raw)).collect();
            let after_pairs: Vec<_> = after.iter().map(|t| (t.kind, t.raw)).collect();
            assert_eq!(
                before_pairs,
                after_pairs,
                "{}",
                String::from_utf8_lossy(stream)
            );
        }
    }
}
