use std::collections::{HashMap, HashSet};

use super::eval::{const_eval, static_size};
use super::lexer::{tokenize, Tok, Token};
use super::*;

type Result<T> = std::result::Result<T, TemplateError>;

const KEYWORDS: &[&str] =
    &["alias", "block", "const", "else", "endian", "enum", "file", "flags", "frame", "if", "match", "struct", "where"];
/// Keywords that cannot be field names or appear in expressions. The others only start
/// top-level declarations, so fields may be called `flags` or `file`.
const RESERVED: &[&str] = &["else", "if", "match", "where"];

pub fn parse(src: &str) -> Result<Template> {
    if !detect(src)? {
        return Err(TemplateError::new(1, 1, "a block template must start with '#lang blocks 1'"));
    }
    let mut p = Parser {
        src,
        toks: tokenize(src)?,
        pos: 0,
        t: Template::default(),
        field_names: HashSet::new(),
        decls_seen: false,
    };
    p.parse_top()?;
    let mut r = Resolver { t: p.t, field_names: p.field_names };
    r.run()?;
    Ok(r.t)
}

/// `u32`, `f64be`, …
pub fn prim_type(name: &str) -> Option<PrimType> {
    let (base, endian) = if let Some(b) = name.strip_suffix("le") {
        (b, Some(Endian::Little))
    } else if let Some(b) = name.strip_suffix("be") {
        (b, Some(Endian::Big))
    } else {
        (name, None)
    };
    let prim = match base {
        "u8" => Prim::U8,
        "u16" => Prim::U16,
        "u32" => Prim::U32,
        "u64" => Prim::U64,
        "i8" => Prim::I8,
        "i16" => Prim::I16,
        "i32" => Prim::I32,
        "i64" => Prim::I64,
        "f32" => Prim::F32,
        "f64" => Prim::F64,
        _ => return None,
    };
    Some(PrimType { prim, endian })
}

fn str_kind(name: &str) -> Option<(StrKind, Option<Endian>)> {
    Some(match name {
        "ascii" => (StrKind::Ascii, None),
        "utf8" => (StrKind::Utf8, None),
        "cp" => (StrKind::Cp, None),
        "utf16" => (StrKind::Utf16, None),
        "utf16le" => (StrKind::Utf16, Some(Endian::Little)),
        "utf16be" => (StrKind::Utf16, Some(Endian::Big)),
        "utf32" => (StrKind::Utf32, None),
        "utf32le" => (StrKind::Utf32, Some(Endian::Little)),
        "utf32be" => (StrKind::Utf32, Some(Endian::Big)),
        _ => return None,
    })
}

/// Names declared by a list of items, including those in `if` / `match` branches.
fn declared_names(items: &[Item], out: &mut Vec<String>) {
    for it in items {
        match it {
            Item::Field(f) if f.name != "_" => {
                if !out.contains(&f.name) {
                    out.push(f.name.clone());
                }
            }
            Item::Field(_) => {}
            Item::If { names, .. } | Item::Match { names, .. } => {
                for n in names {
                    if !out.contains(n) {
                        out.push(n.clone());
                    }
                }
            }
        }
    }
}

struct Parser<'s> {
    src: &'s str,
    toks: Vec<Token>,
    pos: usize,
    t: Template,
    /// every field name in the template, for the ambiguous enum item check
    field_names: HashSet<String>,
    decls_seen: bool,
}

impl Parser<'_> {
    fn tok(&self) -> &Token {
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn peek(&self) -> &Tok {
        &self.tok().tok
    }

    fn here(&self) -> Pos {
        Pos { line: self.tok().line, col: self.tok().col }
    }

    fn next(&mut self) -> Token {
        let t = self.tok().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn err(&self, msg: impl Into<String>) -> TemplateError {
        self.here().err(msg)
    }

    fn found(&self) -> String {
        describe(self.peek())
    }

    fn is_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(x) if *x == p)
    }

    fn eat_punct(&mut self, p: &str) -> bool {
        if self.is_punct(p) {
            self.next();
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, p: &str) -> Result<()> {
        if self.eat_punct(p) {
            Ok(())
        } else {
            Err(self.err(format!("expected '{p}', found {}", self.found())))
        }
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == w)
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if self.is_word(w) {
            self.next();
            true
        } else {
            false
        }
    }

    /// The name of a top-level declaration, which may not be a keyword.
    fn expect_decl_name(&mut self) -> Result<String> {
        if let Tok::Ident(s) = self.peek() {
            if KEYWORDS.contains(&s.as_str()) {
                return Err(self.err(format!("'{s}' is a keyword and cannot be used as a name")));
            }
        }
        self.expect_name()
    }

    /// An identifier other than `if`, `else`, `match` and `where`.
    fn expect_name(&mut self) -> Result<String> {
        match self.peek().clone() {
            Tok::Ident(s) if RESERVED.contains(&s.as_str()) => {
                Err(self.err(format!("'{s}' is a keyword and cannot be used as a name")))
            }
            Tok::Ident(s) => {
                self.next();
                Ok(s)
            }
            _ => Err(self.err(format!("expected a name, found {}", self.found()))),
        }
    }

    fn is_sep(&self) -> bool {
        matches!(self.peek(), Tok::Newline | Tok::Punct(","))
    }

    fn skip_seps(&mut self) {
        while self.is_sep() {
            self.next();
        }
    }

    /// `{ x sep x sep … }` with optional separators at both ends (1.3).
    fn parse_list<T>(&mut self, mut f: impl FnMut(&mut Self) -> Result<T>) -> Result<Vec<T>> {
        self.expect_punct("{")?;
        let mut out = Vec::new();
        self.skip_seps();
        while !self.is_punct("}") {
            if *self.peek() == Tok::Eof {
                return Err(self.err("expected '}', found end of input"));
            }
            out.push(f(self)?);
            if self.is_punct("}") {
                break;
            }
            if !self.is_sep() {
                return Err(self.err(format!("expected a line break, ',' or '}}', found {}", self.found())));
            }
            self.skip_seps();
        }
        self.next();
        Ok(out)
    }

    fn declare(&mut self, name: &str, d: Decl, pos: Pos) -> Result<()> {
        if self.t.names.contains_key(name) {
            return Err(pos.err(format!("duplicate declaration '{name}'")));
        }
        self.t.names.insert(name.to_string(), d);
        if matches!(d, Decl::Struct(_) | Decl::Block(_)) {
            self.t.root_names.push(name.to_string());
        }
        Ok(())
    }

    // ---------------------------------------------------------------- top level

    fn parse_top(&mut self) -> Result<()> {
        self.skip_seps();
        while *self.peek() != Tok::Eof {
            self.parse_top_item()?;
            if *self.peek() == Tok::Eof {
                break;
            }
            if !self.is_sep() {
                return Err(self.err(format!("expected a line break or ',', found {}", self.found())));
            }
            self.skip_seps();
        }
        Ok(())
    }

    fn parse_top_item(&mut self) -> Result<()> {
        let pos = self.here();
        let Tok::Ident(word) = self.peek().clone() else {
            return Err(self.err(format!("expected a declaration, found {}", self.found())));
        };
        if word == "endian" {
            if self.decls_seen || self.t.endian.is_some() {
                return Err(self.err("'endian' must come once, before all other declarations"));
            }
            self.next();
            self.t.endian = Some(if self.eat_word("little") {
                Endian::Little
            } else if self.eat_word("big") {
                Endian::Big
            } else {
                return Err(self.err(format!("expected 'little' or 'big', found {}", self.found())));
            });
            return Ok(());
        }
        self.decls_seen = true;
        match word.as_str() {
            "const" => {
                self.next();
                let name = self.expect_decl_name()?;
                self.expect_punct("=")?;
                let e = self.parse_expr()?;
                self.declare(&name, Decl::Const, pos)?;
                self.t.const_exprs.push((name, e, pos));
            }
            "struct" => {
                self.next();
                let name = self.expect_decl_name()?;
                self.declare(&name, Decl::Struct(self.t.structs.len()), pos)?;
                let items = self.parse_items()?;
                self.t.structs.push(StructDef { name, items, pos });
            }
            "enum" => {
                self.next();
                let name = self.expect_decl_name()?;
                self.expect_punct(":")?;
                let base = self.parse_int_type()?;
                self.declare(&name, Decl::Enum(self.t.enums.len()), pos)?;
                let raw = self.parse_list(|p| {
                    let pos = p.here();
                    let n = p.expect_name()?;
                    let v = if p.eat_punct("=") { Some(p.parse_expr()?) } else { None };
                    Ok((n, v, pos))
                })?;
                self.t.enums.push(EnumDef { name, base, raw, items: vec![], pos });
            }
            "flags" => {
                self.next();
                let name = self.expect_decl_name()?;
                self.expect_punct(":")?;
                let base = self.parse_int_type()?;
                self.declare(&name, Decl::Flags(self.t.flags.len()), pos)?;
                let raw = self.parse_list(|p| {
                    let pos = p.here();
                    let n = p.expect_name()?;
                    p.expect_punct("=")?;
                    Ok((n, p.parse_expr()?, pos))
                })?;
                self.t.flags.push(FlagsDef { name, base, raw, bits: vec![], pos });
            }
            "alias" => {
                self.next();
                let name = self.expect_decl_name()?;
                self.expect_punct("=")?;
                let members = self.parse_name_list()?;
                self.declare(&name, Decl::Alias(self.t.aliases.len()), pos)?;
                self.t.aliases.push(AliasDef { name, members, targets: vec![], pos });
            }
            "frame" => {
                self.next();
                self.parse_frame(pos)?;
            }
            "block" => {
                self.next();
                self.parse_block(pos)?;
            }
            "file" => {
                self.next();
                if self.t.file.is_some() {
                    return Err(pos.err("more than one 'file { }'"));
                }
                self.t.file = Some(self.parse_items()?);
            }
            _ => return Err(self.err(format!("expected a declaration, found {}", self.found()))),
        }
        Ok(())
    }

    /// `A | B | C`
    fn parse_name_list(&mut self) -> Result<Vec<(String, Pos)>> {
        let mut out = Vec::new();
        loop {
            let pos = self.here();
            out.push((self.expect_name()?, pos));
            if !self.eat_punct("|") {
                return Ok(out);
            }
        }
    }

    fn parse_int_type(&mut self) -> Result<PrimType> {
        if let Tok::Ident(s) = self.peek() {
            if let Some(p) = prim_type(s).filter(|p| p.prim.is_integer()) {
                self.next();
                return Ok(p);
            }
        }
        Err(self.err(format!("expected an integer type, found {}", self.found())))
    }

    fn parse_frame(&mut self, pos: Pos) -> Result<()> {
        let name = self.expect_decl_name()?;
        self.declare(&name, Decl::Frame(self.t.frames.len()), pos)?;
        let mut params = Vec::new();
        if self.eat_punct("(") {
            while !self.is_punct(")") {
                let ppos = self.here();
                let p = self.expect_name()?;
                if params.contains(&p) {
                    return Err(ppos.err(format!("duplicate parameter '{p}'")));
                }
                params.push(p);
                if !self.eat_punct(",") {
                    break;
                }
            }
            self.expect_punct(")")?;
        }
        let align_expr = if self.eat_word("align") { Some(self.parse_expr()?) } else { None };
        let sections = self.parse_list(|p| {
            let pos = p.here();
            let name = p.expect_name()?;
            let kind_at = if p.eat_punct(":") {
                let kind = if p.eat_punct("[") {
                    let ty = p.parse_int_type()?;
                    p.expect_punct(";")?;
                    let count = p.parse_expr()?;
                    p.expect_punct("]")?;
                    SectionKind::Slots { ty, count }
                } else if p.eat_word("size") {
                    p.expect_punct("(")?;
                    let size = p.parse_expr()?;
                    p.expect_punct(")")?;
                    SectionKind::Open { size }
                } else {
                    return Err(p.err(format!("expected '[' or 'size', found {}", p.found())));
                };
                let at = if p.eat_punct("@") { Some(p.parse_expr()?) } else { None };
                (kind, at)
            } else {
                let at = if p.eat_punct("@") { Some(p.parse_expr()?) } else { None };
                (SectionKind::Fields(p.parse_items()?), at)
            };
            Ok(Section { name, at: kind_at.1, before: None, kind: kind_at.0, pos })
        })?;
        let mut seen = HashSet::new();
        for s in &sections {
            if !seen.insert(&s.name) {
                return Err(s.pos.err(format!("duplicate frame section '{}'", s.name)));
            }
        }
        self.t.frames.push(FrameDef { name, params, align_expr, align: 1, sections, prefix: 0, pos });
        Ok(())
    }

    fn parse_block(&mut self, pos: Pos) -> Result<()> {
        let name = self.expect_decl_name()?;
        self.declare(&name, Decl::Block(self.t.blocks.len()), pos)?;
        self.expect_punct(":")?;
        let fpos = self.here();
        let frame = self.expect_name()?;
        let mut arg_exprs = Vec::new();
        if self.eat_punct("(") {
            while !self.is_punct(")") {
                arg_exprs.push(self.parse_expr()?);
                if !self.eat_punct(",") {
                    break;
                }
            }
            self.expect_punct(")")?;
        }
        let raw_sections = self.parse_list(|p| {
            let pos = p.here();
            let n = p.expect_name()?;
            Ok((n, p.parse_items()?, pos))
        })?;
        self.t.blocks.push(BlockDef {
            name,
            frame_name: (frame, fpos),
            frame: 0,
            arg_exprs,
            args: vec![],
            raw_sections,
            bodies: vec![],
            pos,
        });
        Ok(())
    }

    // ------------------------------------------------------------------ items

    fn parse_items(&mut self) -> Result<Vec<Item>> {
        self.parse_list(|p| p.parse_item())
    }

    fn parse_item(&mut self) -> Result<Item> {
        let pos = self.here();
        if self.eat_word("if") {
            let mut branches = vec![(self.parse_expr()?, self.parse_items()?)];
            let mut otherwise = None;
            while self.eat_word("else") {
                if self.eat_word("if") {
                    branches.push((self.parse_expr()?, self.parse_items()?));
                } else {
                    otherwise = Some(self.parse_items()?);
                    break;
                }
            }
            let mut names = Vec::new();
            for (_, b) in &branches {
                declared_names(b, &mut names);
            }
            if let Some(o) = &otherwise {
                declared_names(o, &mut names);
            }
            return Ok(Item::If { branches, otherwise, names, pos });
        }
        if self.eat_word("match") {
            let expr = self.parse_expr()?;
            let arms = self.parse_list(|p| {
                let mut patterns = Vec::new();
                loop {
                    if p.eat_word("_") {
                        patterns.push(Pattern::Wild);
                    } else {
                        // a top-level '|' separates alternatives
                        patterns.push(Pattern::Expr(p.parse_bin(4)?));
                    }
                    if !p.eat_punct("|") {
                        break;
                    }
                }
                p.expect_punct("=>")?;
                Ok(Arm { patterns, items: p.parse_items()? })
            })?;
            let mut names = Vec::new();
            for a in &arms {
                declared_names(&a.items, &mut names);
            }
            return Ok(Item::Match { expr, arms, names, pos });
        }

        let name = self.expect_name()?;
        self.expect_punct(":")?;
        let ty = self.parse_type()?;
        let at = if self.eat_punct("@") { Some(self.parse_expr()?) } else { None };
        let check = if self.eat_word("where") {
            let start = self.tok().start;
            let expr = self.parse_expr()?;
            let end = self.toks[self.pos - 1].end;
            Some(Check { expr, text: self.src[start..end].trim().to_string() })
        } else {
            None
        };
        if name != "_" {
            self.field_names.insert(name.clone());
        }
        Ok(Item::Field(Field { name, ty, at, check, pos }))
    }

    fn parse_type(&mut self) -> Result<Type> {
        let pos = self.here();
        if self.eat_punct("[") {
            let elem = self.parse_type()?;
            self.expect_punct(";")?;
            let count = if self.eat_punct("..") { None } else { Some(Box::new(self.parse_expr()?)) };
            self.expect_punct("]")?;
            return Ok(Type::Array(Box::new(elem), count));
        }
        if self.eat_punct("->") {
            return Ok(Type::LinkNames(self.parse_name_list()?));
        }
        let name = match self.peek() {
            Tok::Ident(s) => s.clone(),
            _ => return Err(self.err(format!("expected a type, found {}", self.found()))),
        };
        if let Some(p) = prim_type(&name) {
            self.next();
            return Ok(Type::Prim(p));
        }
        if let Some((kind, endian)) = str_kind(&name) {
            self.next();
            let len = if self.eat_punct("(") {
                let e = self.parse_expr()?;
                self.expect_punct(")")?;
                Some(Box::new(e))
            } else {
                None
            };
            return Ok(Type::Str(kind, endian, len));
        }
        let name = self.expect_name()?;
        Ok(Type::Named(name, pos))
    }

    // ------------------------------------------------------------ expressions

    fn parse_expr(&mut self) -> Result<Expr> {
        let c = self.parse_bin(0)?;
        if self.eat_punct("?") {
            let a = self.parse_expr()?;
            self.expect_punct(":")?;
            let b = self.parse_expr()?;
            return Ok(Expr::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    fn parse_bin(&mut self, min_prec: u8) -> Result<Expr> {
        let mut lhs = self.parse_unary()?;
        while let Tok::Punct(p) = self.peek() {
            let Some((op, prec)) = bin_op(p) else { break };
            if prec < min_prec {
                break;
            }
            self.next();
            let rhs = self.parse_bin(prec + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        if self.eat_punct("-") {
            return Ok(Expr::Unary(UnOp::Neg, Box::new(self.parse_unary()?)));
        }
        if self.eat_punct("+") {
            return self.parse_unary();
        }
        if self.eat_punct("~") {
            return Ok(Expr::Unary(UnOp::BitNot, Box::new(self.parse_unary()?)));
        }
        if self.eat_punct("!") {
            return Ok(Expr::Unary(UnOp::Not, Box::new(self.parse_unary()?)));
        }
        let pos = self.here();
        match self.peek().clone() {
            Tok::Num(n) => {
                self.next();
                Ok(Expr::Num(n))
            }
            Tok::Str(s) => {
                self.next();
                Ok(Expr::Str(s))
            }
            Tok::Punct("(") => {
                self.next();
                let e = self.parse_expr()?;
                self.expect_punct(")")?;
                Ok(e)
            }
            Tok::Ident(s) if s == "sizeof" => {
                self.next();
                self.expect_punct("(")?;
                let ty = self.parse_type()?;
                self.expect_punct(")")?;
                Ok(Expr::SizeOf(Box::new(ty), pos))
            }
            Tok::Ident(s) if !RESERVED.contains(&s.as_str()) => {
                self.next();
                let mut path = vec![s];
                while self.eat_punct(".") {
                    path.push(self.expect_name()?);
                }
                Ok(Expr::Path(path, pos))
            }
            _ => Err(self.err(format!("expected an expression, found {}", self.found()))),
        }
    }
}

fn bin_op(p: &str) -> Option<(BinOp, u8)> {
    use BinOp::*;
    Some(match p {
        "||" => (Or, 1),
        "&&" => (And, 2),
        "|" => (BitOr, 3),
        "^" => (BitXor, 4),
        "&" => (BitAnd, 5),
        "==" => (Eq, 6),
        "!=" => (Ne, 6),
        "<" => (Lt, 7),
        ">" => (Gt, 7),
        "<=" => (Le, 7),
        ">=" => (Ge, 7),
        "<<" => (Shl, 8),
        ">>" => (Shr, 8),
        "+" => (Add, 9),
        "-" => (Sub, 9),
        "*" => (Mul, 10),
        "/" => (Div, 10),
        "%" => (Rem, 10),
        _ => return None,
    })
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => format!("'{s}'"),
        Tok::Num(n) => format!("'{n}'"),
        Tok::Str(s) => format!("\"{s}\""),
        Tok::Punct(p) => format!("'{p}'"),
        Tok::Newline => "a line break".into(),
        Tok::Eof => "end of input".into(),
    }
}

// ------------------------------------------------------------------ resolution

/// What type resolution needs to know, separate from the template it mutates.
struct Names {
    names: HashMap<String, Decl>,
    aliases: Vec<Vec<Target>>,
    targets: HashMap<Target, String>,
}

impl Names {
    fn resolve_type(&self, ty: &mut Type) -> Result<()> {
        match ty {
            Type::Prim(_) | Type::Struct(_) | Type::Enum(_) | Type::Flags(_) | Type::Block(_) | Type::Link(..) => Ok(()),
            Type::Str(_, _, len) => len.as_deref_mut().map_or(Ok(()), |e| self.resolve_expr(e)),
            Type::Array(elem, count) => {
                self.resolve_type(elem)?;
                count.as_deref_mut().map_or(Ok(()), |e| self.resolve_expr(e))
            }
            Type::Named(name, pos) => {
                *ty = match self.names.get(name.as_str()) {
                    Some(Decl::Struct(i)) => Type::Struct(*i),
                    Some(Decl::Enum(i)) => Type::Enum(*i),
                    Some(Decl::Flags(i)) => Type::Flags(*i),
                    Some(Decl::Block(i)) => Type::Block(*i),
                    Some(Decl::Alias(_)) => {
                        return Err(pos.err(format!("alias '{name}' can only be used as a link target")))
                    }
                    Some(_) => return Err(pos.err(format!("'{name}' is not a type"))),
                    None => return Err(pos.err(format!("unknown type '{name}'"))),
                };
                Ok(())
            }
            Type::LinkNames(list) => {
                let mut out: Vec<Target> = Vec::new();
                for (name, pos) in list.iter() {
                    let add: Vec<Target> = match self.names.get(name.as_str()) {
                        Some(Decl::Struct(i)) => vec![Target::Struct(*i)],
                        Some(Decl::Block(i)) => vec![Target::Block(*i)],
                        Some(Decl::Alias(i)) => self.aliases[*i].clone(),
                        Some(_) => return Err(pos.err(format!("'{name}' cannot be a link target"))),
                        None => return Err(pos.err(format!("unknown type '{name}'"))),
                    };
                    for t in add {
                        if !out.contains(&t) {
                            out.push(t);
                        }
                    }
                }
                if out.len() > 1 && out.iter().any(|t| matches!(t, Target::Struct(_))) {
                    return Err(list[0].1.err("a struct can only be the single target of a link"));
                }
                let text = out.iter().map(|t| self.targets[t].as_str()).collect::<Vec<_>>().join(" | ");
                *ty = Type::Link(out, text);
                Ok(())
            }
        }
    }

    fn resolve_expr(&self, e: &mut Expr) -> Result<()> {
        match e {
            Expr::Num(_) | Expr::Str(_) | Expr::Path(..) => Ok(()),
            Expr::Unary(_, a) => self.resolve_expr(a),
            Expr::Binary(_, a, b) => {
                self.resolve_expr(a)?;
                self.resolve_expr(b)
            }
            Expr::Cond(c, a, b) => {
                self.resolve_expr(c)?;
                self.resolve_expr(a)?;
                self.resolve_expr(b)
            }
            Expr::SizeOf(ty, pos) => {
                if matches!(**ty, Type::LinkNames(_)) {
                    return Err(pos.err("sizeof of a link type"));
                }
                self.resolve_type(ty)
            }
        }
    }

    fn resolve_items(&self, items: &mut [Item]) -> Result<()> {
        for it in items {
            match it {
                Item::Field(f) => {
                    self.resolve_type(&mut f.ty)?;
                    if let Some(e) = &mut f.at {
                        self.resolve_expr(e)?;
                    }
                    if let Some(c) = &mut f.check {
                        self.resolve_expr(&mut c.expr)?;
                    }
                }
                Item::If { branches, otherwise, .. } => {
                    for (c, b) in branches {
                        self.resolve_expr(c)?;
                        self.resolve_items(b)?;
                    }
                    if let Some(o) = otherwise {
                        self.resolve_items(o)?;
                    }
                }
                Item::Match { expr, arms, .. } => {
                    self.resolve_expr(expr)?;
                    for a in arms {
                        for p in &mut a.patterns {
                            if let Pattern::Expr(e) = p {
                                self.resolve_expr(e)?;
                            }
                        }
                        self.resolve_items(&mut a.items)?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ConstKey {
    Const(usize),
    Enum(usize),
}

#[derive(Clone, Copy)]
enum Ctx {
    /// `file { }`: `@` places fields
    File,
    /// structs, field sections, open sections
    Plain,
    /// a slot section with this slot type
    Slots(PrimType),
}

struct Resolver {
    t: Template,
    field_names: HashSet<String>,
}

impl Resolver {
    fn run(&mut self) -> Result<()> {
        self.index_enum_items()?;
        self.resolve_aliases()?;
        self.resolve_types()?;
        self.bind_blocks()?;
        self.check_containment()?;
        self.eval_consts()?;
        self.eval_flags()?;
        self.eval_frames()?;
        self.validate()
    }

    fn index_enum_items(&mut self) -> Result<()> {
        for (i, e) in self.t.enums.iter().enumerate() {
            let mut seen = HashSet::new();
            for (n, _, pos) in &e.raw {
                if !seen.insert(n) {
                    return Err(pos.err(format!("duplicate item '{n}' in enum '{}'", e.name)));
                }
                self.t.enum_items.entry(n.clone()).or_default().push(i);
            }
        }
        for f in &self.t.flags {
            let mut seen = HashSet::new();
            for (n, _, pos) in &f.raw {
                if !seen.insert(n) {
                    return Err(pos.err(format!("duplicate flag '{n}' in '{}'", f.name)));
                }
            }
        }
        Ok(())
    }

    fn resolve_aliases(&mut self) -> Result<()> {
        fn expand(t: &Template, i: usize, stack: &mut Vec<usize>, out: &mut Vec<Target>) -> Result<()> {
            let a = &t.aliases[i];
            if stack.contains(&i) {
                return Err(a.pos.err(format!("alias '{}' refers to itself", a.name)));
            }
            stack.push(i);
            for (name, pos) in &a.members {
                match t.names.get(name) {
                    Some(Decl::Block(b)) => {
                        if !out.contains(&Target::Block(*b)) {
                            out.push(Target::Block(*b));
                        }
                    }
                    Some(Decl::Alias(j)) => expand(t, *j, stack, out)?,
                    Some(_) => return Err(pos.err(format!("alias members must be blocks, '{name}' is not one"))),
                    None => return Err(pos.err(format!("unknown block '{name}'"))),
                }
            }
            stack.pop();
            Ok(())
        }
        for i in 0..self.t.aliases.len() {
            let mut out = Vec::new();
            expand(&self.t, i, &mut Vec::new(), &mut out)?;
            self.t.aliases[i].targets = out;
        }
        Ok(())
    }

    fn resolve_types(&mut self) -> Result<()> {
        let mut targets = HashMap::new();
        for (i, s) in self.t.structs.iter().enumerate() {
            targets.insert(Target::Struct(i), s.name.clone());
        }
        for (i, b) in self.t.blocks.iter().enumerate() {
            targets.insert(Target::Block(i), b.name.clone());
        }
        let n = Names {
            names: self.t.names.clone(),
            aliases: self.t.aliases.iter().map(|a| a.targets.clone()).collect(),
            targets,
        };
        let t = &mut self.t;
        for (_, e, _) in &mut t.const_exprs {
            n.resolve_expr(e)?;
        }
        for en in &mut t.enums {
            for (_, e, _) in &mut en.raw {
                if let Some(e) = e {
                    n.resolve_expr(e)?;
                }
            }
        }
        for f in &mut t.flags {
            for (_, e, _) in &mut f.raw {
                n.resolve_expr(e)?;
            }
        }
        for s in &mut t.structs {
            n.resolve_items(&mut s.items)?;
        }
        if let Some(items) = &mut t.file {
            n.resolve_items(items)?;
        }
        for f in &mut t.frames {
            if let Some(e) = &mut f.align_expr {
                n.resolve_expr(e)?;
            }
            for s in &mut f.sections {
                if let Some(e) = &mut s.at {
                    n.resolve_expr(e)?;
                }
                match &mut s.kind {
                    SectionKind::Fields(items) => n.resolve_items(items)?,
                    SectionKind::Slots { count, .. } => n.resolve_expr(count)?,
                    SectionKind::Open { size } => n.resolve_expr(size)?,
                }
            }
        }
        for b in &mut t.blocks {
            for e in &mut b.arg_exprs {
                n.resolve_expr(e)?;
            }
            for (_, items, _) in &mut b.raw_sections {
                n.resolve_items(items)?;
            }
        }
        Ok(())
    }

    fn bind_blocks(&mut self) -> Result<()> {
        let t = &mut self.t;
        for b in &mut t.blocks {
            let (fname, fpos) = &b.frame_name;
            let fi = match t.names.get(fname) {
                Some(Decl::Frame(i)) => *i,
                Some(_) => return Err(fpos.err(format!("'{fname}' is not a frame"))),
                None => return Err(fpos.err(format!("unknown frame '{fname}'"))),
            };
            let f = &t.frames[fi];
            b.frame = fi;
            if b.arg_exprs.len() != f.params.len() {
                return Err(fpos.err(format!(
                    "frame '{fname}' takes {} argument(s), {} given",
                    f.params.len(),
                    b.arg_exprs.len()
                )));
            }
            b.bodies = vec![None; f.sections.len()];
            for (name, items, pos) in &b.raw_sections {
                let Some(si) = f.sections.iter().position(|s| s.name == *name) else {
                    return Err(pos.err(format!("frame '{fname}' has no section '{name}'")));
                };
                if matches!(f.sections[si].kind, SectionKind::Fields(_)) {
                    return Err(pos.err(format!("section '{name}' is a field section of '{fname}' and cannot be changed")));
                }
                if b.bodies[si].is_some() {
                    return Err(pos.err(format!("duplicate block section '{name}'")));
                }
                b.bodies[si] = Some(items.clone());
            }
        }
        Ok(())
    }

    /// Structs and blocks cannot contain themselves by value (4.5).
    fn check_containment(&self) -> Result<()> {
        fn deps_items(items: &[Item], out: &mut Vec<Target>) {
            for it in items {
                match it {
                    Item::Field(f) => deps_type(&f.ty, out),
                    Item::If { branches, otherwise, .. } => {
                        for (_, b) in branches {
                            deps_items(b, out);
                        }
                        if let Some(o) = otherwise {
                            deps_items(o, out);
                        }
                    }
                    Item::Match { arms, .. } => {
                        for a in arms {
                            deps_items(&a.items, out);
                        }
                    }
                }
            }
        }
        fn deps_type(ty: &Type, out: &mut Vec<Target>) {
            match ty {
                Type::Array(e, _) => deps_type(e, out),
                Type::Struct(i) => out.push(Target::Struct(*i)),
                Type::Block(i) => out.push(Target::Block(*i)),
                _ => {}
            }
        }
        let t = &self.t;
        let deps = |n: Target| {
            let mut out = Vec::new();
            match n {
                Target::Struct(i) => deps_items(&t.structs[i].items, &mut out),
                Target::Block(i) => {
                    let b = &t.blocks[i];
                    for s in &t.frames[b.frame].sections {
                        if let SectionKind::Fields(items) = &s.kind {
                            deps_items(items, &mut out);
                        }
                    }
                    for body in b.bodies.iter().flatten() {
                        deps_items(body, &mut out);
                    }
                }
            }
            out
        };
        fn visit(
            n: Target,
            deps: &dyn Fn(Target) -> Vec<Target>,
            state: &mut HashMap<Target, u8>,
            t: &Template,
        ) -> Result<()> {
            match state.get(&n) {
                Some(2) => return Ok(()),
                Some(1) => {
                    let pos = match n {
                        Target::Struct(i) => t.structs[i].pos,
                        Target::Block(i) => t.blocks[i].pos,
                    };
                    return Err(pos.err(format!("'{}' contains itself", t.target_name(n))));
                }
                _ => {}
            }
            state.insert(n, 1);
            for d in deps(n) {
                visit(d, deps, state, t)?;
            }
            state.insert(n, 2);
            Ok(())
        }
        let mut state = HashMap::new();
        for i in 0..t.structs.len() {
            visit(Target::Struct(i), &deps, &mut state, t)?;
        }
        for i in 0..t.blocks.len() {
            visit(Target::Block(i), &deps, &mut state, t)?;
        }
        Ok(())
    }

    /// Constants and enum values, in dependency order.
    fn eval_consts(&mut self) -> Result<()> {
        let const_idx: HashMap<String, usize> =
            self.t.const_exprs.iter().enumerate().map(|(i, (n, _, _))| (n.clone(), i)).collect();
        let mut state: HashMap<ConstKey, u8> = HashMap::new();
        let keys: Vec<ConstKey> = (0..self.t.const_exprs.len())
            .map(ConstKey::Const)
            .chain((0..self.t.enums.len()).map(ConstKey::Enum))
            .collect();
        for k in keys {
            self.visit_const(k, &const_idx, &mut state)?;
        }
        Ok(())
    }

    fn const_deps(&self, k: ConstKey, const_idx: &HashMap<String, usize>) -> Vec<ConstKey> {
        let mut paths = Vec::new();
        match k {
            ConstKey::Const(i) => expr_paths(&self.t, &self.t.const_exprs[i].1, &mut paths),
            ConstKey::Enum(i) => {
                for e in self.t.enums[i].raw.iter().filter_map(|(_, e, _)| e.as_ref()) {
                    expr_paths(&self.t, e, &mut paths);
                }
            }
        }
        let mut out = Vec::new();
        for p in paths {
            if p.len() == 1 {
                if let Some(&c) = const_idx.get(&p[0]) {
                    out.push(ConstKey::Const(c));
                } else if let Some(es) = self.t.enum_items.get(&p[0]) {
                    out.extend(es.iter().map(|&e| ConstKey::Enum(e)));
                }
            } else if let Some(Decl::Enum(e)) = self.t.names.get(&p[0]) {
                out.push(ConstKey::Enum(*e));
            }
        }
        // an enum item may use the items before it
        out.retain(|d| *d != k);
        out
    }

    fn visit_const(
        &mut self,
        k: ConstKey,
        const_idx: &HashMap<String, usize>,
        state: &mut HashMap<ConstKey, u8>,
    ) -> Result<()> {
        let (name, pos) = match k {
            ConstKey::Const(i) => (&self.t.const_exprs[i].0, self.t.const_exprs[i].2),
            ConstKey::Enum(i) => (&self.t.enums[i].name, self.t.enums[i].pos),
        };
        match state.get(&k) {
            Some(2) => return Ok(()),
            Some(_) => return Err(pos.err(format!("'{name}' is defined in terms of itself"))),
            None => {}
        }
        state.insert(k, 1);
        for d in self.const_deps(k, const_idx) {
            self.visit_const(d, const_idx, state)?;
        }
        match k {
            ConstKey::Const(i) => {
                let (name, e, pos) = &self.t.const_exprs[i];
                let v = const_eval(&self.t, e).map_err(|m| pos.err(format!("const '{name}': {m}")))?;
                let name = name.clone();
                self.t.consts.insert(name, v);
            }
            ConstKey::Enum(i) => {
                self.t.enums[i].items.clear();
                let mut next: i128 = 0;
                for j in 0..self.t.enums[i].raw.len() {
                    let (n, e, pos) = &self.t.enums[i].raw[j];
                    let v = match e {
                        Some(e) => match const_eval(&self.t, e) {
                            Ok(Value::Int(v)) => v,
                            Ok(Value::Str(_)) => return Err(pos.err(format!("enum item '{n}' needs an integer value"))),
                            Err(m) => return Err(pos.err(format!("enum item '{n}': {m}"))),
                        },
                        None => next,
                    };
                    let n = n.clone();
                    self.t.enums[i].items.push((n, v));
                    next = v.wrapping_add(1);
                }
            }
        }
        state.insert(k, 2);
        Ok(())
    }

    fn eval_flags(&mut self) -> Result<()> {
        for i in 0..self.t.flags.len() {
            let f = &self.t.flags[i];
            let max = f.base.prim.size() * 8;
            let mut bits: Vec<(String, u32)> = Vec::new();
            for (n, e, pos) in &f.raw {
                let b = match const_eval(&self.t, e) {
                    Ok(Value::Int(b)) => b,
                    Ok(Value::Str(_)) => return Err(pos.err(format!("flag '{n}' needs a bit number"))),
                    Err(m) => return Err(pos.err(format!("flag '{n}': {m}"))),
                };
                if b < 0 || b >= max as i128 {
                    return Err(pos.err(format!("flag bit {b} is out of range 0 … {}", max - 1)));
                }
                if let Some((other, _)) = bits.iter().find(|(_, x)| *x as i128 == b) {
                    return Err(pos.err(format!("bit {b} is used by both '{other}' and '{n}'")));
                }
                bits.push((n.clone(), b as u32));
            }
            self.t.flags[i].bits = bits;
        }
        Ok(())
    }

    fn eval_frames(&mut self) -> Result<()> {
        for i in 0..self.t.frames.len() {
            let f = &self.t.frames[i];
            if let Some(e) = &f.align_expr {
                let a = match const_eval(&self.t, e) {
                    Ok(Value::Int(a)) if a > 0 && a.count_ones() == 1 && a <= u64::MAX as i128 => a as u64,
                    _ => return Err(f.pos.err("'align' needs a constant power of two")),
                };
                self.t.frames[i].align = a;
            }
            // a constant negative `@` places a section before the block start, e.g. a recovery link
            let mut started = false;
            for si in 0..self.t.frames[i].sections.len() {
                let sec = &self.t.frames[i].sections[si];
                let before = match &sec.at {
                    Some(e) => match const_eval(&self.t, e) {
                        Ok(Value::Int(v)) if v < 0 => Some(v.unsigned_abs().min(u64::MAX as u128) as u64),
                        _ => None,
                    },
                    None => None,
                };
                match before {
                    Some(_) if started => {
                        return Err(sec.pos.err(format!(
                            "section '{}' is placed before the block start, so it must come before the other sections",
                            sec.name
                        )))
                    }
                    Some(k) => {
                        let f = &mut self.t.frames[i];
                        f.prefix = f.prefix.max(k);
                        f.sections[si].before = Some(k);
                    }
                    None => started = true,
                }
            }
        }
        for i in 0..self.t.blocks.len() {
            let b = &self.t.blocks[i];
            let mut args = Vec::new();
            for e in &b.arg_exprs {
                args.push(const_eval(&self.t, e).map_err(|m| b.frame_name.1.err(format!("non-constant argument: {m}")))?);
            }
            self.t.blocks[i].args = args;
        }
        Ok(())
    }

    // ------------------------------------------------------------ validation

    fn validate(&mut self) -> Result<()> {
        let t = &self.t;
        let params: HashSet<&str> = t.frames.iter().flat_map(|f| f.params.iter().map(String::as_str)).collect();
        let mut v = Validator { t, field_names: &self.field_names, params, warnings: Vec::new() };
        for (_, e, _) in &t.const_exprs {
            v.expr(e)?;
        }
        for s in &t.structs {
            v.items(&s.items, Ctx::Plain, &mut HashSet::new())?;
        }
        if let Some(items) = &t.file {
            v.items(items, Ctx::File, &mut HashSet::new())?;
        }
        for f in &t.frames {
            for s in &f.sections {
                if let Some(e) = &s.at {
                    v.expr(e)?;
                }
                match &s.kind {
                    SectionKind::Fields(items) => v.items(items, Ctx::Plain, &mut HashSet::new())?,
                    SectionKind::Slots { count, .. } => v.expr(count)?,
                    SectionKind::Open { size } => v.expr(size)?,
                }
            }
        }
        for b in &t.blocks {
            let f = &t.frames[b.frame];
            for (si, body) in b.bodies.iter().enumerate() {
                let Some(items) = body else { continue };
                let ctx = match f.sections[si].kind {
                    SectionKind::Slots { ty, .. } => Ctx::Slots(ty),
                    _ => Ctx::Plain,
                };
                v.items(items, ctx, &mut HashSet::new())?;
            }
        }
        let warnings = v.warnings;
        self.t.warnings = warnings;
        Ok(())
    }
}

/// Paths used by an expression, including the size expressions of `sizeof` types.
fn expr_paths(t: &Template, e: &Expr, out: &mut Vec<Vec<String>>) {
    match e {
        Expr::Num(_) | Expr::Str(_) => {}
        Expr::Path(p, _) => out.push(p.clone()),
        Expr::Unary(_, a) => expr_paths(t, a, out),
        Expr::Binary(_, a, b) => {
            expr_paths(t, a, out);
            expr_paths(t, b, out);
        }
        Expr::Cond(c, a, b) => {
            expr_paths(t, c, out);
            expr_paths(t, a, out);
            expr_paths(t, b, out);
        }
        Expr::SizeOf(ty, _) => type_paths(t, ty, out),
    }
}

fn type_paths(t: &Template, ty: &Type, out: &mut Vec<Vec<String>>) {
    match ty {
        Type::Str(_, _, Some(len)) => expr_paths(t, len, out),
        Type::Array(elem, count) => {
            type_paths(t, elem, out);
            if let Some(c) = count {
                expr_paths(t, c, out);
            }
        }
        Type::Struct(i) => {
            for it in &t.structs[*i].items {
                if let Item::Field(f) = it {
                    type_paths(t, &f.ty, out);
                }
            }
        }
        _ => {}
    }
}

fn has_checks(items: &[Item]) -> bool {
    items.iter().any(|it| match it {
        Item::Field(f) => f.check.is_some(),
        Item::If { branches, otherwise, .. } => {
            branches.iter().any(|(_, b)| has_checks(b)) || otherwise.as_deref().is_some_and(has_checks)
        }
        Item::Match { arms, .. } => arms.iter().any(|a| has_checks(&a.items)),
    })
}

struct Validator<'a> {
    t: &'a Template,
    field_names: &'a HashSet<String>,
    params: HashSet<&'a str>,
    warnings: Vec<String>,
}

impl Validator<'_> {
    fn items(&mut self, items: &[Item], ctx: Ctx, declared: &mut HashSet<String>) -> Result<()> {
        for it in items {
            match it {
                Item::Field(f) => self.field(f, ctx, declared)?,
                Item::If { branches, otherwise, .. } => {
                    let mut all = HashSet::new();
                    for (c, b) in branches {
                        self.expr(c)?;
                        let mut d = declared.clone();
                        self.items(b, ctx, &mut d)?;
                        all.extend(d);
                    }
                    if let Some(o) = otherwise {
                        let mut d = declared.clone();
                        self.items(o, ctx, &mut d)?;
                        all.extend(d);
                    }
                    declared.extend(all);
                }
                Item::Match { expr, arms, .. } => {
                    self.expr(expr)?;
                    let mut all = HashSet::new();
                    for a in arms {
                        for p in &a.patterns {
                            if let Pattern::Expr(e) = p {
                                self.pattern(e)?;
                            }
                        }
                        let mut d = declared.clone();
                        self.items(&a.items, ctx, &mut d)?;
                        all.extend(d);
                    }
                    declared.extend(all);
                }
            }
        }
        Ok(())
    }

    fn field(&mut self, f: &Field, ctx: Ctx, declared: &mut HashSet<String>) -> Result<()> {
        if f.name != "_" && !declared.insert(f.name.clone()) {
            return Err(f.pos.err(format!("duplicate field '{}'", f.name)));
        }
        let is_link = |ty: &Type| matches!(ty, Type::Link(..));
        match ctx {
            Ctx::Slots(slot) => {
                let fits = |ty: &Type| match ty {
                    Type::Prim(p) => p.prim.is_integer() && p.prim.size() == slot.prim.size(),
                    Type::Enum(i) => self.t.enums[*i].base.prim.size() == slot.prim.size(),
                    Type::Flags(i) => self.t.flags[*i].base.prim.size() == slot.prim.size(),
                    ty => is_link(ty),
                };
                let ok = match &f.ty {
                    Type::Array(elem, Some(_)) => fits(elem),
                    ty => fits(ty),
                };
                if !ok {
                    return Err(f.pos.err(format!(
                        "'{}': a slot section entry must be a link, {} or an enum or flags type of the same size",
                        f.name,
                        slot.name()
                    )));
                }
                let link = match &f.ty {
                    Type::Array(elem, _) => is_link(elem),
                    ty => is_link(ty),
                };
                if f.at.is_some() && !link {
                    return Err(f.pos.err("'@' in a slot section is only allowed on links"));
                }
            }
            Ctx::File | Ctx::Plain => {
                if f.at.is_some() && matches!(ctx, Ctx::Plain) {
                    return Err(f.pos.err("'@' is only allowed on fields of 'file { }'"));
                }
            }
        }
        self.ty(&f.ty, ctx, f.pos)?;
        if let Some(e) = &f.at {
            self.expr(e)?;
        }
        if let Some(c) = &f.check {
            self.expr(&c.expr)?;
        }
        Ok(())
    }

    fn ty(&mut self, ty: &Type, ctx: Ctx, pos: Pos) -> Result<()> {
        match ty {
            Type::Link(targets, text) => {
                if !matches!(ctx, Ctx::Slots(_)) {
                    return Err(pos.err("link types are only allowed in slot sections"));
                }
                let blocks: Vec<usize> =
                    targets.iter().filter_map(|t| if let Target::Block(b) = t { Some(*b) } else { None }).collect();
                if blocks.len() > 1 {
                    for b in blocks {
                        let blk = &self.t.blocks[b];
                        let f = &self.t.frames[blk.frame];
                        let checked = f.sections[f.leading()]
                            .iter()
                            .any(|s| matches!(&s.kind, SectionKind::Fields(items) if has_checks(items)));
                        let w = format!(
                            "line {}: block '{}' in '-> {text}' has no checks before its first slot or open section, so it always matches",
                            pos.line, blk.name
                        );
                        if !checked && !self.warnings.contains(&w) {
                            self.warnings.push(w);
                        }
                    }
                }
                Ok(())
            }
            Type::Str(_, _, Some(len)) => self.expr(len),
            Type::Array(elem, count) => {
                self.ty(elem, ctx, pos)?;
                match count {
                    Some(c) => self.expr(c),
                    None if static_size(self.t, elem) == Some(0) => {
                        Err(pos.err("'[T; ..]' needs an element type whose size is not 0"))
                    }
                    None => Ok(()),
                }
            }
            _ => Ok(()),
        }
    }

    fn expr(&mut self, e: &Expr) -> Result<()> {
        match e {
            Expr::Num(_) | Expr::Str(_) => Ok(()),
            Expr::Path(p, pos) => {
                let n = &p[0];
                let ambiguous = self.t.enum_items.get(n).is_some_and(|es| es.len() > 1);
                if p.len() == 1
                    && ambiguous
                    && !self.field_names.contains(n)
                    && !self.params.contains(n.as_str())
                    && !self.t.consts.contains_key(n)
                {
                    return Err(pos.err(format!("ambiguous enum item '{n}', write it as Enum.{n}")));
                }
                Ok(())
            }
            Expr::Unary(_, a) => self.expr(a),
            Expr::Binary(_, a, b) => {
                self.expr(a)?;
                self.expr(b)
            }
            Expr::Cond(c, a, b) => {
                self.expr(c)?;
                self.expr(a)?;
                self.expr(b)
            }
            Expr::SizeOf(ty, pos) => {
                if static_size(self.t, ty).is_none() {
                    return Err(pos.err("sizeof of a type whose size depends on file data"));
                }
                Ok(())
            }
        }
    }

    /// Patterns may only use literals, constants and enum items (5.3).
    fn pattern(&self, e: &Expr) -> Result<()> {
        match e {
            Expr::Num(_) | Expr::Str(_) | Expr::SizeOf(..) => Ok(()),
            Expr::Path(p, pos) => {
                let ok = match p.as_slice() {
                    [n] => self.t.consts.contains_key(n) || self.t.enum_items.contains_key(n),
                    [en, item] => matches!(self.t.names.get(en), Some(Decl::Enum(i))
                        if self.t.enums[*i].items.iter().any(|(n, _)| n == item)),
                    _ => false,
                };
                if ok {
                    Ok(())
                } else {
                    Err(pos.err(format!("'{}' is not a constant; match patterns must be constant", p.join("."))))
                }
            }
            Expr::Unary(_, a) => self.pattern(a),
            Expr::Binary(_, a, b) => {
                self.pattern(a)?;
                self.pattern(b)
            }
            Expr::Cond(c, a, b) => {
                self.pattern(c)?;
                self.pattern(a)?;
                self.pattern(b)
            }
        }
    }
}
