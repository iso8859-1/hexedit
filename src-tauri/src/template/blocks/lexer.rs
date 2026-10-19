use super::TemplateError;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Num(i128),
    Str(String),
    Punct(&'static str),
    /// a line break that acts as a separator (1.3)
    Newline,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    pub col: usize,
    /// byte range in the source
    pub start: usize,
    pub end: usize,
}

const PUNCT2: &[&str] = &["=>", "->", "..", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||"];
const PUNCT1: &[&str] = &[
    "{", "}", "(", ")", "[", "]", ",", ";", ":", "=", "@", ".", "+", "-", "*", "/", "%", "<", ">", "&", "^",
    "|", "!", "~", "?",
];

/// Tokenizes everything after the `#lang` line, which the caller has checked.
pub fn tokenize(src: &str) -> Result<Vec<Token>, TemplateError> {
    let body_start = src.find('\n').map_or(src.len(), |i| i + 1);
    let chars: Vec<(usize, char)> = src[body_start..].char_indices().map(|(i, c)| (i + body_start, c)).collect();
    let at = |i: usize| chars.get(i).map(|&(_, c)| c);
    let byte = |i: usize| chars.get(i).map_or(src.len(), |&(b, _)| b);
    let mut toks: Vec<Token> = Vec::new();
    // open brackets, to drop line breaks inside ( ) and [ ]
    let mut open: Vec<char> = Vec::new();
    let mut i = 0;
    let mut line = 2;
    let mut col = 1;

    macro_rules! advance {
        () => {{
            if chars[i].1 == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            i += 1;
        }};
    }

    while i < chars.len() {
        let c = chars[i].1;
        let (tl, tc, start) = (line, col, byte(i));
        if c == '\n' {
            advance!();
            if !matches!(open.last(), Some('(' | '[')) {
                toks.push(Token { tok: Tok::Newline, line: tl, col: tc, start, end: start + 1 });
            }
            continue;
        }
        if c.is_whitespace() {
            advance!();
            continue;
        }
        if c == '/' && at(i + 1) == Some('/') {
            while i < chars.len() && chars[i].1 != '\n' {
                advance!();
            }
            continue;
        }
        if c == '/' && at(i + 1) == Some('*') {
            advance!();
            advance!();
            loop {
                if i >= chars.len() {
                    return Err(TemplateError::new(tl, tc, "unterminated comment"));
                }
                if chars[i].1 == '*' && at(i + 1) == Some('/') {
                    advance!();
                    advance!();
                    break;
                }
                advance!();
            }
            continue;
        }

        let tok = if c.is_ascii_alphabetic() || c == '_' {
            let mut s = String::new();
            while let Some(ch) = at(i).filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_') {
                s.push(ch);
                advance!();
            }
            Tok::Ident(s)
        } else if c.is_ascii_digit() {
            let mut s = String::new();
            while let Some(ch) = at(i).filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_') {
                s.push(ch);
                advance!();
            }
            Tok::Num(parse_number(&s).ok_or_else(|| TemplateError::new(tl, tc, format!("invalid number '{s}'")))?)
        } else if c == '"' {
            advance!();
            let mut s = String::new();
            loop {
                match at(i) {
                    None | Some('\n') => return Err(TemplateError::new(tl, tc, "unterminated string")),
                    Some('"') => {
                        advance!();
                        break;
                    }
                    Some('\\') => {
                        let (el, ec) = (line, col);
                        advance!();
                        let e = at(i).ok_or_else(|| TemplateError::new(tl, tc, "unterminated string"))?;
                        advance!();
                        let bad = |m: &str| TemplateError::new(el, ec, m);
                        match e {
                            '\\' => s.push('\\'),
                            '"' => s.push('"'),
                            'n' => s.push('\n'),
                            'r' => s.push('\r'),
                            't' => s.push('\t'),
                            '0' => s.push('\0'),
                            'x' => {
                                let mut h = String::new();
                                for _ in 0..2 {
                                    match at(i).filter(char::is_ascii_hexdigit) {
                                        Some(d) => {
                                            h.push(d);
                                            advance!();
                                        }
                                        None => return Err(bad("\\x needs two hex digits")),
                                    }
                                }
                                s.push(char::from(u8::from_str_radix(&h, 16).unwrap()));
                            }
                            'u' => {
                                if at(i) != Some('{') {
                                    return Err(bad("expected '{' after \\u"));
                                }
                                advance!();
                                let mut h = String::new();
                                while let Some(d) = at(i).filter(char::is_ascii_hexdigit) {
                                    h.push(d);
                                    advance!();
                                }
                                if at(i) != Some('}') {
                                    return Err(bad("expected '}' to end \\u{…}"));
                                }
                                advance!();
                                let ch = u32::from_str_radix(&h, 16)
                                    .ok()
                                    .and_then(char::from_u32)
                                    .ok_or_else(|| bad("invalid \\u{…} escape"))?;
                                s.push(ch);
                            }
                            other => return Err(bad(&format!("unknown escape '\\{other}'"))),
                        }
                    }
                    Some(ch) => {
                        s.push(ch);
                        advance!();
                    }
                }
            }
            Tok::Str(s)
        } else {
            let two: String = chars[i..chars.len().min(i + 2)].iter().map(|&(_, c)| c).collect();
            let p = if let Some(p) = PUNCT2.iter().find(|p| **p == two) {
                advance!();
                advance!();
                *p
            } else if let Some(p) = PUNCT1.iter().find(|p| p.starts_with(c)) {
                advance!();
                *p
            } else {
                return Err(TemplateError::new(tl, tc, format!("unexpected character '{c}'")));
            };
            match p {
                "(" | "[" | "{" => open.push(c),
                ")" | "]" | "}" => {
                    open.pop();
                }
                _ => {}
            }
            Tok::Punct(p)
        };
        toks.push(Token { tok, line: tl, col: tc, start, end: byte(i) });
    }
    toks.push(Token { tok: Tok::Eof, line, col, start: src.len(), end: src.len() });

    // 1.3: a line break directly after '{', directly before '}' or before 'else' is whitespace,
    // and several in a row count as one
    let mut out: Vec<Token> = Vec::with_capacity(toks.len());
    for (k, t) in toks.iter().enumerate() {
        if t.tok == Tok::Newline {
            let prev = out.last().map(|p| &p.tok);
            let next = toks[k + 1..].iter().find(|n| n.tok != Tok::Newline).map(|n| &n.tok);
            let skip = matches!(prev, Some(Tok::Newline) | Some(Tok::Punct("{")))
                || matches!(next, Some(Tok::Punct("}")))
                || matches!(next, Some(Tok::Ident(s)) if s == "else");
            if skip {
                continue;
            }
        }
        out.push(t.clone());
    }
    Ok(out)
}

/// Decimal, `0x`, `0b` and `0o` integers with optional `_` separators.
pub fn parse_number(s: &str) -> Option<i128> {
    let s = s.replace('_', "");
    let lower = s.to_ascii_lowercase();
    let (digits, radix) = if let Some(h) = lower.strip_prefix("0x") {
        (h, 16)
    } else if let Some(b) = lower.strip_prefix("0b") {
        (b, 2)
    } else if let Some(o) = lower.strip_prefix("0o") {
        (o, 8)
    } else {
        (lower.as_str(), 10)
    };
    if digits.is_empty() || digits.starts_with(['+', '-']) {
        return None;
    }
    i128::from_str_radix(digits, radix).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        tokenize(&format!("#lang blocks 1\n{src}")).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn numbers() {
        assert_eq!(parse_number("0xFFFF_FFFF"), Some(0xFFFF_FFFF));
        assert_eq!(parse_number("0b101010"), Some(42));
        assert_eq!(parse_number("0o52"), Some(42));
        assert_eq!(parse_number("042"), Some(42));
        assert_eq!(parse_number("0x"), None);
        assert_eq!(parse_number("12u"), None);
    }

    #[test]
    fn strings_and_punctuation() {
        let t = toks(r#"a => "x\x41\u{263A}\"" .. ->"#);
        assert_eq!(t[1], Tok::Punct("=>"));
        assert_eq!(t[2], Tok::Str("xA\u{263A}\"".into()));
        assert_eq!(t[3], Tok::Punct(".."));
        assert_eq!(t[4], Tok::Punct("->"));
        assert!(tokenize("#lang blocks 1\n\"abc\ndef\"").is_err());
    }

    #[test]
    fn line_breaks() {
        // collapsed, dropped after '{', before '}' and 'else', and inside parentheses
        let t = toks("{\n\na\n\nb // c\n}\nelse\nf(1\n+ 2)\n/* x\ny */ z");
        use Tok::*;
        assert_eq!(
            t,
            vec![
                Punct("{"),
                Ident("a".into()),
                Newline,
                Ident("b".into()),
                Punct("}"),
                Ident("else".into()),
                Newline,
                Ident("f".into()),
                Punct("("),
                Num(1),
                Punct("+"),
                Num(2),
                Punct(")"),
                Newline,
                Ident("z".into()),
                Eof
            ]
        );
    }
}
