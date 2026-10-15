use std::collections::HashMap;
use std::rc::Rc;

use encoding_rs::Encoding;
use serde::Serialize;

use super::*;

/// Array elements shown in the tree; the rest is only accounted for in the size.
const MAX_CHILDREN: u64 = 2000;
/// Upper bound on evaluated nodes, protects against garbage array counts.
const MAX_NODES: u64 = 1_000_000;
/// Longest string value rendered for char arrays.
const MAX_STRING_CHARS: usize = 512;

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub offset: u64,
    pub size: u64,
    pub value: String,
    /// (bit offset from LSB of the storage unit, width) for bitfields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bits: Option<(u32, u32)>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
    /// array elements that were not materialised as children
    #[serde(skip_serializing_if = "is_zero")]
    pub more: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// lazy link: decoded only when the user follows it
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<NodeLink>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeLink {
    pub offset: u64,
    #[serde(rename = "type")]
    pub ty: String,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

impl Node {
    fn new(name: &str, ty: &str, offset: u64, size: u64, value: String) -> Self {
        Node {
            name: name.to_string(),
            ty: ty.to_string(),
            offset,
            size,
            value,
            bits: None,
            children: vec![],
            more: 0,
            error: None,
            link: None,
        }
    }

    fn error(name: &str, offset: u64, msg: String) -> Self {
        let mut n = Node::new(name, "", offset, 0, String::new());
        n.error = Some(msg);
        n
    }
}

pub struct Options {
    pub endian: Endian,
    pub encoding: &'static Encoding,
}

pub enum Root {
    /// the template's top-level declarations
    Instances,
    /// a single type placed at the base offset
    Type(String),
}

#[derive(Debug, Clone)]
enum Val {
    Int(i128),
    Float(f64),
    Struct(Rc<HashMap<String, Val>>),
}

pub fn apply(t: &Template, data: &[u8], root: Root, base: u64, opts: &Options) -> Result<Node, String> {
    let mut ev = Ev::new(t, data, opts.endian, opts.encoding, false);
    match root {
        Root::Type(name) => {
            let ts = t.lookup_type(&name).ok_or_else(|| format!("unknown type '{name}'"))?;
            let (node, _) = ev.eval_decl(&name, &ts.ty, &ts.name, &ts.dims, None, None, base)?;
            Ok(node)
        }
        Root::Instances => {
            if t.instances.is_empty() {
                return Err("the template has no top-level declarations".into());
            }
            ev.scopes.push(HashMap::new());
            let mut cur = base;
            let mut children = Vec::new();
            for f in &t.instances {
                let name = f.name.clone().unwrap_or_default();
                let off = match &f.at {
                    Some(e) => match ev.eval_expr(e) {
                        Ok(v) if v >= 0 => v as u64,
                        Ok(v) => {
                            children.push(Node::error(&name, cur, format!("negative offset {v}")));
                            continue;
                        }
                        Err(e) => {
                            children.push(Node::error(&name, cur, format!("{e} (line {})", f.line)));
                            continue;
                        }
                    },
                    None => cur,
                };
                match ev.eval_field(f, off) {
                    Ok((node, val)) => {
                        cur = off.saturating_add(node.size);
                        if let Some(v) = val {
                            ev.scopes[0].insert(name, v);
                        }
                        children.push(node);
                    }
                    Err(e) => children.push(Node::error(&name, off, e)),
                }
            }
            let start = children.iter().map(|c| c.offset).min().unwrap_or(base);
            let end = children.iter().map(|c| c.offset.saturating_add(c.size)).max().unwrap_or(base);
            let mut root = Node::new("template", "", start, end.saturating_sub(start), String::new());
            root.children = children;
            Ok(root)
        }
    }
}

/// Evaluates an expression that may only use constants.
pub fn const_eval(t: &Template, e: &Expr) -> Result<i128, String> {
    Ev::new(t, &[], Endian::Little, encoding_rs::UTF_8, true).eval_expr(e)
}

/// Size of a struct if it does not depend on decoded data.
pub fn static_struct_size(t: &Template, idx: usize) -> Option<u64> {
    let mut ev = Ev::new(t, &[], Endian::Little, encoding_rs::UTF_8, true);
    ev.eval_struct("", "", idx, 0).ok().map(|(n, _)| n.size)
}

struct Ev<'a> {
    t: &'a Template,
    data: &'a [u8],
    endian: Endian,
    enc: &'static Encoding,
    scopes: Vec<HashMap<String, Val>>,
    nodes: u64,
    /// computing sizes only: no data, and any data dependency is an error
    static_mode: bool,
}

struct BitUnit {
    rel: u64,
    size: u64,
    used: u64,
}

fn align_up(v: u64, a: u64) -> u64 {
    if a <= 1 {
        v
    } else {
        v.div_ceil(a).saturating_mul(a)
    }
}

impl<'a> Ev<'a> {
    fn new(t: &'a Template, data: &'a [u8], endian: Endian, enc: &'static Encoding, static_mode: bool) -> Self {
        Ev { t, data, endian, enc, scopes: Vec::new(), nodes: 0, static_mode }
    }

    fn tick(&mut self) -> Result<(), String> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            Err(format!("evaluation limit of {MAX_NODES} nodes reached"))
        } else {
            Ok(())
        }
    }

    fn bytes(&self, offset: u64, len: u64) -> Option<&'a [u8]> {
        let end = offset.checked_add(len)?;
        if end > self.data.len() as u64 {
            return None;
        }
        Some(&self.data[offset as usize..end as usize])
    }

    fn read_uint(&self, offset: u64, size: u64, endian: Endian) -> Option<u128> {
        let b = self.bytes(offset, size)?;
        Some(match endian {
            Endian::Little => b.iter().rev().fold(0u128, |a, &x| (a << 8) | x as u128),
            Endian::Big => b.iter().fold(0u128, |a, &x| (a << 8) | x as u128),
        })
    }

    // ------------------------------------------------------------ expressions

    fn lookup(&self, path: &[String]) -> Result<i128, String> {
        let first = &path[0];
        let found = self.scopes.iter().rev().find_map(|s| s.get(first)).cloned();
        let mut v = match found {
            Some(v) => v,
            None => {
                if path.len() == 1 {
                    if let Some(c) = self.t.consts.get(first) {
                        return Ok(*c);
                    }
                    if first == "FILE_SIZE" {
                        if self.static_mode {
                            return Err("FILE_SIZE depends on file data".into());
                        }
                        return Ok(self.data.len() as i128);
                    }
                }
                return Err(format!("unknown identifier '{first}'"));
            }
        };
        for seg in &path[1..] {
            v = match v {
                Val::Struct(m) => m
                    .get(seg)
                    .cloned()
                    .ok_or_else(|| format!("'{}' has no field '{seg}'", path.join(".")))?,
                _ => return Err(format!("'{seg}': not a struct")),
            };
        }
        match v {
            Val::Int(i) => Ok(i),
            Val::Float(f) => Ok(f as i128),
            Val::Struct(_) => Err(format!("'{}' is a struct, not a number", path.join("."))),
        }
    }

    fn eval_expr(&self, e: &Expr) -> Result<i128, String> {
        Ok(match e {
            Expr::Num(n) => *n,
            Expr::Path(p) => self.lookup(p)?,
            Expr::Unary(op, a) => {
                let a = self.eval_expr(a)?;
                match op {
                    UnOp::Neg => a.wrapping_neg(),
                    UnOp::Not => (a == 0) as i128,
                    UnOp::BitNot => !a,
                }
            }
            Expr::Binary(op, a, b) => {
                let a = self.eval_expr(a)?;
                // short-circuit like C
                match op {
                    BinOp::And if a == 0 => return Ok(0),
                    BinOp::Or if a != 0 => return Ok(1),
                    _ => {}
                }
                let b = self.eval_expr(b)?;
                match op {
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
                }
            }
            Expr::Cond(c, a, b) => {
                if self.eval_expr(c)? != 0 {
                    self.eval_expr(a)?
                } else {
                    self.eval_expr(b)?
                }
            }
            Expr::SizeOf(ts) => {
                let mut n: u64 = 1;
                for d in &ts.dims {
                    let c = self.eval_expr(d)?;
                    if c < 0 {
                        return Err(format!("negative array size {c}"));
                    }
                    n = n.saturating_mul(c as u64);
                }
                let es = self.static_size(&ts.ty).ok_or_else(|| format!("sizeof({}) depends on file data", ts.name))?;
                es.saturating_mul(n) as i128
            }
        })
    }

    fn static_size(&self, ty: &Ty) -> Option<u64> {
        match ty {
            Ty::Prim(p) => Some(p.size()),
            Ty::Enum(i) => Some(self.t.enums[*i].base.size()),
            Ty::Pointer(s) => Some(*s as u64),
            Ty::Struct(i) => self.t.structs[*i].static_size,
            Ty::Void => None,
        }
    }

    // ------------------------------------------------------------ declarations

    fn eval_field(&mut self, f: &Field, offset: u64) -> Result<(Node, Option<Val>), String> {
        let name = f.name.clone().unwrap_or_default();
        self.eval_decl(&name, &f.ty, &f.type_name, &f.dims, f.endian, f.link.as_ref(), offset)
            .map_err(|e| format!("{e} (line {})", f.line))
    }

    #[allow(clippy::too_many_arguments)]
    fn eval_decl(
        &mut self,
        name: &str,
        ty: &Ty,
        tn: &str,
        dims: &[Expr],
        endian: Option<Endian>,
        link: Option<&Link>,
        offset: u64,
    ) -> Result<(Node, Option<Val>), String> {
        let mut counts = Vec::with_capacity(dims.len());
        for d in dims {
            let n = self.eval_expr(d)?;
            if n < 0 {
                return Err(format!("negative array size {n}"));
            }
            counts.push(n.min(u64::MAX as i128) as u64);
        }
        self.eval_dims(name, ty, tn, &counts, endian.unwrap_or(self.endian), link, offset)
    }

    #[allow(clippy::too_many_arguments)]
    fn eval_dims(
        &mut self,
        name: &str,
        ty: &Ty,
        tn: &str,
        counts: &[u64],
        endian: Endian,
        link: Option<&Link>,
        offset: u64,
    ) -> Result<(Node, Option<Val>), String> {
        let Some((&n, rest)) = counts.split_first() else {
            let (mut node, val) = self.eval_single(name, ty, tn, endian, offset)?;
            if let (Some(l), Some(Val::Int(v))) = (link, &val) {
                if !self.static_mode {
                    self.attach_link(&mut node, l, *v);
                }
            }
            return Ok((node, val));
        };
        self.tick()?;
        let full_tn: String = format!("{tn}{}", counts.iter().map(|c| format!("[{c}]")).collect::<String>());
        let elem_static = self.static_size(ty).map(|s| rest.iter().fold(s, |a, &c| a.saturating_mul(c)));

        if self.static_mode {
            let es = elem_static.ok_or("size depends on file data")?;
            return Ok((Node::new(name, &full_tn, offset, es.saturating_mul(n), String::new()), None));
        }

        if rest.is_empty() && link.is_none() {
            if let Ty::Prim(p @ (Prim::Char | Prim::Char8 | Prim::Char16 | Prim::Char32)) = ty {
                return Ok((self.string_node(name, &full_tn, *p, n, endian, offset), None));
            }
        }

        let shown = n.min(MAX_CHILDREN);
        let mut children = Vec::with_capacity(shown as usize);
        let mut cur = offset;
        for i in 0..n {
            if i >= shown {
                if let Some(es) = elem_static {
                    cur = cur.saturating_add(es.saturating_mul(n - i));
                    break;
                }
                let (node, _) = self.eval_dims("", ty, tn, rest, endian, link, cur)?;
                cur = cur.saturating_add(node.size);
                continue;
            }
            let (node, _) = self.eval_dims(&format!("[{i}]"), ty, tn, rest, endian, link, cur)?;
            cur = cur.saturating_add(node.size);
            children.push(node);
        }

        let size = cur - offset;
        let value = match ty {
            Ty::Prim(Prim::U8 | Prim::I8) if rest.is_empty() => {
                let avail = self.bytes(offset, size.min(16)).unwrap_or(&[]);
                let mut s: String = avail.iter().map(|b| format!("{b:02X} ")).collect();
                s.pop();
                if size > 16 {
                    s.push_str(" …");
                }
                s
            }
            _ => format!("[{n}]"),
        };
        let mut node = Node::new(name, &full_tn, offset, size, value);
        node.children = children;
        node.more = n - shown;
        if offset.saturating_add(size) > self.data.len() as u64 {
            node.error = Some("extends beyond end of data".into());
        }
        Ok((node, None))
    }

    /// Turns an offset field into a link; the target itself is not decoded.
    fn attach_link(&mut self, node: &mut Node, l: &Link, v: i128) {
        node.ty = format!("{} -> {}", node.ty, l.type_name);
        if v == 0 && l.at.is_none() {
            node.value = "null".into();
            return;
        }
        let target = match &l.at {
            None => Ok(v),
            Some(e) => {
                self.scopes.push(HashMap::from([(l.var.clone(), Val::Int(v))]));
                let r = self.eval_expr(e);
                self.scopes.pop();
                r
            }
        };
        let raw = match target {
            Ok(t) if t != v => format!(" (raw {v})"),
            _ => String::new(),
        };
        match target {
            Ok(t) if t >= 0 && (t as u128) < self.data.len() as u128 => {
                node.value = format!("0x{t:X}{raw}");
                node.link = Some(NodeLink { offset: t as u64, ty: l.type_name.clone() });
            }
            Ok(t) => {
                node.value = if t < 0 { format!("{t}{raw}") } else { format!("0x{t:X}{raw}") };
                node.error.get_or_insert_with(|| if t < 0 { format!("link target {t} is outside the file") } else { format!("link target 0x{t:X} is outside the file") });
            }
            Err(e) => {
                node.error.get_or_insert_with(|| format!("link target: {e}"));
            }
        }
    }

    fn eval_single(
        &mut self,
        name: &str,
        ty: &Ty,
        tn: &str,
        endian: Endian,
        offset: u64,
    ) -> Result<(Node, Option<Val>), String> {
        match ty {
            Ty::Struct(i) => self.eval_struct(name, tn, *i, offset),
            Ty::Void => Err("cannot decode void".into()),
            _ => {
                self.tick()?;
                let (prim, size) = match ty {
                    Ty::Prim(p) => (Some(*p), p.size()),
                    Ty::Enum(i) => (Some(self.t.enums[*i].base), self.t.enums[*i].base.size()),
                    Ty::Pointer(s) => (None, *s as u64),
                    _ => unreachable!(),
                };
                let mut node = Node::new(name, tn, offset, size, String::new());
                let Some(raw) = self.read_uint(offset, size, endian) else {
                    if !self.static_mode {
                        node.error = Some("beyond end of data".into());
                    }
                    return Ok((node, None));
                };
                let (value, val) = match (ty, prim) {
                    (Ty::Enum(i), Some(p)) => {
                        let v = sign_extend(raw, p);
                        (enum_name(&self.t.enums[*i], v), Val::Int(v))
                    }
                    (_, Some(p)) => format_prim(p, raw),
                    _ => (format!("0x{:0w$X}", raw, w = size as usize * 2), Val::Int(raw as i128)),
                };
                node.value = value;
                Ok((node, Some(val)))
            }
        }
    }

    fn eval_struct(&mut self, name: &str, tn: &str, idx: usize, offset: u64) -> Result<(Node, Option<Val>), String> {
        let t = self.t;
        let def = &t.structs[idx];
        if !def.defined {
            return Err(format!("'{}' is an incomplete type", def.name));
        }
        self.tick()?;
        self.scopes.push(HashMap::new());
        let pack = def.pack.unwrap_or(u64::MAX);
        let mut cur = 0u64; // running offset for structs
        let mut size = 0u64; // max member size for unions
        let mut max_align = 1u64;
        let mut children = Vec::new();
        let mut bit: Option<BitUnit> = None;

        for f in &def.fields {
            let a = t.align_of(&f.ty).min(pack).max(1);
            max_align = max_align.max(a);

            if let Some(bexpr) = &f.bits {
                match self.eval_bitfield(f, bexpr, a, def.is_union, offset, &mut cur, &mut size, &mut bit) {
                    Ok(Some((node, val))) => {
                        if let (Some(n), Some(v)) = (&f.name, val) {
                            self.scopes.last_mut().unwrap().insert(n.clone(), v);
                        }
                        children.push(node);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        if self.static_mode {
                            self.scopes.pop();
                            return Err(e);
                        }
                        let fname = f.name.clone().unwrap_or_default();
                        children.push(Node::error(&fname, offset.saturating_add(cur), format!("{e} (line {})", f.line)));
                    }
                }
                continue;
            }
            bit = None;

            let rel = if def.is_union { 0 } else { align_up(cur, a) };
            match self.eval_field(f, offset.saturating_add(rel)) {
                Ok((node, val)) => {
                    if def.is_union {
                        size = size.max(node.size);
                    } else {
                        cur = rel.saturating_add(node.size);
                    }
                    let scope = self.scopes.last_mut().unwrap();
                    match &f.name {
                        Some(n) => {
                            if let Some(v) = val {
                                scope.insert(n.clone(), v);
                            }
                            children.push(node);
                        }
                        None => {
                            // anonymous struct/union: its members belong to this scope
                            if let Some(Val::Struct(m)) = val {
                                for (k, v) in m.iter() {
                                    scope.insert(k.clone(), v.clone());
                                }
                            }
                            children.extend(node.children);
                        }
                    }
                }
                Err(e) => {
                    if self.static_mode {
                        self.scopes.pop();
                        return Err(e);
                    }
                    let fname = f.name.clone().unwrap_or_default();
                    children.push(Node::error(&fname, offset.saturating_add(rel), e));
                    if !def.is_union {
                        cur = rel;
                    }
                }
            }
        }

        let total = align_up(if def.is_union { size } else { cur }, max_align);
        let map = self.scopes.pop().unwrap();
        let mut node = Node::new(name, tn, offset, total, String::new());
        node.children = children;
        Ok((node, Some(Val::Struct(Rc::new(map)))))
    }

    #[allow(clippy::too_many_arguments)]
    fn eval_bitfield(
        &mut self,
        f: &Field,
        bexpr: &Expr,
        align: u64,
        is_union: bool,
        offset: u64,
        cur: &mut u64,
        size: &mut u64,
        bit: &mut Option<BitUnit>,
    ) -> Result<Option<(Node, Option<Val>)>, String> {
        let width = self.eval_expr(bexpr)?;
        let (p, en) = match &f.ty {
            Ty::Prim(p) if p.is_integer() => (*p, None),
            Ty::Enum(i) => (self.t.enums[*i].base, Some(&self.t.enums[*i])),
            _ => return Err("bitfields need an integer type".into()),
        };
        let unit_size = p.size();
        let unit_bits = unit_size * 8;
        if width < 0 || width as u64 > unit_bits {
            return Err(format!("invalid bitfield width {width}"));
        }
        let width = width as u64;
        if width == 0 {
            *bit = None;
            if !is_union {
                *cur = align_up(*cur, align);
            }
            return Ok(None);
        }
        self.tick()?;

        let (unit_rel, used) = if is_union {
            *size = (*size).max(unit_size);
            (0, 0)
        } else {
            match bit {
                Some(b) if b.size == unit_size && b.used + width <= unit_bits => {
                    let u = b.used;
                    b.used += width;
                    (b.rel, u)
                }
                _ => {
                    let rel = align_up(*cur, align);
                    *cur = rel + unit_size;
                    *bit = Some(BitUnit { rel, size: unit_size, used: width });
                    (rel, 0)
                }
            }
        };
        let Some(name) = &f.name else { return Ok(None) };

        let endian = f.endian.unwrap_or(self.endian);
        // little endian allocates from the LSB, big endian from the MSB
        let shift = match endian {
            Endian::Little => used,
            Endian::Big => unit_bits - used - width,
        };
        let unit_off = offset.saturating_add(unit_rel);
        let mut node = Node::new(name, &format!("{} : {width}", f.type_name), unit_off, unit_size, String::new());
        node.bits = Some((shift as u32, width as u32));
        let Some(raw) = self.read_uint(unit_off, unit_size, endian) else {
            if !self.static_mode {
                node.error = Some("beyond end of data".into());
            }
            // no value: references to this field fail
            return Ok(Some((node, None)));
        };
        let mask = if width >= 128 { u128::MAX } else { (1u128 << width) - 1 };
        let x = (raw >> shift) & mask;
        let v = if p.is_signed() && (x >> (width - 1)) & 1 == 1 { x as i128 - (1i128 << width) } else { x as i128 };
        node.value = match en {
            Some(e) => enum_name(e, v),
            None if p == Prim::Bool => format!("{}", v != 0),
            None if (0..10).contains(&v) => v.to_string(),
            None => format!("{v} (0x{x:X})"),
        };
        Ok(Some((node, Some(Val::Int(v)))))
    }

    fn string_node(&self, name: &str, tn: &str, p: Prim, n: u64, endian: Endian, offset: u64) -> Node {
        let unit = p.size();
        let total = n.saturating_mul(unit);
        let len = self.data.len() as u64;
        let avail = if offset >= len { &[][..] } else { self.bytes(offset, total.min(len - offset)).unwrap_or(&[]) };
        let text: String = match p {
            Prim::Char | Prim::Char8 => {
                let cut = avail.iter().position(|&b| b == 0).unwrap_or(avail.len());
                if p == Prim::Char {
                    self.enc.decode_without_bom_handling(&avail[..cut]).0.into_owned()
                } else {
                    String::from_utf8_lossy(&avail[..cut]).into_owned()
                }
            }
            Prim::Char16 => {
                let units: Vec<u16> = avail
                    .chunks_exact(2)
                    .map(|c| match endian {
                        Endian::Little => u16::from_le_bytes([c[0], c[1]]),
                        Endian::Big => u16::from_be_bytes([c[0], c[1]]),
                    })
                    .take_while(|&u| u != 0)
                    .collect();
                String::from_utf16_lossy(&units)
            }
            _ => avail
                .chunks_exact(4)
                .map(|c| match endian {
                    Endian::Little => u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                    Endian::Big => u32::from_be_bytes([c[0], c[1], c[2], c[3]]),
                })
                .take_while(|&u| u != 0)
                .map(|u| char::from_u32(u).unwrap_or('\u{FFFD}'))
                .collect(),
        };
        let mut node = Node::new(name, tn, offset, total, quote(&text));
        if (avail.len() as u64) < total {
            node.error = Some("extends beyond end of data".into());
        }
        node
    }
}

fn sign_extend(raw: u128, p: Prim) -> i128 {
    let bits = p.size() * 8;
    if p.is_signed() && bits < 128 {
        let s = 128 - bits as u32;
        ((raw << s) as i128) >> s
    } else {
        raw as i128
    }
}

fn format_prim(p: Prim, raw: u128) -> (String, Val) {
    let w = p.size() as usize * 2;
    match p {
        Prim::F32 => {
            let f = f32::from_bits(raw as u32);
            (crate::inspector::fmt_float(f), Val::Float(f as f64))
        }
        Prim::F64 => {
            let f = f64::from_bits(raw as u64);
            (crate::inspector::fmt_float(f), Val::Float(f))
        }
        Prim::Bool => (if raw != 0 { "true".into() } else { "false".into() }, Val::Int(raw as i128)),
        Prim::Char | Prim::Char8 => {
            let v = sign_extend(raw, p);
            let s = if (0x20..0x7F).contains(&raw) {
                format!("'{}' ({v})", raw as u8 as char)
            } else {
                format!("{v} (0x{raw:02X})")
            };
            (s, Val::Int(v))
        }
        Prim::Char16 | Prim::Char32 => {
            let s = match char::from_u32(raw as u32) {
                Some(c) if !c.is_control() => format!("'{c}' (U+{raw:04X})"),
                _ => format!("U+{raw:04X}"),
            };
            (s, Val::Int(raw as i128))
        }
        _ => {
            let v = sign_extend(raw, p);
            (format!("{v} (0x{raw:0w$X})"), Val::Int(v))
        }
    }
}

fn enum_name(e: &EnumDef, v: i128) -> String {
    if let Some((n, _)) = e.items.iter().find(|(_, x)| *x == v) {
        return format!("{n} ({v})");
    }
    // try to decompose into single-bit flags
    if v > 0 {
        let flags: Vec<&(String, i128)> =
            e.items.iter().filter(|(_, x)| *x > 0 && x.count_ones() == 1 && v & x == *x).collect();
        if !flags.is_empty() && flags.iter().fold(0, |a, (_, x)| a | x) == v {
            let names: Vec<&str> = flags.iter().map(|(n, _)| n.as_str()).collect();
            return format!("{} ({v})", names.join(" | "));
        }
    }
    format!("{v}")
}

fn quote(s: &str) -> String {
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
            c if c.is_control() => out.push_str(&format!("\\x{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str, root: &str, data: &[u8]) -> Node {
        let t = parse(src).unwrap_or_else(|e| panic!("{e}"));
        let root = if root.is_empty() { Root::Instances } else { Root::Type(root.into()) };
        apply(&t, data, root, 0, &Options { endian: Endian::Little, encoding: encoding_rs::WINDOWS_1252 }).unwrap()
    }

    fn child<'n>(n: &'n Node, name: &str) -> &'n Node {
        n.children.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no child {name}"))
    }

    #[test]
    fn natural_alignment_and_pack() {
        let src = "struct A { char c; int i; short s; };\n#pragma pack(1)\nstruct B { char c; int i; short s; };";
        let t = parse(src).unwrap();
        assert_eq!(t.structs[0].static_size, Some(12));
        assert_eq!(t.structs[1].static_size, Some(7));
        let n = run(src, "A", &[1, 0, 0, 0, 0x78, 0x56, 0x34, 0x12, 5, 0, 0, 0]);
        assert_eq!(child(&n, "i").offset, 4);
        assert!(child(&n, "i").value.starts_with("305419896"));
        assert_eq!(child(&n, "s").offset, 8);
    }

    #[test]
    fn dynamic_arrays_and_strings() {
        let src = "typedef struct { uint8_t len; char name[len]; uint16_t tail; } Rec;";
        let n = run(src, "Rec", &[3, b'a', b'b', b'c', 0x34, 0x12]);
        assert_eq!(child(&n, "name").value, "\"abc\"");
        // no alignment padding: uint16 at offset 4 is already aligned
        assert_eq!(child(&n, "tail").offset, 4);
        assert_eq!(n.size, 6);
        let t = parse(src).unwrap();
        assert_eq!(t.structs[0].static_size, None);
    }

    #[test]
    fn instances_with_placement_and_paths() {
        let src = "#pragma pack(1)\nstruct H { uint32_t count; uint32_t off; };\nH hdr;\nuint16_t items[hdr.count] @ hdr.off;";
        let data = [2, 0, 0, 0, 10, 0, 0, 0, 0, 0, 1, 0, 2, 0];
        let n = run(src, "", &data);
        let items = child(&n, "items");
        assert_eq!(items.offset, 10);
        assert_eq!(items.children.len(), 2);
        assert!(items.children[1].value.starts_with("2 "));
    }

    #[test]
    fn bitfields_enums_unions() {
        let src = "enum Color : uint8_t { RED = 1, GREEN, BLUE = 4 };\n\
                   struct S { uint16_t a : 4, b : 8; uint16_t c : 6; enum Color col; union { uint32_t u; float f; }; };";
        let data = [0xAB, 0xCD, 0x3F, 0x00, 0x02, 0, 0, 0, 0, 0, 0x80, 0x3F];
        let n = run(src, "S", &data);
        // 0xCDAB -> a = 0xB, b = 0xDA, c is in a new unit (4+8+6 > 16)
        assert_eq!(child(&n, "a").value, "11 (0xB)");
        assert!(child(&n, "b").value.starts_with("218"));
        assert_eq!(child(&n, "c").offset, 2);
        assert_eq!(child(&n, "c").value, "63 (0x3F)");
        assert_eq!(child(&n, "col").value, "GREEN (2)");
        assert_eq!(child(&n, "f").value, "1");
        assert_eq!(child(&n, "f").offset, 8);
        assert_eq!(n.size, 12);
    }

    #[test]
    fn sizeof_and_defines() {
        let src = "#define N 3\nstruct P { uint32_t x, y; };\nstruct Q { struct P pts[N]; uint8_t pad[sizeof(struct P) * 2]; };";
        let t = parse(src).unwrap();
        assert_eq!(t.structs[1].static_size, Some(24 + 16));
    }

    #[test]
    fn big_endian_pragma() {
        let src = "#pragma endian big\nstruct B { uint32_t v; uint16_t w; };";
        let n = run(src, "B", &[0, 0, 1, 0, 0x12, 0x34, 0, 0]);
        assert!(child(&n, "v").value.starts_with("256 "));
        assert!(child(&n, "w").value.starts_with("4660 "));
    }

    #[test]
    fn errors_are_reported() {
        assert!(parse("struct A { struct A a; };").is_err());
        assert!(parse("struct A { foo x; };").is_err());
        assert!(parse("struct A { int x[]; };").is_err());
        assert!(parse("struct A { struct A *next; int v; };").is_ok());
        let e = parse("struct A {\n int x\n};").unwrap_err();
        assert_eq!(e.line, 3);
    }

    #[test]
    fn bundled_examples() {
        for (name, src) in [
            ("bmp", include_str!("../../../templates/bmp.h")),
            ("png", include_str!("../../../templates/png.h")),
            ("wav", include_str!("../../../templates/wav.h")),
            ("pe", include_str!("../../../templates/pe.h")),
            ("zip", include_str!("../../../templates/zip.h")),
        ] {
            let t = parse(src).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!t.instances.is_empty(), "{name}");
            for r in &t.root_names {
                assert!(t.lookup_type(r).is_some(), "{name}: {r}");
            }
        }
        // a minimal ZIP local header: STORED, name "a.txt", 3 bytes data
        let mut d = vec![0x50, 0x4B, 3, 4, 20, 0, 0x00, 0x08, 0, 0];
        d.extend([0x21, 0x5A, 0x63, 0x58]); // 11:25:02, 2024-03-03
        d.extend([0; 4]);
        d.extend(3u32.to_le_bytes());
        d.extend(3u32.to_le_bytes());
        d.extend(5u16.to_le_bytes());
        d.extend(0u16.to_le_bytes());
        d.extend(b"a.txt");
        d.extend(b"abc");
        let n = run(include_str!("../../../templates/zip.h"), "LocalFileHeader", &d);
        assert_eq!(child(&n, "name").value, "\"a.txt\"");
        assert_eq!(child(child(&n, "flags"), "utf8").value, "1");
        assert_eq!(child(&n, "method").value, "STORED (0)");
        assert_eq!(child(child(&n, "modTime"), "hour").value, "11 (0xB)");
        assert_eq!(child(child(&n, "modDate"), "year1980").value, "44 (0x2C)");
        assert_eq!(n.size, d.len() as u64);
    }

    #[test]
    fn lazy_links() {
        let src = "#pragma pack(1)\n\
                   struct Node { uint32_t value; uint32_t next -> Node; };\n\
                   typedef struct { uint16_t n; uint16_t offs[n] -> Node @ offs * 4; } Table;\n\
                   Table tbl;\n\
                   uint32_t first @ 2 -> struct Node;";
        // tbl: n = 2, offs = [2, 0] -> targets 8 and 0; first (at 2) is 2 -> 2
        let mut d = vec![2, 0, 2, 0, 0, 0, 0, 0];
        d.extend(7u32.to_le_bytes());
        d.extend(0u32.to_le_bytes());
        let n = run(src, "", &d);
        let offs = child(child(&n, "tbl"), "offs");
        let l = offs.children[0].link.as_ref().unwrap();
        assert_eq!((l.offset, l.ty.as_str()), (8, "Node"));
        assert_eq!(offs.children[0].value, "0x8 (raw 2)");
        assert_eq!(offs.children[0].ty, "uint16_t -> Node");
        // with '@' a zero value is a regular offset
        assert_eq!(offs.children[1].link.as_ref().unwrap().offset, 0);
        let first = child(&n, "first");
        assert_eq!((first.offset, first.link.as_ref().unwrap().offset), (2, 2));
        // nothing behind the link is decoded; a zero offset is null
        let node = run(src, "Node", &d[8..]);
        assert_eq!(child(&node, "next").value, "null");
        assert!(child(&node, "next").link.is_none());
        // following the link decodes the type at the target
        let t = parse(src).unwrap();
        let opts = Options { endian: Endian::Little, encoding: encoding_rs::WINDOWS_1252 };
        let tgt = apply(&t, &d, Root::Type(l.ty.clone()), l.offset, &opts).unwrap();
        assert!(child(&tgt, "value").value.starts_with("7 "));
        // targets outside the file are errors, not links
        let n = run(src, "Node", &[0, 0, 0, 0, 0xFF, 0, 0, 0]);
        assert!(child(&n, "next").link.is_none() && child(&n, "next").error.is_some());

        assert!(parse("struct A { float f -> A; };").is_err());
        assert!(parse("struct A { uint8_t b : 4 -> A; };").is_err());
        assert!(parse("struct A { uint32_t o -> struct { int x; }; };").is_err());
        assert!(t.root_names.iter().any(|r| r == "Node"));
        let n = run("uint8_t tail[2] @ FILE_SIZE - 2;", "", &d);
        assert_eq!(child(&n, "tail").offset, d.len() as u64 - 2);
        assert!(parse("struct A { uint8_t x[FILE_SIZE]; };").unwrap().structs[0].static_size.is_none());
    }

    #[test]
    fn truncated_data() {
        let n = run("struct A { uint32_t a; uint32_t b; };", "A", &[1, 0, 0, 0, 2]);
        assert!(child(&n, "a").error.is_none());
        assert!(child(&n, "b").error.is_some());
    }
}
