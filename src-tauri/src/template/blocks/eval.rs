use std::collections::HashMap;
use std::rc::Rc;

use encoding_rs::Encoding;

use super::super::eval::{sign_extend, Node, NodeLink, Options, Root, MAX_CHILDREN, MAX_NODES, MAX_STRING_CHARS};
use super::*;

/// Decoded text kept for comparisons; longer strings are cut.
const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Bytes shown for raw sections and unknown blocks.
const PREVIEW_BYTES: u64 = 16;

/// A decoded field as seen by expressions.
#[derive(Debug, Clone)]
enum Val {
    Int(i128),
    Enum(i128, usize),
    Flags(i128, usize),
    Float(f64),
    Str(Rc<str>),
    Struct(Rc<HashMap<String, Val>>),
    Array,
    /// in an untaken branch, a missing slot, or unreadable
    Absent,
}

pub fn apply(t: &Template, data: &[u8], root: Root, base: u64, opts: &Options) -> Result<Node, String> {
    let mut d = Dec::new(t, data, t.endian.unwrap_or(opts.endian), opts.encoding, false);
    let mut node = match root {
        Root::Instances => {
            let items = t.file.as_ref().ok_or("the template has no 'file { }'")?;
            d.scopes.push(HashMap::new());
            let mut cur = base;
            let mut children = Vec::new();
            d.items(items, &mut cur, d.data_len(), &mut children, true);
            let start = children.iter().filter(|c| c.size > 0).map(|c| c.offset).min().unwrap_or(base);
            let end = children.iter().map(|c| c.offset.saturating_add(c.size)).max().unwrap_or(base);
            let mut root = Node::new("template", "", start, end.saturating_sub(start), String::new());
            root.children = children;
            root
        }
        Root::ByMagic => d.by_magic(base)?,
        Root::Type(name) => {
            let targets = t.targets(&name).ok_or_else(|| format!("unknown type '{name}'"))?;
            d.follow(&targets, &name, base)
        }
    };
    if d.stopped {
        set_error(&mut node, format!("evaluation limit of {MAX_NODES} nodes reached"));
    }
    Ok(node)
}

/// Evaluates an expression that may only use constants.
pub fn const_eval(t: &Template, e: &Expr) -> Result<Value, String> {
    Dec::new(t, &[], Endian::Little, encoding_rs::UTF_8, true).eval(e)
}

/// Size of a type when it does not depend on decoded data.
pub fn static_size(t: &Template, ty: &Type) -> Option<u64> {
    Dec::new(t, &[], Endian::Little, encoding_rs::UTF_8, true).static_size(ty)
}

fn set_error(node: &mut Node, msg: String) {
    match &mut node.error {
        Some(e) => {
            e.push_str("; ");
            e.push_str(&msg);
        }
        None => node.error = Some(msg),
    }
}

fn has_error(n: &Node) -> bool {
    n.error.is_some() || n.children.iter().any(has_error)
}

fn align_up(v: u64, a: u64) -> u64 {
    if a <= 1 {
        v
    } else {
        v.div_ceil(a).saturating_mul(a)
    }
}

fn hex_preview(b: &[u8], total: u64) -> String {
    let mut s: String = b.iter().take(PREVIEW_BYTES as usize).map(|x| format!("{x:02X} ")).collect();
    s.pop();
    if total > PREVIEW_BYTES {
        s.push_str(" …");
    }
    s
}

/// Trailing NUL and space characters do not count in string comparisons (8.3).
fn trim(s: &str) -> &str {
    s.trim_end_matches(['\0', ' '])
}

fn str_kind_name(kind: StrKind, endian: Option<Endian>) -> String {
    let base = match kind {
        StrKind::Ascii => "ascii",
        StrKind::Utf8 => "utf8",
        StrKind::Cp => "cp",
        StrKind::Utf16 => "utf16",
        StrKind::Utf32 => "utf32",
    };
    match endian {
        Some(Endian::Little) => format!("{base}le"),
        Some(Endian::Big) => format!("{base}be"),
        None => base.into(),
    }
}

/// Quotes a string for display; `ascii` shows bytes ≥ 0x80 as `\xHH`.
fn quote(s: &str, ascii: bool) -> String {
    let mut out = String::from("\"");
    for (i, c) in s.chars().enumerate() {
        if i >= MAX_STRING_CHARS {
            out.push('…');
            break;
        }
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() || (ascii && c as u32 >= 0x80) => out.push_str(&format!("\\x{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Expression text for the type column.
fn expr_text(e: &Expr) -> String {
    match e {
        Expr::Num(n) => n.to_string(),
        Expr::Str(s) => format!("{s:?}"),
        Expr::Path(p, _) => p.join("."),
        Expr::Unary(op, a) => {
            let o = match op {
                UnOp::Neg => "-",
                UnOp::Not => "!",
                UnOp::BitNot => "~",
            };
            format!("{o}{}", expr_text(a))
        }
        Expr::Binary(op, a, b) => {
            use BinOp::*;
            let o = match op {
                Mul => "*",
                Div => "/",
                Rem => "%",
                Add => "+",
                Sub => "-",
                Shl => "<<",
                Shr => ">>",
                Lt => "<",
                Gt => ">",
                Le => "<=",
                Ge => ">=",
                Eq => "==",
                Ne => "!=",
                BitAnd => "&",
                BitXor => "^",
                BitOr => "|",
                And => "&&",
                Or => "||",
            };
            format!("({} {o} {})", expr_text(a), expr_text(b))
        }
        Expr::Cond(c, a, b) => format!("({} ? {} : {})", expr_text(c), expr_text(a), expr_text(b)),
        Expr::SizeOf(..) => "sizeof(…)".into(),
    }
}

struct Dec<'a> {
    t: &'a Template,
    data: &'a [u8],
    endian: Endian,
    enc: &'static Encoding,
    scopes: Vec<HashMap<String, Val>>,
    /// blocks whose frame parameters are visible, innermost last
    blocks: Vec<usize>,
    nodes: u64,
    stopped: bool,
    /// constants only: no data and no fields
    konst: bool,
}

impl<'a> Dec<'a> {
    fn new(t: &'a Template, data: &'a [u8], endian: Endian, enc: &'static Encoding, konst: bool) -> Self {
        Dec { t, data, endian, enc, scopes: Vec::new(), blocks: Vec::new(), nodes: 0, stopped: false, konst }
    }

    fn data_len(&self) -> u64 {
        self.data.len() as u64
    }

    /// Counts a node; false once the evaluation limit is reached.
    fn tick(&mut self) -> bool {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            self.stopped = true;
        }
        !self.stopped
    }

    fn bytes(&self, offset: u64, len: u64) -> Option<&'a [u8]> {
        let end = offset.checked_add(len)?;
        if end > self.data_len() {
            return None;
        }
        Some(&self.data[offset as usize..end as usize])
    }

    /// The bytes of `offset..offset + len` that exist.
    fn avail(&self, offset: u64, len: u64) -> &'a [u8] {
        let n = self.data_len();
        if offset >= n {
            return &[];
        }
        &self.data[offset as usize..offset.saturating_add(len).min(n) as usize]
    }

    fn read_uint(&self, offset: u64, size: u64, endian: Endian) -> Option<u128> {
        let b = self.bytes(offset, size)?;
        Some(match endian {
            Endian::Little => b.iter().rev().fold(0u128, |a, &x| (a << 8) | x as u128),
            Endian::Big => b.iter().fold(0u128, |a, &x| (a << 8) | x as u128),
        })
    }

    fn bound_error(&self, end: u64, bound: u64, extends: bool) -> Option<String> {
        let pre = if extends { "extends " } else { "" };
        if end > self.data_len() {
            Some(format!("{pre}beyond end of data"))
        } else if end > bound {
            Some(format!("{pre}beyond end of section"))
        } else {
            None
        }
    }

    // ------------------------------------------------------------ expressions

    fn eval(&self, e: &Expr) -> Result<Value, String> {
        let int = |v: Value| match v {
            Value::Int(i) => Ok(i),
            Value::Str(_) => Err("type mismatch in expression".to_string()),
        };
        Ok(match e {
            Expr::Num(n) => Value::Int(*n),
            Expr::Str(s) => Value::Str(s.clone()),
            Expr::Path(p, _) => self.lookup(p)?,
            Expr::Unary(op, a) => {
                let a = int(self.eval(a)?)?;
                Value::Int(match op {
                    UnOp::Neg => a.wrapping_neg(),
                    UnOp::Not => (a == 0) as i128,
                    UnOp::BitNot => !a,
                })
            }
            Expr::Binary(op, a, b) => {
                let a = self.eval(a)?;
                // short-circuit like C
                match (op, &a) {
                    (BinOp::And, Value::Int(0)) => return Ok(Value::Int(0)),
                    (BinOp::Or, Value::Int(x)) if *x != 0 => return Ok(Value::Int(1)),
                    _ => {}
                }
                let b = self.eval(b)?;
                if let (BinOp::Eq | BinOp::Ne, Value::Str(x), Value::Str(y)) = (op, &a, &b) {
                    let eq = trim(x) == trim(y);
                    return Ok(Value::Int((eq == (*op == BinOp::Eq)) as i128));
                }
                let (a, b) = (int(a)?, int(b)?);
                Value::Int(match op {
                    BinOp::Mul => a.wrapping_mul(b),
                    BinOp::Div => a.checked_div(b).ok_or("division by zero")?,
                    BinOp::Rem => a.checked_rem(b).ok_or("division by zero")?,
                    BinOp::Add => a.wrapping_add(b),
                    BinOp::Sub => a.wrapping_sub(b),
                    BinOp::Shl => a.wrapping_shl(b.clamp(0, 127) as u32),
                    BinOp::Shr => a.wrapping_shr(b.clamp(0, 127) as u32),
                    BinOp::Lt => (a < b) as i128,
                    BinOp::Gt => (a > b) as i128,
                    BinOp::Le => (a <= b) as i128,
                    BinOp::Ge => (a >= b) as i128,
                    BinOp::Eq => (a == b) as i128,
                    BinOp::Ne => (a != b) as i128,
                    BinOp::BitAnd => a & b,
                    BinOp::BitXor => a ^ b,
                    BinOp::BitOr => a | b,
                    BinOp::And | BinOp::Or => (b != 0) as i128,
                })
            }
            Expr::Cond(c, a, b) => {
                if int(self.eval(c)?)? != 0 {
                    self.eval(a)?
                } else {
                    self.eval(b)?
                }
            }
            Expr::SizeOf(ty, _) => Value::Int(
                static_size(self.t, ty).ok_or("sizeof of a type whose size depends on file data")? as i128,
            ),
        })
    }

    fn enum_item(&self, e: usize, item: &str) -> Option<i128> {
        self.t.enums[e].items.iter().find(|(n, _)| n == item).map(|(_, v)| *v)
    }

    fn find_field(&self, name: &str) -> Option<&Val> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    /// Name lookup (9).
    fn lookup(&self, path: &[String]) -> Result<Value, String> {
        let first = &path[0];
        if let Some(v) = self.find_field(first) {
            return self.select(v.clone(), path);
        }
        let t = self.t;
        if path.len() == 1 {
            if let Some(&b) = self.blocks.last() {
                let blk = &t.blocks[b];
                if let Some(i) = t.frames[blk.frame].params.iter().position(|p| p == first) {
                    return Ok(blk.args[i].clone());
                }
            }
            if let Some(c) = t.consts.get(first) {
                return Ok(c.clone());
            }
            if let Some(es) = t.enum_items.get(first) {
                if es.len() > 1 {
                    return Err(format!("ambiguous enum item '{first}'"));
                }
                if let Some(v) = self.enum_item(es[0], first) {
                    return Ok(Value::Int(v));
                }
            }
            if first == "FILE_SIZE" {
                if self.konst {
                    return Err("FILE_SIZE is not a constant".into());
                }
                return Ok(Value::Int(self.data.len() as i128));
            }
        } else if let (2, Some(Decl::Enum(e))) = (path.len(), t.names.get(first)) {
            return self
                .enum_item(*e, &path[1])
                .map(Value::Int)
                .ok_or_else(|| format!("enum '{first}' has no item '{}'", path[1]));
        }
        Err(format!("unknown identifier '{first}'"))
    }

    fn select(&self, mut v: Val, path: &[String]) -> Result<Value, String> {
        for (k, seg) in path.iter().enumerate().skip(1) {
            let so_far = || path[..k].join(".");
            v = match v {
                Val::Struct(m) => m.get(seg).cloned().ok_or_else(|| format!("'{}' has no field '{seg}'", so_far()))?,
                Val::Flags(raw, f) => match self.t.flags[f].bits.iter().find(|(n, _)| n == seg) {
                    Some((_, bit)) => Val::Int((raw >> bit) & 1),
                    None => return Err(format!("'{}' has no flag '{seg}'", so_far())),
                },
                Val::Absent => return Err(format!("'{}' is not present", so_far())),
                _ => return Err(format!("'{}' has no field '{seg}'", so_far())),
            };
        }
        let name = || path.join(".");
        match v {
            Val::Int(i) | Val::Enum(i, _) | Val::Flags(i, _) => Ok(Value::Int(i)),
            Val::Float(f) if f.is_finite() => Ok(Value::Int(f.trunc() as i128)),
            Val::Float(_) => Err("non-finite value in expression".into()),
            Val::Str(s) => Ok(Value::Str(s.to_string())),
            Val::Struct(_) => Err(format!("'{}' is a struct, not a value", name())),
            Val::Array => Err(format!("'{}' is an array, not a value", name())),
            Val::Absent => Err(format!("'{}' is not present", name())),
        }
    }

    /// An array or section size: a non-negative integer.
    fn count(&self, e: &Expr, what: &str) -> Result<u64, String> {
        match self.eval(e)? {
            Value::Int(n) if n < 0 => Err(format!("negative {what} size {n}")),
            Value::Int(n) => Ok(n.min(u64::MAX as i128) as u64),
            Value::Str(_) => Err("type mismatch in expression".into()),
        }
    }

    fn static_size(&self, ty: &Type) -> Option<u64> {
        match ty {
            Type::Prim(p) => Some(p.prim.size()),
            Type::Enum(i) => Some(self.t.enums[*i].base.prim.size()),
            Type::Flags(i) => Some(self.t.flags[*i].base.prim.size()),
            Type::Str(kind, _, Some(len)) => self.count(len, "array").ok()?.checked_mul(kind.unit()),
            Type::Array(elem, Some(n)) => self.static_size(elem)?.checked_mul(self.count(n, "array").ok()?),
            Type::Struct(i) => {
                let mut total = 0u64;
                for it in &self.t.structs[*i].items {
                    match it {
                        Item::Field(f) if f.at.is_none() => total = total.checked_add(self.static_size(&f.ty)?)?,
                        _ => return None,
                    }
                }
                Some(total)
            }
            _ => None,
        }
    }

    // ------------------------------------------------------------- scalars

    fn type_name(&self, ty: &Type) -> String {
        let t = self.t;
        match ty {
            Type::Prim(p) => p.name(),
            Type::Str(k, e, len) => match len {
                Some(l) => format!("{}({})", str_kind_name(*k, *e), expr_text(l)),
                None => str_kind_name(*k, *e),
            },
            Type::Array(elem, n) => match n {
                Some(n) => format!("[{}; {}]", self.type_name(elem), expr_text(n)),
                None => format!("[{}; ..]", self.type_name(elem)),
            },
            Type::Link(_, text) => format!("-> {text}"),
            Type::Struct(i) => t.structs[*i].name.clone(),
            Type::Enum(i) => t.enums[*i].name.clone(),
            Type::Flags(i) => t.flags[*i].name.clone(),
            Type::Block(i) => t.blocks[*i].name.clone(),
            Type::Named(n, _) => n.clone(),
            Type::LinkNames(l) => l.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(" | "),
        }
    }

    fn scalar_prim(&self, ty: &Type) -> Option<PrimType> {
        match ty {
            Type::Prim(p) => Some(*p),
            Type::Enum(i) => Some(self.t.enums[*i].base),
            Type::Flags(i) => Some(self.t.flags[*i].base),
            _ => None,
        }
    }

    /// Display text and value of an integer, float, enum or flags type.
    fn scalar(&self, ty: &Type, raw: u128) -> (String, Val) {
        let p = self.scalar_prim(ty).expect("scalar type").prim;
        match ty {
            Type::Enum(i) => {
                let v = sign_extend(raw, p);
                match self.t.enums[*i].items.iter().find(|(_, x)| *x == v) {
                    Some((n, _)) => (format!("{n} ({v})"), Val::Enum(v, *i)),
                    None => (v.to_string(), Val::Enum(v, *i)),
                }
            }
            Type::Flags(i) => {
                let f = &self.t.flags[*i];
                let mut names = Vec::new();
                for bit in 0..p.size() * 8 {
                    if (raw >> bit) & 1 == 1 {
                        match f.bits.iter().find(|(_, b)| *b as u64 == bit) {
                            Some((n, _)) => names.push(n.clone()),
                            None => names.push(format!("bit {bit}")),
                        }
                    }
                }
                (format!("0x{raw:X} {{{}}}", names.join(", ")), Val::Flags(raw as i128, *i))
            }
            _ => match p {
                Prim::F32 => {
                    let f = f32::from_bits(raw as u32);
                    (crate::inspector::fmt_float(f), Val::Float(f as f64))
                }
                Prim::F64 => {
                    let f = f64::from_bits(raw as u64);
                    (crate::inspector::fmt_float(f), Val::Float(f))
                }
                _ => {
                    let v = sign_extend(raw, p);
                    let w = p.size() as usize * 2;
                    (format!("{v} (0x{raw:0w$X})"), Val::Int(v))
                }
            },
        }
    }

    fn decode_scalar(&mut self, name: &str, ty: &Type, off: u64, bound: u64) -> (Node, Val) {
        self.tick();
        let p = self.scalar_prim(ty).expect("scalar type");
        let size = p.prim.size();
        let mut node = Node::new(name, &self.type_name(ty), off, size, String::new());
        let Some(raw) = self.read_uint(off, size, p.endian.unwrap_or(self.endian)) else {
            node.error = Some("beyond end of data".into());
            return (node, Val::Absent);
        };
        let (text, val) = self.scalar(ty, raw);
        node.value = text;
        node.error = self.bound_error(off + size, bound, false);
        (node, val)
    }

    // ------------------------------------------------------------- fields

    fn decode_type(&mut self, name: &str, ty: &Type, off: u64, bound: u64) -> (Node, Val) {
        match ty {
            Type::Prim(_) | Type::Enum(_) | Type::Flags(_) => self.decode_scalar(name, ty, off, bound),
            Type::Str(kind, endian, len) => self.decode_string(name, *kind, *endian, len.as_deref(), off, bound),
            Type::Array(elem, count) => self.decode_array(name, elem, count.as_deref(), off, bound),
            Type::Struct(i) => self.decode_struct(name, *i, off, bound),
            Type::Block(i) => {
                let (node, vars, _) = self.decode_block(*i, off, name, bound);
                (node, Val::Struct(Rc::new(vars)))
            }
            // rejected by the parser outside slot sections
            Type::Link(..) | Type::Named(..) | Type::LinkNames(_) => {
                (Node::error(name, off, "link outside a slot section".into()), Val::Absent)
            }
        }
    }

    fn decode_text(&self, b: &[u8], kind: StrKind, endian: Endian) -> String {
        let b = &b[..b.len().min(MAX_TEXT_BYTES)];
        match kind {
            // bytes >= 0x80 keep their value, quote() shows them as \xHH
            StrKind::Ascii => b.iter().map(|&x| x as char).collect(),
            StrKind::Utf8 => String::from_utf8_lossy(b).into_owned(),
            StrKind::Cp => self.enc.decode_without_bom_handling(b).0.into_owned(),
            StrKind::Utf16 => {
                let units: Vec<u16> = b
                    .chunks_exact(2)
                    .map(|c| match endian {
                        Endian::Little => u16::from_le_bytes([c[0], c[1]]),
                        Endian::Big => u16::from_be_bytes([c[0], c[1]]),
                    })
                    .collect();
                String::from_utf16_lossy(&units)
            }
            StrKind::Utf32 => b
                .chunks_exact(4)
                .map(|c| {
                    let u = match endian {
                        Endian::Little => u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                        Endian::Big => u32::from_be_bytes([c[0], c[1], c[2], c[3]]),
                    };
                    char::from_u32(u).unwrap_or('\u{FFFD}')
                })
                .collect(),
        }
    }

    fn decode_string(
        &mut self,
        name: &str,
        kind: StrKind,
        endian: Option<Endian>,
        len: Option<&Expr>,
        off: u64,
        bound: u64,
    ) -> (Node, Val) {
        self.tick();
        let unit = kind.unit();
        let kname = str_kind_name(kind, endian);
        // position of the first NUL code unit, in bytes
        let nul = |b: &[u8]| b.chunks(unit as usize).position(|c| c.iter().all(|&x| x == 0)).map(|i| i * unit as usize);
        let (tn, size, text_bytes) = match len {
            Some(e) => {
                let n = match self.count(e, "array") {
                    Ok(n) => n,
                    Err(m) => return (Node::error(name, off, m), Val::Absent),
                };
                let total = n.saturating_mul(unit);
                let b = self.avail(off, total);
                let cut = nul(b).unwrap_or(b.len() - b.len() % unit as usize);
                (format!("{kname}({n})"), total, &b[..cut])
            }
            None => {
                let limit = bound.min(self.data_len());
                let b = self.avail(off, limit.saturating_sub(off));
                let b = &b[..b.len() - b.len() % unit as usize];
                match nul(b) {
                    Some(cut) => (kname, cut as u64 + unit, &b[..cut]),
                    None => (kname, b.len() as u64, b),
                }
            }
        };
        let text = self.decode_text(text_bytes, kind, endian.unwrap_or(self.endian));
        let mut node = Node::new(name, &tn, off, size, quote(&text, kind == StrKind::Ascii));
        node.error = self.bound_error(off.saturating_add(size), bound, false);
        if size == 0 && len.is_none() && off >= self.data_len() {
            node.error = Some("beyond end of data".into());
        }
        (node, Val::Str(text.into()))
    }

    fn decode_array(&mut self, name: &str, elem: &Type, count: Option<&Expr>, off: u64, bound: u64) -> (Node, Val) {
        if !self.tick() {
            return (Node::error(name, off, "evaluation limit reached".into()), Val::Array);
        }
        let es = static_size(self.t, elem);
        let elem_name = self.type_name(elem);
        let mut children = Vec::new();
        let mut cur = off;
        let mut more = 0u64;
        let n = match count {
            Some(e) => {
                let n = match self.count(e, "array") {
                    Ok(n) => n,
                    Err(m) => {
                        let mut node = Node::new(name, &format!("[{elem_name}; {}]", expr_text(e)), off, 0, String::new());
                        node.error = Some(m);
                        return (node, Val::Array);
                    }
                };
                for i in 0..n {
                    if self.stopped {
                        break;
                    }
                    if i >= MAX_CHILDREN {
                        if let Some(es) = es {
                            cur = cur.saturating_add(es.saturating_mul(n - i));
                            more += n - i;
                            break;
                        }
                        let (node, _) = self.decode_type("", elem, cur, bound);
                        cur = cur.saturating_add(node.size);
                        more += 1;
                        continue;
                    }
                    let (node, _) = self.decode_type(&format!("[{i}]"), elem, cur, bound);
                    cur = cur.saturating_add(node.size);
                    children.push(node);
                }
                n
            }
            None => {
                let limit = bound.min(self.data_len());
                let mut i = 0u64;
                while cur < limit && !self.stopped {
                    if i >= MAX_CHILDREN {
                        if let Some(es) = es {
                            let k = (limit - cur).div_ceil(es);
                            cur = cur.saturating_add(k.saturating_mul(es));
                            more += k;
                            i += k;
                            break;
                        }
                    }
                    let (node, _) = self.decode_type(&format!("[{i}]"), elem, cur, bound);
                    let sz = node.size;
                    if i < MAX_CHILDREN {
                        children.push(node);
                    } else {
                        more += 1;
                    }
                    i += 1;
                    if sz == 0 {
                        break;
                    }
                    cur = cur.saturating_add(sz);
                }
                i
            }
        };
        let size = cur - off;
        let value = match elem {
            Type::Prim(PrimType { prim: Prim::U8 | Prim::I8, .. }) => hex_preview(self.avail(off, size), size),
            _ => format!("[{n}]"),
        };
        let mut node = Node::new(name, &format!("[{elem_name}; {n}]"), off, size, value);
        node.children = children;
        node.more = more;
        node.error = self.bound_error(off.saturating_add(size), bound, true);
        (node, Val::Array)
    }

    fn decode_struct(&mut self, name: &str, si: usize, off: u64, bound: u64) -> (Node, Val) {
        self.tick();
        let def = &self.t.structs[si];
        self.scopes.push(HashMap::new());
        let mut cur = off;
        let mut children = Vec::new();
        self.items(&def.items, &mut cur, bound, &mut children, false);
        let vars = self.scopes.pop().unwrap();
        let mut node = Node::new(name, &def.name, off, cur - off, String::new());
        node.children = children;
        (node, Val::Struct(Rc::new(vars)))
    }

    fn bind(&mut self, name: &str, v: Val) {
        if name != "_" {
            self.scopes.last_mut().unwrap().insert(name.to_string(), v);
        }
    }

    fn mark_absent(&mut self, names: &[String]) {
        let scope = self.scopes.last_mut().unwrap();
        for n in names {
            scope.entry(n.clone()).or_insert(Val::Absent);
        }
    }

    fn check(&self, f: &Field, node: &mut Node) {
        if let Some(c) = &f.check {
            if !matches!(self.eval(&c.expr), Ok(Value::Int(v)) if v != 0) {
                set_error(node, format!("check failed: {}", c.text));
            }
        }
    }

    /// The branch of an `if` or `match` to decode. Evaluation errors become warning rows.
    fn choose<'i>(&self, it: &'i Item, at: u64, out: &mut Vec<Node>) -> Option<&'i [Item]> {
        match it {
            Item::Field(_) => None,
            Item::If { branches, otherwise, .. } => {
                for (c, body) in branches {
                    match self.eval(c) {
                        Ok(Value::Int(v)) if v != 0 => return Some(body),
                        Ok(Value::Int(_)) => {}
                        Ok(Value::Str(_)) => out.push(Node::error("if", at, "type mismatch in expression".into())),
                        Err(m) => out.push(Node::error("if", at, m)),
                    }
                }
                otherwise.as_deref()
            }
            Item::Match { expr, arms, .. } => {
                // the enum of the matched value, for bare item names in patterns
                let ctx = match expr {
                    Expr::Path(p, _) if p.len() == 1 => match self.find_field(&p[0]) {
                        Some(Val::Enum(_, e)) => Some(*e),
                        _ => None,
                    },
                    _ => None,
                };
                let v = match self.eval(expr) {
                    Ok(v) => v,
                    Err(m) => {
                        out.push(Node::error("match", at, m));
                        return None;
                    }
                };
                for arm in arms {
                    let hit = arm.patterns.iter().any(|p| match p {
                        Pattern::Wild => true,
                        Pattern::Expr(e) => {
                            let pv = match (e, ctx) {
                                (Expr::Path(p, _), Some(en)) if p.len() == 1 => {
                                    self.enum_item(en, &p[0]).map(Value::Int).ok_or(()).or_else(|_| self.eval(e).map_err(|_| ()))
                                }
                                _ => self.eval(e).map_err(|_| ()),
                            };
                            match (pv, &v) {
                                (Ok(Value::Int(a)), Value::Int(b)) => a == *b,
                                (Ok(Value::Str(a)), Value::Str(b)) => trim(&a) == trim(b),
                                _ => false,
                            }
                        }
                    });
                    if hit {
                        return Some(&arm.items);
                    }
                }
                None
            }
        }
    }

    /// Decodes items into the current scope (5). `file` allows `@` placement.
    fn items(&mut self, items: &[Item], cur: &mut u64, bound: u64, out: &mut Vec<Node>, file: bool) {
        for it in items {
            if self.stopped {
                return;
            }
            let f = match it {
                Item::Field(f) => f,
                Item::If { names, .. } | Item::Match { names, .. } => {
                    if let Some(body) = self.choose(it, *cur, out) {
                        self.items(body, cur, bound, out, file);
                    }
                    self.mark_absent(names);
                    continue;
                }
            };
            let off = match (&f.at, file) {
                (Some(e), true) => match self.eval(e) {
                    Ok(Value::Int(v)) if v >= 0 => v.min(u64::MAX as i128) as u64,
                    r => {
                        let msg = match r {
                            Ok(Value::Int(v)) => format!("negative offset {v}"),
                            Ok(Value::Str(_)) => "type mismatch in expression".into(),
                            Err(m) => m,
                        };
                        out.push(Node::error(&f.name, *cur, msg));
                        self.bind(&f.name, Val::Absent);
                        continue;
                    }
                },
                _ => *cur,
            };
            let (mut node, val) = self.decode_type(&f.name, &f.ty, off, bound);
            *cur = off.saturating_add(node.size);
            self.bind(&f.name, val);
            self.check(f, &mut node);
            out.push(node);
        }
    }

    // ------------------------------------------------------------- blocks

    /// Evaluates a section placement relative to the block start.
    fn section_start(&self, sec: &Section, base: u64, cur: u64) -> Result<u64, String> {
        if let Some(k) = sec.before {
            return base.checked_sub(k).ok_or_else(|| format!("offset -{k} is before the start of the file"));
        }
        match &sec.at {
            None => Ok(cur),
            Some(e) => match self.eval(e)? {
                Value::Int(v) if v >= 0 => Ok(base.saturating_add(v.min(u64::MAX as i128) as u64)),
                Value::Int(v) => Err(format!("negative offset {v}")),
                Value::Str(_) => Err("type mismatch in expression".into()),
            },
        }
    }

    /// Makes a finished section's fields visible to the rest of the block.
    fn merge_section(&mut self, name: &str, vars: HashMap<String, Val>) {
        let scope = self.scopes.last_mut().unwrap();
        let vars = Rc::new(vars);
        scope.insert(name.to_string(), Val::Struct(vars.clone()));
        for (k, v) in vars.iter() {
            scope.insert(k.clone(), v.clone());
        }
    }

    fn field_section(&mut self, sec: &Section, items: &[Item], start: u64, bound: u64) -> (Node, u64) {
        self.scopes.push(HashMap::new());
        let mut c = start;
        let mut kids = Vec::new();
        self.items(items, &mut c, bound, &mut kids, false);
        let vars = self.scopes.pop().unwrap();
        self.merge_section(&sec.name, vars);
        let mut node = Node::new(&sec.name, "", start, c - start, String::new());
        node.children = kids;
        (node, c)
    }

    /// Decodes a block at `base` (6.3). Returns its node, its fields and the end of its extent.
    fn decode_block(&mut self, bi: usize, base: u64, name: &str, outer: u64) -> (Node, HashMap<String, Val>, u64) {
        self.tick();
        let t = self.t;
        let blk = &t.blocks[bi];
        let fr = &t.frames[blk.frame];
        self.blocks.push(bi);
        self.scopes.push(HashMap::new());
        let n = fr.sections.len();
        let mut nodes: Vec<Option<Node>> = vec![None; n];
        // slot sections waiting for pass 2: start, count, raw values
        let mut slots: Vec<Option<PendingSlots>> = vec![None; n];
        let mut cur = base;
        let mut ext = base;
        let mut lo = base;

        // pass 1: layout and content
        for (si, sec) in fr.sections.iter().enumerate() {
            if self.stopped {
                break;
            }
            let (start, mut err) = match self.section_start(sec, base, cur) {
                Ok(s) => (s, None),
                Err(m) => (cur, Some(m)),
            };
            let (mut node, end) = match &sec.kind {
                SectionKind::Fields(items) => self.field_section(sec, items, start, outer),
                SectionKind::Slots { ty, count } => {
                    let k = self.count(count, "section").unwrap_or_else(|m| {
                        err.get_or_insert(m);
                        0
                    });
                    let s = ty.prim.size();
                    let in_file = self.data_len().saturating_sub(start) / s;
                    let mut raws = Vec::new();
                    for i in 0..k.min(in_file + 1) {
                        if !self.tick() {
                            break;
                        }
                        raws.push(self.read_uint(start + i * s, s, ty.endian.unwrap_or(self.endian)));
                    }
                    let end = start.saturating_add(k.saturating_mul(s));
                    let node = if blk.bodies[si].is_some() {
                        slots[si] = Some((start, k, raws));
                        Node::new(&sec.name, "", start, end - start, String::new())
                    } else {
                        let mut node = Node::new(&sec.name, &format!("[{}; {k}]", ty.name()), start, end - start, String::new());
                        for i in 0..k.min(MAX_CHILDREN) {
                            let raw = raws.get(i as usize).copied().flatten();
                            node.children.push(self.raw_slot(&format!("{}[{i}]", sec.name), *ty, start + i * s, raw).0);
                        }
                        node.more = k.saturating_sub(MAX_CHILDREN);
                        node
                    };
                    (node, end)
                }
                SectionKind::Open { size } => {
                    let sz = self.count(size, "section").unwrap_or_else(|m| {
                        err.get_or_insert(m);
                        0
                    });
                    let end = start.saturating_add(sz);
                    let mut node = Node::new(&sec.name, &format!("size({sz})"), start, sz, String::new());
                    match &blk.bodies[si] {
                        Some(items) => {
                            self.scopes.push(HashMap::new());
                            let mut c = start;
                            self.items(items, &mut c, end, &mut node.children, false);
                            let vars = self.scopes.pop().unwrap();
                            self.merge_section(&sec.name, vars);
                            if c < end {
                                let rest = end - c;
                                node.children.push(Node::new(&format!("⋯ {rest} unparsed bytes"), "", c, rest, String::new()));
                            }
                        }
                        None => {
                            node.ty = format!("[u8; {sz}]");
                            node.value = hex_preview(self.avail(start, sz), sz);
                        }
                    }
                    if end > self.data_len() {
                        set_error(&mut node, "beyond end of data".into());
                    }
                    (node, end)
                }
            };
            if let Some(m) = err {
                set_error(&mut node, m);
            }
            nodes[si] = Some(node);
            ext = ext.max(end);
            lo = lo.min(start);
            // sections before the block start do not move the block's own layout
            if sec.before.is_none() {
                cur = end;
            }
        }

        // pass 2: slot binding, in frame order
        for (si, sec) in fr.sections.iter().enumerate() {
            let (Some((start, k, raws)), Some(items), SectionKind::Slots { ty, .. }) =
                (slots[si].take(), &blk.bodies[si], &sec.kind)
            else {
                continue;
            };
            let node = nodes[si].as_mut().unwrap();
            self.scopes.push(HashMap::new());
            let mut idx = 0u64;
            let mut kids = Vec::new();
            let slots = Slots { start, count: k, ty: *ty, raws: &raws };
            self.slot_items(items, &slots, &mut idx, &mut kids);
            let s = ty.prim.size();
            for i in idx..k.min(idx.saturating_add(MAX_CHILDREN)) {
                let raw = raws.get(i as usize).copied().flatten();
                kids.push(self.raw_slot(&format!("{}[{i}]", sec.name), *ty, start + i * s, raw).0);
            }
            let vars = self.scopes.pop().unwrap();
            self.merge_section(&sec.name, vars);
            node.children = kids;
            node.more = k.saturating_sub(idx).saturating_sub(MAX_CHILDREN);
        }

        let vars = self.scopes.pop().unwrap();
        self.blocks.pop();
        let mut node = Node::new(name, &blk.name, lo, ext - lo, String::new());
        node.children = nodes.into_iter().flatten().collect();
        (node, vars, ext)
    }

    fn raw_slot(&mut self, name: &str, ty: PrimType, off: u64, raw: Option<u128>) -> (Node, Val) {
        self.tick();
        let mut node = Node::new(name, &ty.name(), off, ty.prim.size(), String::new());
        match raw {
            Some(r) => {
                let (text, v) = self.scalar(&Type::Prim(ty), r);
                node.value = text;
                (node, v)
            }
            None => {
                node.error = Some("beyond end of data".into());
                (node, Val::Absent)
            }
        }
    }

    /// Binds slot entries to raw slot values (7.1).
    fn slot_items(&mut self, items: &[Item], s: &Slots, idx: &mut u64, out: &mut Vec<Node>) {
        let size = s.ty.prim.size();
        for it in items {
            if self.stopped {
                return;
            }
            let f = match it {
                Item::Field(f) => f,
                Item::If { names, .. } | Item::Match { names, .. } => {
                    if let Some(body) = self.choose(it, s.start + *idx * size, out) {
                        self.slot_items(body, s, idx, out);
                    }
                    self.mark_absent(names);
                    continue;
                }
            };
            let off = s.start.saturating_add(idx.saturating_mul(size));
            let n = match &f.ty {
                Type::Array(_, Some(n)) => match self.count(n, "array") {
                    Ok(n) => n,
                    Err(m) => {
                        out.push(Node::error(&f.name, off, m));
                        self.bind(&f.name, Val::Array);
                        continue;
                    }
                },
                _ => 1,
            };
            if *idx >= s.count && n > 0 {
                out.push(Node::new(&f.name, &self.type_name(&f.ty), off, 0, "not present".into()));
                self.bind(&f.name, Val::Absent);
                continue;
            }
            let mut node = match &f.ty {
                Type::Array(elem, Some(_)) => {
                    let have = n.min(s.count - *idx);
                    let mut node = Node::new(&f.name, &format!("[{}; {n}]", self.type_name(elem)), off, have * size, format!("[{n}]"));
                    for j in 0..have {
                        if self.stopped {
                            break;
                        }
                        let (e, _) = self.slot_entry(&format!("[{j}]"), &f.name, elem, f.at.as_ref(), s, *idx + j);
                        if j < MAX_CHILDREN {
                            node.children.push(e);
                        } else {
                            node.more += 1;
                        }
                    }
                    if have < n {
                        node.error = Some(format!("only {have} of {n} slots present"));
                    }
                    *idx += have;
                    self.bind(&f.name, Val::Array);
                    node
                }
                ty => {
                    let (node, v) = self.slot_entry(&f.name, &f.name, ty, f.at.as_ref(), s, *idx);
                    *idx += 1;
                    self.bind(&f.name, v);
                    node
                }
            };
            self.check(f, &mut node);
            out.push(node);
        }
    }

    /// One slot as a link or a plain value; `var` names the raw value in `at`.
    fn slot_entry(&mut self, name: &str, var: &str, ty: &Type, at: Option<&Expr>, s: &Slots, i: u64) -> (Node, Val) {
        self.tick();
        let size = s.ty.prim.size();
        let off = s.start + i * size;
        let Some(raw) = s.raws.get(i as usize).copied().flatten() else {
            let mut node = Node::new(name, &self.type_name(ty), off, size, String::new());
            node.error = Some("beyond end of data".into());
            return (node, Val::Absent);
        };
        match ty {
            Type::Link(_, text) => {
                let v = sign_extend(raw, s.ty.prim);
                (self.link_node(name, var, text, off, size, v, at), Val::Int(v))
            }
            _ => {
                let (text, v) = self.scalar(ty, raw);
                (Node::new(name, &self.type_name(ty), off, size, text), v)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn link_node(&mut self, name: &str, var: &str, text: &str, off: u64, size: u64, raw: i128, at: Option<&Expr>) -> Node {
        let mut node = Node::new(name, &format!("-> {text}"), off, size, String::new());
        if raw == 0 && at.is_none() {
            node.value = "null".into();
            return node;
        }
        let target = match at {
            None => Ok(Value::Int(raw)),
            Some(e) => {
                self.scopes.push(HashMap::from([(var.to_string(), Val::Int(raw))]));
                let r = self.eval(e);
                self.scopes.pop();
                r
            }
        };
        match target {
            Ok(Value::Int(t)) => {
                let shown = if t < 0 { t.to_string() } else { format!("0x{t:X}") };
                let raw_note = if t != raw { format!(" (raw {raw})") } else { String::new() };
                node.value = format!("→ {shown} ({text}){raw_note}");
                if t >= 0 && (t as u128) < self.data.len() as u128 {
                    node.link = Some(NodeLink { offset: t as u64, ty: text.to_string() });
                } else {
                    node.error = Some(format!("link target {shown} is outside the file"));
                }
            }
            Ok(Value::Str(_)) => node.error = Some("type mismatch in expression".into()),
            Err(m) => {
                node.value = format!("raw {raw}");
                node.error = Some(m);
            }
        }
        node
    }

    // ------------------------------------------------------------ dispatch

    /// Whether the checks of the block's leading field sections pass at `p` (7.3).
    fn probe(&mut self, bi: usize, p: u64) -> bool {
        let t = self.t;
        let fr = &t.frames[t.blocks[bi].frame];
        self.blocks.push(bi);
        self.scopes.push(HashMap::new());
        let mut ok = true;
        let mut cur = p;
        for sec in &fr.sections[fr.leading()] {
            let SectionKind::Fields(items) = &sec.kind else { break };
            let Ok(start) = self.section_start(sec, p, cur) else {
                ok = false;
                break;
            };
            let (node, end) = self.field_section(sec, items, start, self.data_len());
            cur = end;
            if has_error(&node) {
                ok = false;
                break;
            }
        }
        self.scopes.pop();
        self.blocks.pop();
        ok && !self.stopped
    }

    fn dispatch(&mut self, cands: &[usize], p: u64, list: &str) -> (usize, Option<String>) {
        for &c in cands {
            if self.probe(c, p) {
                return (c, None);
            }
        }
        let t = self.t;
        let frames: Vec<usize> = cands.iter().map(|&c| t.blocks[c].frame).collect();
        for b in 0..t.blocks.len() {
            if !cands.contains(&b) && frames.contains(&t.blocks[b].frame) && self.probe(b, p) {
                return (b, Some(format!("expected {list}, found {}", t.blocks[b].name)));
            }
        }
        (cands[0], Some("unknown block".into()))
    }

    fn block_root(&mut self, bi: usize, base: u64, warning: Option<String>) -> Node {
        let t = self.t;
        let name = &t.blocks[bi].name;
        let (mut node, _, ext) = self.decode_block(bi, base, name, self.data_len());
        if let Some(w) = warning {
            let rest = node.error.take();
            node.error = Some(w);
            if let Some(r) = rest {
                set_error(&mut node, r);
            }
        }
        node.dispatched = Some(name.clone());
        if ext > base {
            let next = align_up(ext, t.frames[t.blocks[bi].frame].align);
            node.next = Some(NodeLink { offset: next, ty: BY_MAGIC.into() });
        }
        node
    }

    /// Decodes a root or link target type at `base`.
    fn follow(&mut self, targets: &[Target], name: &str, base: u64) -> Node {
        if let [Target::Struct(i)] = targets {
            let sname = &self.t.structs[*i].name;
            return self.decode_struct(sname, *i, base, self.data_len()).0;
        }
        let cands: Vec<usize> =
            targets.iter().filter_map(|t| if let Target::Block(b) = t { Some(*b) } else { None }).collect();
        let list = cands.iter().map(|&b| self.t.blocks[b].name.as_str()).collect::<Vec<_>>().join(" | ");
        let (bi, warning) = self.dispatch(&cands, base, if list.is_empty() { name } else { &list });
        self.block_root(bi, base, warning)
    }

    /// **(by magic)**: the first block, in declaration order, whose checks pass (12).
    fn by_magic(&mut self, base: u64) -> Result<Node, String> {
        if self.t.blocks.is_empty() {
            return Err("the template has no blocks".into());
        }
        // a block starting at `base`, else one whose sections before its start begin there, as at
        // "Next block" after a block without them
        let t = self.t;
        let starts = (0..t.blocks.len())
            .map(|b| (b, base))
            .chain((0..t.blocks.len()).filter_map(|b| match t.frames[t.blocks[b].frame].prefix {
                0 => None,
                k => base.checked_add(k).map(|p| (b, p)),
            }));
        for (b, p) in starts {
            if self.probe(b, p) {
                let mut node = self.block_root(b, p, None);
                node.dispatched = None;
                return Ok(node);
            }
        }
        let size = PREVIEW_BYTES.min(self.data_len().saturating_sub(base));
        let mut node = Node::new("unknown", "", base, size, hex_preview(self.avail(base, size), size));
        node.error = Some("unknown block".into());
        Ok(node)
    }
}

type PendingSlots = (u64, u64, Vec<Option<u128>>);

/// A slot section's raw values during pass 2.
struct Slots<'r> {
    start: u64,
    count: u64,
    ty: PrimType,
    raws: &'r [Option<u128>],
}
