# Block template language, version 1

**Status:** implemented, 2026-10-19 (`src-tauri/src/template/blocks/`). The motivation and design
decisions are in [block-templates.md](block-templates.md).

This document defines the syntax and the decoding rules of block templates. Wherever it says that
something works "as in C templates", the rules in [templates.md](templates.md) apply.

Contents:

1. [Source text](#1-source-text)
2. [Grammar](#2-grammar)
3. [Declarations](#3-declarations)
4. [Types](#4-types)
5. [Items: fields, conditionals, checks](#5-items-fields-conditionals-checks)
6. [Frames and blocks](#6-frames-and-blocks)
7. [Links and dispatch](#7-links-and-dispatch)
8. [Expressions](#8-expressions)
9. [Name lookup](#9-name-lookup)
10. [Tree output](#10-tree-output)
11. [Errors and warnings](#11-errors-and-warnings)
12. [Integration with the app](#12-integration-with-the-app)

---

## 1. Source text

### 1.1 Language line

The first line of the template must be exactly:

```
#lang blocks 1
```

There may be a UTF-8 byte order mark before it and spaces or tabs around the tokens. The number is the
language version, and this document defines version 1. A template whose first line is `#lang blocks N`
with any other `N` is rejected with `unsupported block template version N (supported: 1)`.

Text without this line is parsed as a C template. The file extension `.hbt` is a convention for the
Open and Save dialogs and does not change how the text is parsed.

### 1.2 Lexical elements

| Element | Definition |
|---|---|
| Whitespace | space, tab, carriage return |
| Line break | `\n`. Acts as a separator, see [1.3](#13-separators-and-line-breaks) |
| Comments | `// …` to the end of the line, `/* … */` (not nested). A comment counts as whitespace. A line comment does not consume the line break |
| Identifiers | `[A-Za-z_][A-Za-z0-9_]*`, case-sensitive |
| Integers | decimal `42`, hex `0x2A`, binary `0b101010`, octal `0o52`. `_` may separate digits: `0xFFFF_FFFF` |
| Strings | `"…"` with the escapes `\\` `\"` `\n` `\r` `\t` `\0` `\xHH` `\u{H…}` |
| Punctuation | `{ } ( ) [ ] , ; : = => -> @ .. .` |
| Operators | `+ - * / % << >> < > <= >= == != & ^ \| && \|\| ! ~ ? :` |

There are no floating-point or character literals.

**Keywords:** `alias`, `block`, `const`, `else`, `endian`, `enum`, `file`, `flags`, `frame`, `if`,
`match`, `struct`, `where`. None of them can name a top-level declaration. `if`, `else`, `match` and
`where` cannot be used as identifiers at all. The others only start top-level declarations, so they
can name fields, sections, parameters, enum items and flags, and appear in expressions
(`flags: CgFlags`, `if flags.vlsd`).

**Contextual words**, which are identifiers everywhere else:

- `little` and `big` after `endian`,
- `size` and `align` in frames,
- the type names in [4.1](#41-primitive-types) and [4.2](#42-strings) where a type is expected.

A field may therefore be called `size` or `type`.

### 1.3 Separators and line breaks

Items in a list (fields, enum items, match arms, frame sections, block sections) are separated by a
comma or a line break. Several separators in a row count as one, and a separator before the closing
`}` is allowed.

A line break is whitespace when it:

- is inside parentheses `( )` or brackets `[ ]`,
- directly follows `{`, or directly precedes `}`,
- precedes `else`.

To continue an expression on the next line, wrap it in parentheses:

```rust
data: size(length - 24
           - 8 * link_count)       // inside parentheses: fine
```

---

## 2. Grammar

EBNF. `sep` is one or more of `,` or line break, as defined in [1.3](#13-separators-and-line-breaks).
`{ x }` means zero or more, `[ x ]` means optional.

```ebnf
template      = lang-line { sep } [ top-item { sep top-item } ] { sep } ;
lang-line     = "#lang" "blocks" INTEGER ;

top-item      = endian | const | struct | enum | flags | alias | frame | block | file ;

endian        = "endian" ( "little" | "big" ) ;
const         = "const" IDENT "=" expr ;
struct        = "struct" IDENT "{" items "}" ;
enum          = "enum" IDENT ":" int-type "{" [ enum-item { sep enum-item } ] [ sep ] "}" ;
enum-item     = IDENT [ "=" expr ] ;
flags         = "flags" IDENT ":" int-type "{" [ flag-item { sep flag-item } ] [ sep ] "}" ;
flag-item     = IDENT "=" expr ;
alias         = "alias" IDENT "=" IDENT { "|" IDENT } ;
file          = "file" "{" items "}" ;

frame         = "frame" IDENT [ "(" [ IDENT { "," IDENT } ] ")" ] [ "align" expr ]
                "{" [ frame-section { sep frame-section } ] [ sep ] "}" ;
frame-section = IDENT [ "@" expr ] "{" items "}"                     (* field section *)
              | IDENT ":" "[" int-type ";" expr "]" [ "@" expr ]     (* slot section *)
              | IDENT ":" "size" "(" expr ")" [ "@" expr ] ;         (* open section *)

block         = "block" IDENT ":" IDENT [ "(" [ expr { "," expr } ] ")" ]
                "{" [ block-section { sep block-section } ] [ sep ] "}" ;
block-section = IDENT "{" items "}" ;

items         = [ item { sep item } ] [ sep ] ;
item          = field | if | match ;
field         = IDENT ":" type [ "@" expr ] [ "where" expr ] ;
if            = "if" expr "{" items "}" [ "else" ( if | "{" items "}" ) ] ;
match         = "match" expr "{" [ arm { sep arm } ] [ sep ] "}" ;
arm           = pattern { "|" pattern } "=>" "{" items "}" ;
pattern       = "_" | expr ;

type          = prim-type | string-type | array-type | link-type | IDENT ;
int-type      = (* an integer type from 4.1 *) ;
prim-type     = (* a type from 4.1 *) ;
string-type   = string-kind [ "(" expr ")" ] ;
string-kind   = "ascii" | "utf8" | "utf16" | "utf16le" | "utf16be"
              | "utf32" | "utf32le" | "utf32be" | "cp" ;
array-type    = "[" type ";" ( expr | ".." ) "]" ;
link-type     = "->" IDENT { "|" IDENT } ;
```

The expression grammar is in [8](#8-expressions). Two places need care because `|` has two meanings:

- In a `pattern`, a top-level `|` separates alternatives. Write `(A | B)` for a bitwise or.
- In `link-type` and `alias`, `|` separates type names.

`if` and `match` read their expression up to the `{` that opens the body.

---

## 3. Declarations

Top-level declarations may appear in any order. A name may be used before the declaration that
defines it. All declared names (consts, types, frames, blocks) share one namespace, and declaring a
name twice is an error.

| Declaration | Meaning |
|---|---|
| `endian little` / `endian big` | Default byte order for the whole template. Allowed at most once, before all other declarations. Without it, the toolbar's Byte order setting applies |
| `const N = expr` | Integer or string constant. The expression may only use literals and other constants. Cycles are an error |
| `struct S { items }` | A named sequence of items |
| `enum E: T { … }` | An integer type `T` with named values. Items without `= expr` take the previous value plus 1, and the first defaults to 0 |
| `flags F: T { name = bit, … }` | An integer type `T` with named bits. Each bit number must be in `0 … 8·sizeof(T) − 1`. Two names may not share a bit |
| `alias A = B \| C \| …` | A union of blocks, for use in link types only. A name in the list may itself be an alias, which is expanded in place. Cycles are an error |
| `frame` | See [6.1](#61-frames) |
| `block` | See [6.2](#62-blocks) |
| `file { items }` | The entry point decoded by **(template declarations)**. Allowed at most once |

---

## 4. Types

### 4.1 Primitive types

| Type | Size | Notes |
|---|---|---|
| `u8` `u16` `u32` `u64` | 1, 2, 4, 8 | unsigned |
| `i8` `i16` `i32` `i64` | 1, 2, 4, 8 | two's complement |
| `f32` `f64` | 4, 8 | IEEE 754 |

Every primitive type can take the suffix `le` or `be` (`u32be`, `f64le`), which fixes its byte order.
Without a suffix, the template's `endian` applies, or the toolbar setting if there is none. `u8le` and
the like are allowed and behave like `u8`.

The integer types are `u8` to `u64` and `i8` to `i64`, with or without a suffix.

### 4.2 Strings

| Kind | Code unit | Encoding |
|---|---|---|
| `ascii` | 1 byte | ASCII. Bytes ≥ 0x80 are shown as `\xHH` |
| `utf8` | 1 byte | UTF-8 |
| `utf16`, `utf16le`, `utf16be` | 2 bytes | UTF-16. Without a suffix, the default byte order applies |
| `utf32`, `utf32le`, `utf32be` | 4 bytes | UTF-32. Without a suffix, the default byte order applies |
| `cp` | 1 byte | the code page selected in the toolbar |

- **Fixed length** `kind(n)`: occupies exactly `n` code units. The value is the text up to the first NUL
  code unit.
- **Unbounded** `kind`: reads code units up to and including the first NUL, or up to the end of the
  [bound](#44-bounds) if there is no NUL. Its size is the number of bytes read, including the NUL.

Invalid sequences decode as U+FFFD. The display limit is 512 characters, as in C templates.

### 4.3 Arrays

`[T; n]` is `n` consecutive elements of type `T`, and `n` is evaluated when the array is decoded.
A negative `n` is reported as `⚠ negative array size` and the array is decoded as empty.

`[T; ..]` repeats `T` until the end of the [bound](#44-bounds). An element that would cross the bound
is decoded as far as it goes and marked `⚠ extends beyond end of section`. A `[T; ..]` whose element
has size 0 is an error.

Arrays of `u8` and `i8` are shown as hex bytes, like `uint8_t` arrays in C templates. Arrays of strings
are allowed: `[ascii(8); 4]` is four fixed-length strings, and `[utf8; ..]` is a sequence of
NUL-terminated strings.

### 4.4 Bounds

The bound of an item is the end of the innermost enclosing open section (see
[6.1](#61-frames)). Outside of open sections, it is the end of the file. Unbounded strings and `[T; ..]`
stop at the bound. Any field that crosses the bound is marked `⚠ beyond end of section` (or
`⚠ beyond end of data` for the end of the file).

### 4.5 Named types

| Type | Can be used as |
|---|---|
| `struct` | field type, link target, root |
| `enum`, `flags` | field type, slot entry type (see [7.1](#71-slot-entries)) |
| `block` | field type, link target, root |
| `alias` | link target only |

Structs and blocks cannot contain themselves by value, directly or through other types. Doing so is an
error. Through links, any type may refer to any other type, including itself.

A block used as a field type is decoded inline, as described in [6.3](#63-decoding-a-block), and
occupies its [extent](#64-extent-and-next-block) without alignment.

---

## 5. Items: fields, conditionals, checks

Items appear in structs, `file { }`, frame field sections and block sections.

### 5.1 Fields

```rust
name: Type
name: Type @ offset          // only in file { }: absolute file offset
name: Type where check
```

Fields are decoded one after another, each starting where the previous one ended. There is no
alignment padding.

- **`@ expr`** is allowed only on fields of `file { }`. It places the field at the absolute file offset
  `expr`, and the next field continues after it. In a slot section, `@` has a different meaning, see
  [7.2](#72-link-targets).
- **`_` as a name** declares an unnamed field, for padding and reserved bytes. It is shown in the tree
  but cannot be referenced. Several fields may be called `_`.

### 5.2 Checks

`where expr` is evaluated right after the field is decoded. The expression can use the field itself.
A check fails if the expression evaluates to 0 or cannot be evaluated. A failed check is reported as
`⚠ check failed: <expression text>` on the field. Decoding continues normally.

Checks in frame field sections also decide dispatch, see [7.3](#73-dispatch).

### 5.3 `if` and `match`

```rust
if flags.vlsd { vlsd_size: u64 }
else          { data_bytes: u32, inval_bytes: u32 }

match type {
    LIN        => { b: f64, a: f64 }
    TAB | TABI => { val: [f64; val_count] }
    _          => { }
}
```

- `if` decodes its body when the condition is a non-zero integer. `else if` chains are allowed.
- `match` compares the value against each arm's patterns in order and decodes the first matching arm.
  An integer pattern matches by `==`. A string pattern matches by string comparison
  ([8.3](#83-strings)). `_` matches anything. If no arm matches, nothing is decoded and no warning is
  given.
- Pattern expressions must be constant: literals, constants and enum items. When the matched value
  has an enum type, bare item names are looked up in that enum first.
- The items of the decoded branch become part of the enclosing scope and tree, with no extra node.
- A name may appear in several branches of one `if` or `match`. Declaring a name twice on one path
  (for example, before an `if` and again inside it) is an error.
- Referring to a field from a branch that was not decoded fails with
  `⚠ 'x' is not present`.

---

## 6. Frames and blocks

### 6.1 Frames

```rust
frame Mdf4Block(magic) align 8 {
    header {
        id:         ascii(4) where id == magic
        reserved:   u32
        length:     u64
        link_count: u64
    }
    links: [u64; link_count]
    data:  size(length - 24 - 8 * link_count)
}
```

A frame describes the layout shared by a family of blocks. It consists of named sections, laid out in
declaration order. Each section starts where the previous section ended, or at `@ expr` relative to
the start of the block. Section names must be unique within the frame.

A section whose `@` is a constant negative expression lies **before the block start**, for example a
recovery link in front of the block's ID:

```rust
frame Mdf4DataBlock(magic) align 8 {
    recovery: [u64; 1] @ -8
    header { id: ascii(4) where id == magic, … }
    …
}
```

- Such sections must come before all other sections of the frame (an error otherwise).
- They do not move the layout: the first other section without `@` starts at the block start.
- Links and `file { }` placements still give the block start (the ID), not the start of these sections.
- They are not part of dispatch ([7.3](#73-dispatch)), so a slot section may come first.
- If the block start is too close to the start of the file, the section is shown with
  `⚠ offset -N is before the start of the file`.
- The frame's **prefix** is the largest distance of these sections from the block start.

| Section | Syntax | Content |
|---|---|---|
| Field section | `name { items }` | Fixed by the frame. Blocks cannot change it |
| Slot section | `name: [T; count]` | `count` raw values of the integer type `T`. Blocks give them names and meaning |
| Open section | `name: size(expr)` | `expr` bytes. Blocks fill them with items |

- **Parameters** (`magic` above) are bound to the block's arguments. They are visible in all sections of
  the frame and of the block.
- **`align n`** applies only to "Next block" ([6.4](#64-extent-and-next-block)). `n` must be a constant
  power of two. The default is 1.
- `count` and `size(…)` are evaluated when their section is reached. They can use the fields of earlier
  field sections and the parameters. A negative value is reported as `⚠ negative section size` and
  treated as 0.

### 6.2 Blocks

```rust
block CN: Mdf4Block("##CN") {
    data  { … }        // fills the open section "data"
    links { … }        // names the slots of the slot section "links"
}
```

A block names its frame and passes one argument per frame parameter. The arguments must be constant
expressions.

A block section `name { items }` must refer to a slot section or an open section of the frame. Each
section may be given at most once. A section that the block leaves out is shown raw: slots as
`name[i]` integers, and open sections as bytes.

Items in an open section follow [5](#5-items-fields-conditionals-checks). Entries of a slot section
follow [7.1](#71-slot-entries).

### 6.3 Decoding a block

A block at offset `B` is decoded in two passes.

**Pass 1: layout and content.** Each frame section is processed in declaration order:

1. A field section decodes the frame's items.
2. A slot section evaluates `count` and reads `count` raw values of `T`.
3. An open section evaluates its size and decodes the block's items for it, with the section's end as
   the bound. Unused bytes at the end are shown as `⋯ N unparsed bytes`.

**Pass 2: slot binding.** Each slot section with a block body binds its entries to the raw values read
in pass 1 ([7.1](#71-slot-entries)). Slot sections are bound in frame order.

During pass 1, expressions see the parameters and the fields decoded so far. During pass 2, they see
all fields of pass 1 and the slot entries bound so far. As a result, data fields cannot refer to link
names, but link entries can refer to any data field.

The tree shows the sections in frame order, whatever the order in which they were decoded.

### 6.4 Extent and next block

The **extent** of a block runs from the smallest start to the largest end of any of its sections, so
it includes sections before the block start. For a frame with
`align n`, the **next block** starts at the extent's end rounded up to a multiple of `n`.

---

## 7. Links and dispatch

### 7.1 Slot entries

A slot section's body consists of entries. Each entry consumes slots in order, starting with the first:

| Entry | Consumes | Meaning |
|---|---|---|
| `name: -> Target` | 1 slot | a link |
| `name: [-> Target; n]` | `n` slots | an array of links, one per slot |
| `name: T` | 1 slot | a plain value. `T` must be the slot type or an enum or flags type of the same size |
| `name: [T; n]` | `n` slots | an array of plain values |
| `if`, `match` | as their branches | conditional entries, as in [5.3](#53-if-and-match) |

Other types (strings, structs, blocks, `[…; ..]`) are not allowed in a slot section. `@` (see
[7.2](#72-link-targets)) and `where` are allowed on entries.

**Slot count mismatches:**

- **More slots than entries:** the remaining slots are shown after the entries as `name[i]`, using the
  section's name and the slot's index (e.g. `links[9]`), with their raw values.
- **Fewer slots than entries:** each entry that has no slot left is shown as `not present`. Referring to
  it fails with `⚠ 'x' is not present`. An array that runs out of slots keeps the elements that have
  slots and is marked `⚠ only k of n slots present`.

Neither case is an error, since format versions add and drop links.

### 7.2 Link targets

`Target` is a struct, a block, an alias or a `|`-separated list of these, with aliases expanded in
place. A list may contain structs only if it has exactly one entry.

The **target offset** is the slot's raw value. A raw value of 0 is shown as `null` and is not a link.
With `@ expr`, the target offset is `expr` instead, where the entry's name stands for the raw value of
the slot (for arrays, the current element's value). With `@`, a value of 0 is a valid link, as in C
templates:

```rust
first: -> Entry @ table_start + first * 16
```

A target offset outside the file is shown as `⚠ link target 0x… is outside the file` and is not
clickable.

Links are never followed during decoding. A link row shows the target offset and the declared target
list, for example `→ 0x1F80 (TX | MD)`. Nothing at the target is read until the user follows the link.

### 7.3 Dispatch

Following a link to a list of blocks (a single block counts as a list of one) selects the block type to
decode at the target offset `P`:

1. **Candidates, in list order.** For each candidate, decode the field sections of its frame at `P` that
   come before the frame's first slot or open section (skipping the sections before the block start), with the candidate's arguments bound. The
   candidate matches if all `where` checks in these sections pass and none of the fields lie beyond
   the end of the file. The first matching candidate is decoded.
2. **Other blocks.** If no candidate matches, try the other blocks that use the frame of any candidate,
   in declaration order. The first match is decoded with `⚠ expected TX | MD, found CC` on the root.
3. **Nothing.** If nothing matches, decode the first candidate anyway, so its failed checks are
   visible, with `⚠ unknown block` on the root.

A block whose frame has no checks in these leading field sections always matches. A parse-time warning
says so if such a block appears in a list with more than one entry.

Following a link to a struct decodes the struct at the target without dispatch.

---

## 8. Expressions

### 8.1 Values

An expression evaluates to an integer (128-bit signed internally) or a string.

| Source | Value |
|---|---|
| Integer fields, enums | the integer |
| Flags fields | the raw integer. `f.name` is 0 or 1 |
| Float fields | truncated toward zero. NaN and infinities fail with `⚠ non-finite value in expression` |
| String fields, string literals | the string |
| Struct fields | not a value. Use a path into them: `ver.major` |
| Arrays | not a value. Elements cannot be indexed |
| Links | the raw slot value |

### 8.2 Operators

As in C templates, with C precedence, from highest to lowest:

```
unary - ~ !
* / %
+ -
<< >>
< > <= >=
== !=
&
^
|
&&
||
?:
```

Arithmetic is integer only. Comparisons and logical operators yield 0 or 1. Division or remainder by 0
fails with `⚠ division by zero`. An integer used as a condition is true when it is non-zero.

### 8.3 Strings

Strings support only `==` and `!=`, between two strings. Before comparing, trailing NUL and space
characters are removed from both sides, so `file_id == "MDF"` matches `"MDF     "`. Any other operator
applied to a string, or a comparison between a string and an integer, fails with
`⚠ type mismatch in expression`.

### 8.4 Built-ins

| Name | Value |
|---|---|
| `FILE_SIZE` | length of the file in bytes |
| `sizeof(T)` | size of a type whose size does not depend on decoded data. Other types are an error |

### 8.5 Evaluation errors

An expression that fails to evaluate produces a warning on the item that uses it:

- **Array size or section size:** the array or section is decoded as empty.
- **Condition:** the condition counts as false.
- **Check:** the check counts as failed.
- **Link target:** the link is not clickable.

Decoding continues in every case.

---

## 9. Name lookup

A name in an expression is resolved in this order:

1. fields of the current struct or section scope decoded so far, including those from decoded `if` /
   `match` branches,
2. fields of the enclosing scopes decoded so far, out to the current block (all of its sections, as
   allowed by [6.3](#63-decoding-a-block)), or to `file { }`,
3. the parameters of the current block's frame,
4. constants: `const` declarations and enum items,
5. built-ins.

A path `a.b.c` looks up `a` as above and then selects nested struct fields.

Enum items are global constants. If two enums define the same item name, the bare name is ambiguous:
using it is an error, except in `match` patterns on a value of one of those enums. In all cases, the
qualified form `Enum.ITEM` works.

A block reached by following a link starts a fresh scope. It sees only its own fields, its parameters,
constants and built-ins, and nothing from the block the link came from. A block or struct decoded
inline as a field sees its enclosing scopes as in step 2.

---

## 10. Tree output

| Element | Row |
|---|---|
| Block | `CN` with its offset. Children: one node per frame section |
| Section | the section's name. Children: its fields or entries |
| Integer | the value, in the number format chosen in the UI |
| Enum | `LIN (1)`, or the number alone for an unnamed value |
| Flags | `0x1008 {monotonous, default_x}`. Set bits without a name are shown as `bit 17` |
| String | the text in quotes |
| Byte array | hex bytes |
| Link | `→ 0x1F80 (TX \| MD)`. The type column shows `-> TX \| MD`. `null` for 0 without `@`. With `@`, `(raw N)` follows when the target differs from the raw value |
| Missing slot entry | `not present` |
| Extra slot | `links[9]` with the raw value |
| Unused section bytes | `⋯ N unparsed bytes` |

Items from `if` and `match` branches appear directly in their enclosing node. Clicking a row selects
its bytes in the hex view, as in C templates. A link row selects its slot.

The display caps from C templates apply: the first 2000 array elements are shown (followed by a
`… N more elements` row), strings show up to 512 characters, and decoding stops after 1,000,000 nodes.

---

## 11. Errors and warnings

### 11.1 Parse errors

These stop the template from being applied. They are shown as `line L:C: message`, as in C templates.

- syntax errors, unknown or unsupported language version
- unknown type, frame, block or constant
- duplicate declaration, duplicate frame section, duplicate block section
- duplicate field name on one path
- wrong number of frame arguments, non-constant argument, `const` or pattern
- block section that names a field section or no section of the frame
- link type outside a slot section, disallowed type inside a slot section
- `@` on a field outside `file { }`
- a section placed before the block start after another section of the frame
- `endian` after other declarations or more than once, more than one `file { }`
- struct or block containing itself by value, cyclic `alias` or `const`
- `sizeof` of a type with a data-dependent size, `[T; ..]` with an element of size 0
- `align` that is not a constant power of two, flag bit out of range or used twice
- ambiguous enum item

### 11.2 Decode warnings

These are shown inline in the tree with ⚠. Decoding continues.

| Message | Cause |
|---|---|
| `beyond end of data` | a field lies (partly) past the end of the file |
| `beyond end of section` / `extends beyond end of section` | a field or array crosses its open section's end |
| `check failed: <expr>` | a `where` check evaluated to 0 or failed |
| `'x' is not present` | `x` belongs to an untaken branch or a missing slot |
| `unknown identifier 'x'` | `x` is not visible at this point |
| `negative array size N` / `negative section size N` | a size expression is negative |
| `only k of n slots present` | a slot array ran out of slots |
| `expected A \| B, found C` | dispatch fell back to another block |
| `unknown block` | dispatch found no matching block |
| `link target 0x… is outside the file` | the target offset is past the end of the file |
| `offset -N is before the start of the file` | a section before the block start would lie before offset 0 |
| `division by zero`, `type mismatch in expression`, `non-finite value in expression` | evaluation errors |
| `evaluation limit of 1000000 nodes reached` | decoding stopped |

---

## 12. Integration with the app

- **Parser selection:** the editor uses the block parser when the first line is a `#lang blocks` line,
  and the C parser otherwise.
- **Root** lists, in declaration order:
  - **(template declarations)** if there is a `file { }`,
  - **(by magic)** if there is at least one block,
  - every block and struct.
- **(by magic)** decodes at the offset (or the cursor, with **cursor** checked) using dispatch over all
  blocks in declaration order. Step 2 of [7.3](#73-dispatch) does not apply, and step 3 shows the raw
  bytes with `⚠ unknown block`. If no block starts at the offset, the blocks whose frames have a
  prefix are tried with their start at the offset plus the prefix, so the offset may also be the start
  of a recovery link, as at **Next block**.
- **Following a link** works as for lazy links in C templates. It records the view in the navigation history for **←** Back,
  sets Root to the dispatched type and **at** to the target offset, and selects the target in the hex
  view. Root is set to the block that dispatch selected.
- **Next block:** when the root is a block or **(by magic)**, the tree ends with
  `→ Next block at 0x…`. It decodes at the next block offset ([6.4](#64-extent-and-next-block)) using
  **(by magic)**. For a struct root, `→ Next <Struct> at 0x…` decodes the struct directly after the
  current one.
