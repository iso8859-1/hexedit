//! Block templates: a second template language for formats made of
//! self-describing blocks (MDF4 and the like).
//!
//! A block template starts with the line `#lang blocks 1`. The language is
//! specified in `docs/block-templates-spec.md`; section numbers in comments
//! refer to that document.

mod eval;
mod lexer;
mod parser;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use super::{BinOp, Endian, Prim, TemplateError, UnOp};

pub use eval::apply;
pub use parser::parse;

/// The language versions this build understands.
pub const VERSION: i128 = 1;

/// Reserved root name: decode whichever block matches at the offset.
pub const BY_MAGIC: &str = "(by magic)";

/// Checks the `#lang` line (1.1). `Ok(true)` for a block template, `Ok(false)`
/// for text that should be parsed as a C template.
pub fn detect(src: &str) -> Result<bool, TemplateError> {
    let first = src.strip_prefix('\u{FEFF}').unwrap_or(src).lines().next().unwrap_or("");
    let words: Vec<&str> = first.split([' ', '\t']).filter(|w| !w.is_empty()).collect();
    match words.as_slice() {
        ["#lang", "blocks", n] => match lexer::parse_number(n) {
            Some(VERSION) => Ok(true),
            Some(v) => Err(TemplateError::new(
                1,
                1,
                format!("unsupported block template version {v} (supported: {VERSION})"),
            )),
            None => Err(TemplateError::new(1, 1, format!("invalid language version '{n}'"))),
        },
        ["#lang", "blocks"] => Err(TemplateError::new(1, 1, "missing language version after '#lang blocks'")),
        ["#lang", other, ..] => Err(TemplateError::new(1, 1, format!("unknown template language '{other}'"))),
        _ => Ok(false),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    fn err(self, msg: impl Into<String>) -> TemplateError {
        TemplateError::new(self.line, self.col, msg)
    }
}

/// A value an expression evaluates to (8.1).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i128),
    Str(String),
}

#[derive(Debug, Clone)]
pub enum Expr {
    Num(i128),
    Str(String),
    /// `a` or `a.b.c`
    Path(Vec<String>, Pos),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    SizeOf(Box<Type>, Pos),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrKind {
    Ascii,
    Utf8,
    Utf16,
    Utf32,
    /// the code page selected in the toolbar
    Cp,
}

impl StrKind {
    pub fn unit(self) -> u64 {
        match self {
            StrKind::Ascii | StrKind::Utf8 | StrKind::Cp => 1,
            StrKind::Utf16 => 2,
            StrKind::Utf32 => 4,
        }
    }
}

/// An integer or float type with an optional fixed byte order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrimType {
    pub prim: Prim,
    pub endian: Option<Endian>,
}

impl PrimType {
    pub fn name(self) -> String {
        let base = match self.prim {
            Prim::U8 => "u8",
            Prim::U16 => "u16",
            Prim::U32 => "u32",
            Prim::U64 => "u64",
            Prim::I8 => "i8",
            Prim::I16 => "i16",
            Prim::I32 => "i32",
            Prim::I64 => "i64",
            Prim::F32 => "f32",
            _ => "f64",
        };
        match self.endian {
            Some(Endian::Little) => format!("{base}le"),
            Some(Endian::Big) => format!("{base}be"),
            None => base.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Struct(usize),
    Block(usize),
}

#[derive(Debug, Clone)]
pub enum Type {
    Prim(PrimType),
    /// kind, explicit byte order, fixed length in code units
    Str(StrKind, Option<Endian>, Option<Box<Expr>>),
    /// element type and count; `None` is `[T; ..]`
    Array(Box<Type>, Option<Box<Expr>>),
    /// a type name before resolution
    Named(String, Pos),
    /// `-> A | B` before resolution
    LinkNames(Vec<(String, Pos)>),
    /// resolved link: targets with aliases expanded, and their display text
    Link(Vec<Target>, String),
    Struct(usize),
    Enum(usize),
    Flags(usize),
    Block(usize),
}

#[derive(Debug, Clone)]
pub struct Check {
    pub expr: Expr,
    /// the expression as written, for `check failed: …`
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Field {
    /// `_` for unnamed fields
    pub name: String,
    pub ty: Type,
    pub at: Option<Expr>,
    pub check: Option<Check>,
    pub pos: Pos,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Wild,
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct Arm {
    pub patterns: Vec<Pattern>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Field(Field),
    If {
        branches: Vec<(Expr, Vec<Item>)>,
        otherwise: Option<Vec<Item>>,
        /// every name declared in any branch, marked absent when not decoded
        names: Vec<String>,
        pos: Pos,
    },
    Match {
        expr: Expr,
        arms: Vec<Arm>,
        names: Vec<String>,
        pos: Pos,
    },
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub items: Vec<Item>,
    pub pos: Pos,
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub base: PrimType,
    /// name, explicit value, position
    pub raw: Vec<(String, Option<Expr>, Pos)>,
    pub items: Vec<(String, i128)>,
    pub pos: Pos,
}

#[derive(Debug, Clone)]
pub struct FlagsDef {
    pub name: String,
    pub base: PrimType,
    pub raw: Vec<(String, Expr, Pos)>,
    /// name and bit number
    pub bits: Vec<(String, u32)>,
    pub pos: Pos,
}

#[derive(Debug, Clone)]
pub struct AliasDef {
    pub name: String,
    pub members: Vec<(String, Pos)>,
    pub targets: Vec<Target>,
    pub pos: Pos,
}

#[derive(Debug, Clone)]
pub enum SectionKind {
    Fields(Vec<Item>),
    Slots { ty: PrimType, count: Expr },
    Open { size: Expr },
}

#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    /// placement relative to the start of the block
    pub at: Option<Expr>,
    /// for a section placed before the block start (constant negative `@`), the distance to it
    pub before: Option<u64>,
    pub kind: SectionKind,
    pub pos: Pos,
}

#[derive(Debug, Clone)]
pub struct FrameDef {
    pub name: String,
    pub params: Vec<String>,
    pub align_expr: Option<Expr>,
    pub align: u64,
    pub sections: Vec<Section>,
    /// how far the sections before the block start reach back from it
    pub prefix: u64,
    pub pos: Pos,
}

impl FrameDef {
    /// The field sections after the sections before the block start and before the first slot or
    /// open section, which dispatch decodes (7.3).
    pub fn leading(&self) -> std::ops::Range<usize> {
        let from = self.sections.iter().position(|s| s.before.is_none()).unwrap_or(self.sections.len());
        let to = self.sections[from..]
            .iter()
            .position(|s| !matches!(s.kind, SectionKind::Fields(_)))
            .map_or(self.sections.len(), |i| from + i);
        from..to
    }
}

#[derive(Debug, Clone)]
pub struct BlockDef {
    pub name: String,
    pub frame_name: (String, Pos),
    pub frame: usize,
    pub arg_exprs: Vec<Expr>,
    pub args: Vec<Value>,
    /// sections as written: name, items, position
    pub raw_sections: Vec<(String, Vec<Item>, Pos)>,
    /// the block's items for each frame section
    pub bodies: Vec<Option<Vec<Item>>>,
    pub pos: Pos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decl {
    Const,
    Struct(usize),
    Enum(usize),
    Flags(usize),
    Alias(usize),
    Frame(usize),
    Block(usize),
}

#[derive(Debug, Clone, Default)]
pub struct Template {
    pub endian: Option<Endian>,
    pub const_exprs: Vec<(String, Expr, Pos)>,
    pub consts: HashMap<String, Value>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub flags: Vec<FlagsDef>,
    pub aliases: Vec<AliasDef>,
    pub frames: Vec<FrameDef>,
    pub blocks: Vec<BlockDef>,
    pub file: Option<Vec<Item>>,
    /// the shared namespace of all declarations (3)
    pub names: HashMap<String, Decl>,
    /// enum item name -> enums that define it
    pub enum_items: HashMap<String, Vec<usize>>,
    /// blocks and structs in declaration order
    pub root_names: Vec<String>,
    /// parse-time warnings that do not stop the template
    pub warnings: Vec<String>,
}

impl Template {
    /// Resolves a root or link type name: a struct, a block, an alias or a `|` list of these.
    pub fn targets(&self, name: &str) -> Option<Vec<Target>> {
        let mut out = Vec::new();
        for part in name.split('|').map(str::trim) {
            match self.names.get(part)? {
                Decl::Struct(i) => out.push(Target::Struct(*i)),
                Decl::Block(i) => out.push(Target::Block(*i)),
                Decl::Alias(i) => out.extend(self.aliases[*i].targets.iter().copied()),
                _ => return None,
            }
        }
        (!out.is_empty()).then_some(out)
    }

    pub fn target_name(&self, t: Target) -> &str {
        match t {
            Target::Struct(i) => &self.structs[i].name,
            Target::Block(i) => &self.blocks[i].name,
        }
    }
}
