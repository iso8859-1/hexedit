use super::eval::{const_eval, static_struct_size};
use super::lexer::{tokenize, Tok, Token};
use super::*;

type Result<T> = std::result::Result<T, TemplateError>;

const TYPE_KEYWORDS: &[&str] =
    &["signed", "unsigned", "short", "long", "int", "char", "float", "double", "bool", "_Bool", "void"];
const QUALIFIERS: &[&str] = &["const", "volatile", "static", "extern", "register", "mutable", "__unaligned"];

/// Fixed-size type names understood without a typedef.
pub fn builtin_type(name: &str) -> Option<Prim> {
    use Prim::*;
    Some(match name {
        "int8_t" | "__int8" | "s8" | "i8" | "int8" | "INT8" | "CHAR" | "SBYTE" => I8,
        "uint8_t" | "unsigned __int8" | "u8" | "uint8" | "UINT8" | "BYTE" | "UCHAR" | "uchar" | "byte" => U8,
        "int16_t" | "__int16" | "s16" | "i16" | "int16" | "INT16" | "SHORT" => I16,
        "uint16_t" | "u16" | "uint16" | "UINT16" | "WORD" | "USHORT" | "ushort" => U16,
        "int32_t" | "__int32" | "s32" | "i32" | "int32" | "INT32" | "INT" | "LONG" | "BOOL" | "HRESULT" => I32,
        "uint32_t" | "u32" | "uint32" | "UINT32" | "DWORD" | "UINT" | "ULONG" | "uint" => U32,
        "int64_t" | "__int64" | "s64" | "i64" | "int64" | "INT64" | "LONGLONG" | "LONG64" | "ssize_t"
        | "intptr_t" | "ptrdiff_t" | "off_t" | "time_t" => I64,
        "uint64_t" | "u64" | "uint64" | "UINT64" | "QWORD" | "DWORD64" | "ULONGLONG" | "ULONG64" | "size_t"
        | "uintptr_t" => U64,
        "float" | "f32" | "FLOAT" | "float32" => F32,
        "double" | "f64" | "DOUBLE" | "float64" => F64,
        "bool" | "_Bool" | "BOOLEAN" => Bool,
        "char8_t" => Char8,
        "char16_t" | "wchar_t" | "WCHAR" => Char16,
        "char32_t" => Char32,
        _ => return None,
    })
}

fn keyword_type(words: &[String]) -> std::result::Result<Ty, String> {
    let has = |w: &str| words.iter().any(|x| x == w);
    let longs = words.iter().filter(|w| *w == "long").count();
    let unsigned = has("unsigned");
    let signed = has("signed");
    let pick = |s: Prim, u: Prim| if unsigned { u } else { s };
    if unsigned && signed {
        return Err("both signed and unsigned".into());
    }
    Ok(Ty::Prim(if has("void") {
        return Ok(Ty::Void);
    } else if has("char") {
        if unsigned {
            Prim::U8
        } else if signed {
            Prim::I8
        } else {
            Prim::Char
        }
    } else if has("bool") || has("_Bool") {
        Prim::Bool
    } else if has("float") {
        Prim::F32
    } else if has("double") {
        Prim::F64
    } else if has("short") {
        pick(Prim::I16, Prim::U16)
    } else if longs >= 2 {
        pick(Prim::I64, Prim::U64)
    } else {
        // int, long (LLP64: 32 bit), plain signed/unsigned
        pick(Prim::I32, Prim::U32)
    }))
}

pub fn parse(src: &str) -> Result<Template> {
    let toks = tokenize(src)?;
    let mut p = Parser {
        toks,
        pos: 0,
        t: Template::default(),
        pack: None,
        pack_stack: Vec::new(),
        endian: None,
        ptr_size: 8,
    };
    p.parse_top()?;
    Ok(p.t)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    t: Template,
    pack: Option<u64>,
    pack_stack: Vec<Option<u64>>,
    endian: Option<Endian>,
    ptr_size: u8,
}

impl Parser {
    fn tok(&self) -> &Token {
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn peek(&self) -> &Tok {
        &self.tok().tok
    }

    fn next(&mut self) -> Token {
        let t = self.tok().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn err(&self, msg: impl Into<String>) -> TemplateError {
        let t = self.tok();
        TemplateError::new(t.line, t.col, msg)
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
            Err(self.err(format!("expected '{p}', found {}", describe(self.peek()))))
        }
    }

    fn peek_ident(&self) -> Option<&str> {
        match self.peek() {
            Tok::Ident(s) => Some(s),
            _ => None,
        }
    }

    fn expect_ident(&mut self) -> Result<String> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.next();
                Ok(s)
            }
            other => Err(self.err(format!("expected identifier, found {}", describe(&other)))),
        }
    }

    fn skip_qualifiers(&mut self) {
        while let Some(s) = self.peek_ident() {
            if QUALIFIERS.contains(&s) {
                self.next();
            } else {
                break;
            }
        }
    }

    fn add_root_name(&mut self, name: &str) {
        if !name.is_empty() && !self.t.root_names.iter().any(|n| n == name) {
            self.t.root_names.push(name.to_string());
        }
    }

    // ---------------------------------------------------------------- top level

    fn parse_top(&mut self) -> Result<()> {
        loop {
            match self.peek().clone() {
                Tok::Eof => return Ok(()),
                Tok::Directive(d) => {
                    let tok = self.next();
                    self.directive(&d, &tok)?;
                }
                Tok::Punct(";") => {
                    self.next();
                }
                Tok::Ident(s) if s == "typedef" => {
                    self.next();
                    self.parse_typedef()?;
                }
                _ => self.parse_top_decl()?,
            }
        }
    }

    fn parse_top_decl(&mut self) -> Result<()> {
        let line = self.tok().line;
        let (spec, _) = self.parse_type_spec()?;
        if self.eat_punct(";") {
            return Ok(());
        }
        loop {
            let mut f = self.parse_declarator(&spec, line, true)?;
            if f.name.is_none() || f.bits.is_some() {
                return Err(TemplateError::new(line, 1, "top-level declaration needs a name and no bitfield"));
            }
            if self.eat_punct("@") {
                f.at = Some(self.parse_expr()?);
            }
            self.t.instances.push(f);
            if self.eat_punct(",") {
                continue;
            }
            self.expect_punct(";")?;
            return Ok(());
        }
    }

    fn parse_typedef(&mut self) -> Result<()> {
        let line = self.tok().line;
        let (spec, _) = self.parse_type_spec()?;
        loop {
            let f = self.parse_declarator(&spec, line, false)?;
            let name = f.name.ok_or_else(|| self.err("typedef needs a name"))?;
            if let Ty::Struct(i) = f.ty {
                if self.t.structs[i].name.is_empty() {
                    self.t.structs[i].name = name.clone();
                }
            }
            if matches!(f.ty, Ty::Struct(_) | Ty::Enum(_)) || !f.dims.is_empty() {
                self.add_root_name(&name);
            }
            self.t.typedefs.insert(name.clone(), TypeSpec { ty: f.ty, name, dims: f.dims });
            if self.eat_punct(",") {
                continue;
            }
            self.expect_punct(";")?;
            return Ok(());
        }
    }

    // ------------------------------------------------------------- directives

    fn directive(&mut self, d: &str, tok: &Token) -> Result<()> {
        let err = |m: String| TemplateError::new(tok.line, tok.col, m);
        let (word, rest) = split_word(d);
        match word {
            "pragma" => {
                let (what, args) = split_word(rest);
                match what {
                    "pack" => self.pragma_pack(args).map_err(err),
                    "endian" => {
                        self.endian = match args.trim() {
                            "little" | "le" => Some(Endian::Little),
                            "big" | "be" => Some(Endian::Big),
                            "default" | "" => None,
                            other => return Err(err(format!("unknown endianness '{other}'"))),
                        };
                        Ok(())
                    }
                    "ptrsize" => {
                        self.ptr_size = match args.trim() {
                            "4" => 4,
                            "8" => 8,
                            other => return Err(err(format!("pointer size must be 4 or 8, not '{other}'"))),
                        };
                        Ok(())
                    }
                    _ => Ok(()), // once, warning, ... are ignored
                }
            }
            "define" => {
                let (name, body) = split_word(rest);
                if name.contains('(') {
                    return Err(err("function-like macros are not supported".into()));
                }
                if body.trim().is_empty() {
                    return Ok(());
                }
                let mut toks = tokenize(body).map_err(|e| err(e.msg))?;
                for t in &mut toks {
                    t.line = tok.line;
                }
                let saved_toks = std::mem::replace(&mut self.toks, toks);
                let saved_pos = std::mem::replace(&mut self.pos, 0);
                let expr = self.parse_expr();
                let at_end = matches!(self.peek(), Tok::Eof);
                self.toks = saved_toks;
                self.pos = saved_pos;
                let expr = expr?;
                if !at_end {
                    return Err(err(format!("#define {name}: only constant expressions are supported")));
                }
                let v = const_eval(&self.t, &expr).map_err(|e| err(format!("#define {name}: {e}")))?;
                self.t.consts.insert(name.to_string(), v);
                Ok(())
            }
            _ => Ok(()), // include, ifdef, ... are ignored
        }
    }

    fn pragma_pack(&mut self, args: &str) -> std::result::Result<(), String> {
        let inner = args.trim().trim_start_matches('(').trim_end_matches(')');
        let parts: Vec<&str> = inner.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
        let num = |s: &str| -> std::result::Result<u64, String> {
            match s.parse::<u64>() {
                Ok(n) if [1, 2, 4, 8, 16].contains(&n) => Ok(n),
                _ => Err(format!("invalid pack value '{s}'")),
            }
        };
        match parts.first().copied() {
            None => self.pack = None,
            Some("push") => {
                self.pack_stack.push(self.pack);
                if let Some(n) = parts.iter().skip(1).find(|s| s.parse::<u64>().is_ok()) {
                    self.pack = Some(num(n)?);
                }
            }
            Some("pop") => self.pack = self.pack_stack.pop().unwrap_or(None),
            Some(n) => self.pack = Some(num(n)?),
        }
        Ok(())
    }

    // ------------------------------------------------------------------ types

    fn starts_type(&self) -> bool {
        match self.peek_ident() {
            Some(s) => {
                matches!(s, "struct" | "union" | "enum")
                    || TYPE_KEYWORDS.contains(&s)
                    || QUALIFIERS.contains(&s)
                    || self.t.typedefs.contains_key(s)
                    || self.t.struct_tags.contains_key(s)
                    || self.t.enum_tags.contains_key(s)
                    || builtin_type(s).is_some()
            }
            None => false,
        }
    }

    /// Returns the type and whether it is an anonymous inline struct/union.
    fn parse_type_spec(&mut self) -> Result<(TypeSpec, bool)> {
        self.skip_qualifiers();
        let spec = match self.peek().clone() {
            Tok::Ident(s) if s == "struct" || s == "union" => {
                self.next();
                return self.parse_struct(s == "union");
            }
            Tok::Ident(s) if s == "enum" => {
                self.next();
                self.parse_enum()?
            }
            Tok::Ident(s) if TYPE_KEYWORDS.contains(&s.as_str()) => {
                let mut words = Vec::new();
                while let Some(w) = self.peek_ident() {
                    if TYPE_KEYWORDS.contains(&w) {
                        words.push(w.to_string());
                        self.next();
                    } else if QUALIFIERS.contains(&w) {
                        self.next();
                    } else {
                        break;
                    }
                }
                let ty = keyword_type(&words).map_err(|e| self.err(e))?;
                TypeSpec { ty, name: words.join(" "), dims: vec![] }
            }
            Tok::Ident(s) => {
                if let Some(ts) = self.t.typedefs.get(&s) {
                    let ts = ts.clone();
                    self.next();
                    ts
                } else if let Some(p) = builtin_type(&s) {
                    self.next();
                    TypeSpec { ty: Ty::Prim(p), name: s, dims: vec![] }
                } else if let Some(&i) = self.t.struct_tags.get(&s) {
                    // C++ style: tags are type names
                    self.next();
                    TypeSpec { ty: Ty::Struct(i), name: s, dims: vec![] }
                } else if let Some(&i) = self.t.enum_tags.get(&s) {
                    self.next();
                    TypeSpec { ty: Ty::Enum(i), name: s, dims: vec![] }
                } else {
                    return Err(self.err(format!("unknown type '{s}'")));
                }
            }
            other => return Err(self.err(format!("expected a type, found {}", describe(&other)))),
        };
        self.skip_qualifiers();
        Ok((spec, false))
    }

    fn parse_struct(&mut self, is_union: bool) -> Result<(TypeSpec, bool)> {
        let kw = if is_union { "union" } else { "struct" };
        let tag = match self.peek_ident() {
            Some(s) if !TYPE_KEYWORDS.contains(&s) => {
                let s = s.to_string();
                self.next();
                Some(s)
            }
            _ => None,
        };

        if !self.is_punct("{") {
            let tag = tag.ok_or_else(|| self.err(format!("expected '{{' or a tag after '{kw}'")))?;
            let idx = match self.t.struct_tags.get(&tag) {
                Some(&i) => i,
                None => self.new_struct(&tag, is_union),
            };
            self.skip_qualifiers();
            return Ok((TypeSpec { ty: Ty::Struct(idx), name: tag, dims: vec![] }, false));
        }

        let idx = match &tag {
            Some(t) => match self.t.struct_tags.get(t) {
                Some(&i) if self.t.structs[i].defined => {
                    return Err(self.err(format!("redefinition of '{kw} {t}'")));
                }
                Some(&i) => i,
                None => self.new_struct(t, is_union),
            },
            None => self.new_struct("", is_union),
        };
        self.t.structs[idx].is_union = is_union;
        let pack = self.pack;
        self.next(); // {
        let fields = self.parse_struct_body()?;

        let max = pack.unwrap_or(u64::MAX);
        let align = fields.iter().map(|f| self.t.align_of(&f.ty).min(max)).max().unwrap_or(1).max(1);
        {
            let s = &mut self.t.structs[idx];
            s.fields = fields;
            s.pack = pack;
            s.align = align;
            s.defined = true;
        }
        self.t.structs[idx].static_size = static_struct_size(&self.t, idx);
        if let Some(t) = &tag {
            self.add_root_name(t);
        }
        self.skip_qualifiers();
        let name = tag.clone().unwrap_or_else(|| kw.to_string());
        Ok((TypeSpec { ty: Ty::Struct(idx), name, dims: vec![] }, tag.is_none()))
    }

    fn new_struct(&mut self, tag: &str, is_union: bool) -> usize {
        let idx = self.t.structs.len();
        self.t.structs.push(StructDef {
            name: tag.to_string(),
            is_union,
            fields: vec![],
            pack: None,
            defined: false,
            static_size: None,
            align: 1,
        });
        if !tag.is_empty() {
            self.t.struct_tags.insert(tag.to_string(), idx);
        }
        idx
    }

    fn parse_struct_body(&mut self) -> Result<Vec<Field>> {
        let mut fields = Vec::new();
        loop {
            match self.peek().clone() {
                Tok::Punct("}") => {
                    self.next();
                    return Ok(fields);
                }
                Tok::Punct(";") => {
                    self.next();
                }
                Tok::Directive(d) => {
                    let tok = self.next();
                    self.directive(&d, &tok)?;
                }
                Tok::Eof => return Err(self.err("missing '}'")),
                _ => {
                    let line = self.tok().line;
                    let (spec, anon) = self.parse_type_spec()?;
                    if self.eat_punct(";") {
                        if anon {
                            fields.push(Field {
                                name: None,
                                ty: spec.ty,
                                type_name: spec.name,
                                dims: vec![],
                                bits: None,
                                endian: self.endian,
                                at: None,
                                line,
                            });
                        }
                        continue;
                    }
                    loop {
                        fields.push(self.parse_declarator(&spec, line, true)?);
                        if self.eat_punct(",") {
                            continue;
                        }
                        self.expect_punct(";")?;
                        break;
                    }
                }
            }
        }
    }

    fn parse_enum(&mut self) -> Result<TypeSpec> {
        let tag = match self.peek_ident() {
            Some(s) if s != "class" => {
                let s = s.to_string();
                self.next();
                Some(s)
            }
            Some(_) => {
                // C++ `enum class Name`
                self.next();
                Some(self.expect_ident()?)
            }
            None => None,
        };
        let mut base = Prim::I32;
        if self.eat_punct(":") {
            let (ts, _) = self.parse_type_spec()?;
            base = match ts.ty {
                Ty::Prim(p) if p.is_integer() => p,
                Ty::Enum(i) => self.t.enums[i].base,
                _ => return Err(self.err("enum base must be an integer type")),
            };
        }
        if !self.eat_punct("{") {
            let tag = tag.ok_or_else(|| self.err("expected '{' after 'enum'"))?;
            let idx = *self
                .t
                .enum_tags
                .get(&tag)
                .ok_or_else(|| self.err(format!("unknown enum '{tag}'")))?;
            return Ok(TypeSpec { ty: Ty::Enum(idx), name: tag, dims: vec![] });
        }
        let mut items = Vec::new();
        let mut next_val: i128 = 0;
        loop {
            if self.eat_punct("}") {
                break;
            }
            let name = self.expect_ident()?;
            let v = if self.eat_punct("=") {
                let e = self.parse_expr()?;
                const_eval(&self.t, &e).map_err(|m| self.err(m))?
            } else {
                next_val
            };
            self.t.consts.insert(name.clone(), v);
            items.push((name, v));
            next_val = v.wrapping_add(1);
            if !self.eat_punct(",") {
                self.expect_punct("}")?;
                break;
            }
        }
        let idx = self.t.enums.len();
        let name = tag.clone().unwrap_or_else(|| "enum".to_string());
        self.t.enums.push(EnumDef { name: name.clone(), base, items });
        if let Some(t) = &tag {
            self.t.enum_tags.insert(t.clone(), idx);
            self.add_root_name(t);
        }
        Ok(TypeSpec { ty: Ty::Enum(idx), name, dims: vec![] })
    }

    /// Parses `*name[dims] : bits`. `is_object` declarations need a complete type.
    fn parse_declarator(&mut self, spec: &TypeSpec, line: usize, is_object: bool) -> Result<Field> {
        let mut ty = spec.ty.clone();
        let mut type_name = spec.name.clone();
        let mut is_ptr = false;
        while self.eat_punct("*") {
            is_ptr = true;
            type_name.push('*');
            self.skip_qualifiers();
        }
        if is_ptr {
            ty = Ty::Pointer(self.ptr_size);
        }
        let name = match self.peek_ident() {
            Some(s) => {
                let s = s.to_string();
                self.next();
                Some(s)
            }
            None => None,
        };
        let mut dims = Vec::new();
        while self.eat_punct("[") {
            if self.is_punct("]") {
                return Err(self.err("array size required"));
            }
            dims.push(self.parse_expr()?);
            self.expect_punct("]")?;
        }
        if !is_ptr {
            dims.extend(spec.dims.iter().cloned());
        }
        let bits = if self.eat_punct(":") { Some(self.parse_expr()?) } else { None };
        if name.is_none() && bits.is_none() {
            return Err(self.err(format!("expected a name, found {}", describe(self.peek()))));
        }
        if is_object {
            match ty {
                Ty::Struct(i) if !self.t.structs[i].defined => {
                    return Err(TemplateError::new(
                        line,
                        1,
                        format!("'{}' is an incomplete type here", self.t.structs[i].name),
                    ));
                }
                Ty::Void => return Err(TemplateError::new(line, 1, "a field cannot have type void")),
                _ => {}
            }
        }
        Ok(Field { name, ty, type_name, dims, bits, endian: self.endian, at: None, line })
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
        match self.peek().clone() {
            Tok::Num(n) => {
                self.next();
                Ok(Expr::Num(n))
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
                if !self.starts_type() {
                    return Err(self.err("sizeof expects a type"));
                }
                let line = self.tok().line;
                let (spec, _) = self.parse_type_spec()?;
                let mut ts = spec.clone();
                if self.is_punct("*") || self.is_punct("[") {
                    // abstract declarator: sizeof(T*) / sizeof(T[4])
                    let mut ptr = false;
                    while self.eat_punct("*") {
                        ptr = true;
                    }
                    if ptr {
                        ts.ty = Ty::Pointer(self.ptr_size);
                        ts.dims.clear();
                    }
                    let mut dims = Vec::new();
                    while self.eat_punct("[") {
                        dims.push(self.parse_expr()?);
                        self.expect_punct("]")?;
                    }
                    dims.extend(ts.dims);
                    ts.dims = dims;
                }
                if let Ty::Struct(i) = ts.ty {
                    if !self.t.structs[i].defined {
                        return Err(TemplateError::new(line, 1, "sizeof of incomplete type"));
                    }
                }
                self.expect_punct(")")?;
                Ok(Expr::SizeOf(Box::new(ts)))
            }
            Tok::Ident(s) => {
                self.next();
                let mut path = vec![s];
                while self.eat_punct(".") {
                    path.push(self.expect_ident()?);
                }
                Ok(Expr::Path(path))
            }
            other => Err(self.err(format!("expected an expression, found {}", describe(&other)))),
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

fn split_word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    if end == 0 {
        // e.g. "pack(1)" handled by caller; return the whole thing as rest
        return ("", s);
    }
    (&s[..end], &s[end..])
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => format!("'{s}'"),
        Tok::Num(n) => format!("'{n}'"),
        Tok::Punct(p) => format!("'{p}'"),
        Tok::Directive(_) => "a directive".into(),
        Tok::Eof => "end of input".into(),
    }
}
