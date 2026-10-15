use super::TemplateError;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Num(i128),
    Punct(&'static str),
    /// A preprocessor line, without the leading `#`.
    Directive(String),
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    pub col: usize,
}

const PUNCT2: &[&str] = &["<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "::", "->"];
const PUNCT1: &[&str] = &[
    "{", "}", "[", "]", "(", ")", ";", ",", ":", "=", "*", "+", "-", "/", "%", "&", "|", "^", "~",
    ".", "@", "!", "<", ">", "?",
];

pub fn tokenize(src: &str) -> Result<Vec<Token>, TemplateError> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let mut col = 1;
    // true while only whitespace has been seen on the current line
    let mut line_start = true;

    macro_rules! advance {
        () => {{
            if chars[i] == '\n' {
                line += 1;
                col = 1;
                line_start = true;
            } else {
                col += 1;
            }
            i += 1;
        }};
    }

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            advance!();
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                advance!();
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let (l, c0) = (line, col);
            advance!();
            advance!();
            loop {
                if i >= chars.len() {
                    return Err(TemplateError::new(l, c0, "unterminated comment"));
                }
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    advance!();
                    advance!();
                    break;
                }
                advance!();
            }
            continue;
        }

        let (tl, tc) = (line, col);
        let was_line_start = line_start;
        line_start = false;

        if c == '#' && was_line_start {
            advance!();
            let mut text = String::new();
            while i < chars.len() && chars[i] != '\n' {
                if chars[i] == '\\' && chars.get(i + 1) == Some(&'\n') {
                    advance!();
                    advance!();
                    text.push(' ');
                    continue;
                }
                if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
                    while i < chars.len() && chars[i] != '\n' {
                        advance!();
                    }
                    break;
                }
                text.push(chars[i]);
                advance!();
            }
            toks.push(Token { tok: Tok::Directive(text.trim().to_string()), line: tl, col: tc });
            continue;
        }

        if c.is_ascii_alphabetic() || c == '_' {
            let mut s = String::new();
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                s.push(chars[i]);
                advance!();
            }
            toks.push(Token { tok: Tok::Ident(s), line: tl, col: tc });
            continue;
        }

        if c.is_ascii_digit() {
            let mut s = String::new();
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                s.push(chars[i]);
                advance!();
            }
            let v = parse_number(&s)
                .ok_or_else(|| TemplateError::new(tl, tc, format!("invalid number '{s}'")))?;
            toks.push(Token { tok: Tok::Num(v), line: tl, col: tc });
            continue;
        }

        if c == '\'' {
            advance!();
            let mut value: i128 = 0;
            let mut n = 0;
            loop {
                if i >= chars.len() || chars[i] == '\n' {
                    return Err(TemplateError::new(tl, tc, "unterminated character literal"));
                }
                if chars[i] == '\'' {
                    advance!();
                    break;
                }
                let ch = if chars[i] == '\\' {
                    advance!();
                    if i >= chars.len() {
                        return Err(TemplateError::new(tl, tc, "unterminated character literal"));
                    }
                    let e = chars[i];
                    advance!();
                    match e {
                        'n' => 10,
                        'r' => 13,
                        't' => 9,
                        '0' => 0,
                        'a' => 7,
                        'b' => 8,
                        'f' => 12,
                        'v' => 11,
                        'x' => {
                            let mut h = String::new();
                            while i < chars.len() && chars[i].is_ascii_hexdigit() && h.len() < 2 {
                                h.push(chars[i]);
                                advance!();
                            }
                            i128::from_str_radix(&h, 16).map_err(|_| {
                                TemplateError::new(tl, tc, "invalid \\x escape")
                            })?
                        }
                        other => other as i128,
                    }
                } else {
                    let ch = chars[i] as i128;
                    advance!();
                    ch
                };
                // multi-character constants pack big-endian like most compilers do
                value = (value << 8) | (ch & 0xFF);
                n += 1;
            }
            if n == 0 {
                return Err(TemplateError::new(tl, tc, "empty character literal"));
            }
            toks.push(Token { tok: Tok::Num(value), line: tl, col: tc });
            continue;
        }

        let rest2: String = chars[i..chars.len().min(i + 2)].iter().collect();
        if let Some(p) = PUNCT2.iter().find(|p| **p == rest2) {
            advance!();
            advance!();
            toks.push(Token { tok: Tok::Punct(p), line: tl, col: tc });
            continue;
        }
        let rest1 = c.to_string();
        if let Some(p) = PUNCT1.iter().find(|p| **p == rest1) {
            advance!();
            toks.push(Token { tok: Tok::Punct(p), line: tl, col: tc });
            continue;
        }
        return Err(TemplateError::new(tl, tc, format!("unexpected character '{c}'")));
    }
    toks.push(Token { tok: Tok::Eof, line, col });
    Ok(toks)
}

fn parse_number(s: &str) -> Option<i128> {
    let s = s.replace('_', "");
    let lower = s.to_ascii_lowercase();
    let (digits, radix) = if let Some(h) = lower.strip_prefix("0x") {
        (h.to_string(), 16)
    } else if let Some(b) = lower.strip_prefix("0b") {
        (b.to_string(), 2)
    } else if lower.len() > 1 && lower.starts_with('0') && lower.chars().all(|c| c.is_ascii_digit()) {
        (lower[1..].to_string(), 8)
    } else {
        (lower.clone(), 10)
    };
    // strip C integer suffixes (u, l, ul, ull, ...)
    let digits = digits.trim_end_matches(['u', 'l']);
    if digits.is_empty() {
        return None;
    }
    i128::from_str_radix(digits, radix).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(parse_number("0x1F"), Some(31));
        assert_eq!(parse_number("0b101"), Some(5));
        assert_eq!(parse_number("017"), Some(15));
        assert_eq!(parse_number("100UL"), Some(100));
        assert_eq!(parse_number("0"), Some(0));
        assert_eq!(parse_number("12abc"), None);
    }

    #[test]
    fn directives_and_chars() {
        let t = tokenize("#pragma pack(1)\nx = 'AB'; // c\n").unwrap();
        assert_eq!(t[0].tok, Tok::Directive("pragma pack(1)".into()));
        assert_eq!(t[1].tok, Tok::Ident("x".into()));
        assert_eq!(t[3].tok, Tok::Num(0x4142));
    }
}
