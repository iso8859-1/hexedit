//! C-struct based templates.
//!
//! A template is a subset of C declarations (structs, unions, enums, typedefs,
//! bitfields, arrays) with a few extensions:
//! * array sizes may reference previously decoded fields (`uint8_t data[len];`),
//! * top-level declarations place instances in the file (`Header hdr @ 0x10;`),
//! * `#pragma endian big|little`, `#pragma pack(...)`, `#pragma ptrsize 4|8`.

mod eval;
mod lexer;
mod parser;

use std::collections::HashMap;
use std::fmt;

pub use eval::{apply, Node, Options, Root};
pub use parser::parse;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TemplateError {
    pub line: usize,
    pub col: usize,
    pub msg: String,
}

impl TemplateError {
    pub fn new(line: usize, col: usize, msg: impl Into<String>) -> Self {
        Self { line, col, msg: msg.into() }
    }
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}:{}: {}", self.line, self.col, self.msg)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prim {
    /// plain `char`: 8 bit, arrays decode as text in the selected code page
    Char,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Bool,
    /// `char8_t`: arrays decode as UTF-8
    Char8,
    /// `wchar_t` / `char16_t`: arrays decode as UTF-16
    Char16,
    /// `char32_t`: arrays decode as UTF-32
    Char32,
}

impl Prim {
    pub fn size(self) -> u64 {
        use Prim::*;
        match self {
            Char | I8 | U8 | Bool | Char8 => 1,
            I16 | U16 | Char16 => 2,
            I32 | U32 | F32 | Char32 => 4,
            I64 | U64 | F64 => 8,
        }
    }

    pub fn is_signed(self) -> bool {
        matches!(self, Prim::Char | Prim::I8 | Prim::I16 | Prim::I32 | Prim::I64)
    }

    pub fn is_integer(self) -> bool {
        !matches!(self, Prim::F32 | Prim::F64)
    }
}

#[derive(Debug, Clone)]
pub enum Ty {
    Prim(Prim),
    Struct(usize),
    Enum(usize),
    /// pointer of the given byte size
    Pointer(u8),
    Void,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Shl,
    Shr,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    BitAnd,
    BitXor,
    BitOr,
    And,
    Or,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Num(i128),
    /// `a` or `a.b.c`, resolved against decoded fields and constants
    Path(Vec<String>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    SizeOf(Box<TypeSpec>),
}

/// A resolved type plus any array dimensions contributed by a typedef.
#[derive(Debug, Clone)]
pub struct TypeSpec {
    pub ty: Ty,
    /// type name as written in the template, used for display
    pub name: String,
    pub dims: Vec<Expr>,
}

#[derive(Debug, Clone)]
pub struct Field {
    /// `None` for anonymous struct/union members
    pub name: Option<String>,
    pub ty: Ty,
    pub type_name: String,
    pub dims: Vec<Expr>,
    pub bits: Option<Expr>,
    pub endian: Option<Endian>,
    /// absolute placement (`@ expr`), top-level declarations only
    pub at: Option<Expr>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub is_union: bool,
    pub fields: Vec<Field>,
    pub pack: Option<u64>,
    pub defined: bool,
    /// size when it does not depend on decoded data
    pub static_size: Option<u64>,
    pub align: u64,
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub base: Prim,
    pub items: Vec<(String, i128)>,
}

#[derive(Debug, Clone, Default)]
pub struct Template {
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub typedefs: HashMap<String, TypeSpec>,
    pub consts: HashMap<String, i128>,
    pub instances: Vec<Field>,
    /// struct tag -> index
    pub struct_tags: HashMap<String, usize>,
    /// enum tag -> index
    pub enum_tags: HashMap<String, usize>,
    /// names offered as root types, in declaration order
    pub root_names: Vec<String>,
}

impl Template {
    pub fn align_of(&self, ty: &Ty) -> u64 {
        match ty {
            Ty::Prim(p) => p.size(),
            Ty::Enum(i) => self.enums[*i].base.size(),
            Ty::Pointer(s) => *s as u64,
            Ty::Struct(i) => self.structs[*i].align,
            Ty::Void => 1,
        }
    }

    /// Find a type by the name a user would pick as template root.
    pub fn lookup_type(&self, name: &str) -> Option<TypeSpec> {
        if let Some(ts) = self.typedefs.get(name) {
            return Some(ts.clone());
        }
        let bare = name
            .strip_prefix("struct ")
            .or_else(|| name.strip_prefix("union "))
            .or_else(|| name.strip_prefix("enum "))
            .unwrap_or(name);
        if let Some(&i) = self.struct_tags.get(bare) {
            return Some(TypeSpec { ty: Ty::Struct(i), name: bare.to_string(), dims: vec![] });
        }
        if let Some(&i) = self.enum_tags.get(bare) {
            return Some(TypeSpec { ty: Ty::Enum(i), name: bare.to_string(), dims: vec![] });
        }
        parser::builtin_type(name).map(|p| TypeSpec { ty: Ty::Prim(p), name: name.to_string(), dims: vec![] })
    }
}
