# Block templates (concept)

**Status:** implemented, 2026-10-19. The exact syntax and rules are in the specification,
[block-templates-spec.md](block-templates-spec.md). This document explains the design. The MDF4
example below is bundled as **Examples… → MDF4 blocks** (`templates/mdf4.hbt`), with the blocks it
refers to filled in.

The C template language ([templates.md](templates.md)) describes formats that map directly to C structs.
Block templates are a second language for formats made of self-describing blocks. In these formats, the
type of a block is decided by a tag inside the block, fields are present or absent depending on other
fields, and part of a block can only be interpreted after a later part has been decoded. MDF4 is the
motivating example.

## Goals and non-goals

Goals:

- Decode one block at a time. Follow links between blocks lazily, so files with millions of blocks and
  cyclic block graphs cost nothing until you navigate.
- Determine the type of a link target from the bytes at the target (its magic), not only from the link.
- Express optional content: fields enabled by flag bits, counts taken from other fields, variants
  selected by a type field.
- Express a fixed frame (header, link section, data section) once, and let each block describe only
  what differs.

Non-goals:

- Decoding record data (MDF4 `DT`). Once you are in a data block, apply a C template at the payload
  offset. Compressed data (`DZ`) is not decoded at all.
- Being a viewer for a format. Lists of blocks are walked one block at a time, by following `next`
  links. There are no chain or list views.

## File identification

The first line of a block template is:

```
#lang blocks 1
```

The editor chooses the parser from this line, so a template typed directly into the editor works
without being saved. Without the line, the text is parsed as a C template. The number is the language
version. The usual file extension is `.hbt`, used by the Open and Save dialogs. It does not affect
which parser is used.

## Model

A **frame** is the layout shared by a family of blocks. It consists of **sections**, which are decoded
in order:

- a **header** section of plain fields,
- a **links** section: an array of raw slots (for MDF4, `link_count` × `u64`),
- a **data** section with a size computed from the header.

A **block** uses a frame and fills its sections in:

- `data { ... }` lists the data fields.
- `links { ... }` names the slots and gives them target types.

Decoding a block takes four steps:

1. Decode the header.
2. Read the link slots as raw integers. Their count and position depend only on the header.
3. Decode the data section.
4. Bind the `links { }` description to the slots. It runs last, so it can use data fields (flags,
   counts, type fields).

The tree shows the sections in frame order (header, links, data), regardless of the decode order.

## Syntax

Declarations are written as `name: Type`. Fields are separated by newlines or commas. Comments are
`//` and `/* */`.

### Types

| Category | Syntax |
|---|---|
| Integers | `u8` `u16` `u32` `u64` `i8` `i16` `i32` `i64` |
| Floats | `f32` `f64` |
| Explicit byte order | suffix `be` or `le`: `u32be`, `f64le` |
| Arrays | `[T; n]`, where `n` is any expression |
| Rest of section | `[T; ..]` fills the enclosing section |
| Strings | `ascii`, `utf8`, `utf16le`, `utf16be`, `utf32le`, `utf32be`, `cp` (the toolbar code page) |
| Fixed-length strings | `ascii(4)`, `utf16le(n)`: the length counts code units |
| Links | `-> T`, `-> A \| B`, `[-> T; n]` (only in `links { }`) |
| Named types | structs, enums, flags, aliases, blocks |

A string without a length ends at the first NUL or at the end of the enclosing section. A fixed-length
string is shown up to its first NUL.

The file's byte order is set at the top with `endian little` or `endian big`. Without it, the toolbar
setting applies.

### Structs, enums, flags

```rust
struct Version {
    major: u16
    minor: u16
}

enum ChannelType: u8 { VALUE = 0, VLSD, MASTER, VIRTUAL_MASTER, SYNC, MLSD, VIRTUAL, VLSC }

flags CgFlags: u16 { vlsd = 0, bus_event = 1, plain_bus_event = 2, remote_master = 3, event_group = 4 }
```

`flags` lists bit numbers. A flags value is shown as the set of names of its set bits, plus the raw
value. In expressions, `flags.remote_master` is 0 or 1, and `flags` alone is the raw integer.

### Checks: `where`

```rust
id: ascii(4) where id == magic
version: u16 where version >= 400 && version < 500
```

`where` is followed by a boolean expression that may use the field itself and everything visible to it.
A failed check is shown inline as `⚠ check failed: id == magic`. Decoding continues. Checks in a header
also drive dispatch (see [Links and dispatch](#links-and-dispatch)).

### Conditionals

```rust
if flags.equal_length { equal_length: u64 }
else                  { offsets: [u64; count] }

match type {
    LIN      => { b: f64, a: f64 }
    RAT      => { p: [f64; 6] }
    TAB | TABI => { val: [f64; val_count] }
    _        => { }
}
```

`if` / `else` and `match` work in all sections, including `links { }`, where they decide which slots
get which names. A `match` arm can list several values with `|`. `_` is the fallback. Without a
matching arm, nothing is decoded.

### Frames and blocks

```rust
frame Mdf4Block(magic) {
    header {
        id:         ascii(4) where id == magic
        reserved:   u32
        length:     u64
        link_count: u64
    }
    links: [u64; link_count]
    data:  size(length - 24 - 8 * link_count)
}

block TX: Mdf4Block("##TX") {
    data { text: utf8 }
}
```

A frame takes parameters. A block names its frame and supplies arguments. The frame's `links` and `data`
sections start where the previous section ended, unless they are placed with `@ expr` (relative to the
start of the block).

A section can also sit *before* the block start, with a constant negative `@`. Some MDF4 writers put an
8-byte recovery link in front of DT and DZ blocks: links point at the `##DT`, the recovery link sits just
before it, and the block's length does not count it. Such a frame starts with that section:

```rust
frame Mdf4DataBlock(magic) align 8 {
    recovery: [u64; 1] @ -8
    header { id: ascii(4) where id == magic, … }
    …
}

block DT: Mdf4DataBlock("##DT") {
    recovery { link: -> HD | DG | DT | … }
    data     { records: [u8; ..] }
}
```

The block in the tree and its selection in the hex view include the recovery link, and **(by magic)**
finds the block whether the offset is its ID or the recovery link before it, so **Next block** works
after a block without one.

A block may omit `links { }` (all slots are shown raw) or `data { }` (the section is shown as bytes).

### Links

In `links { }`, each entry consumes slots in order:

```rust
links {
    next:        -> CN
    composition: -> CA | CN
    attachments: [-> AT; attachment_count]
    if flags.default_x {
        default_x_dg: -> DG
        default_x_cg: -> CG
        default_x_cn: -> CN
    }
}
```

- An entry consumes one slot. An array consumes `n`.
- If there are more slots than entries, the extra slots are shown as `link[9]` with their raw values.
  Newer format versions that append links remain readable.
- If there are fewer slots than entries, the missing entries are shown as `not present`.
- A slot value of 0 is shown as `null`.

### Aliases

```rust
alias Text = TX | MD
```

An alias names a union of block types for use in links.

### Entry point

```rust
file {
    id: Id
    hd: HD @ 64
}
```

`file { }` is what **(template declarations)** decodes. Its fields are laid out like a struct, and `@`
places a field at an absolute offset.

### Expressions

These are the C template expressions (integer arithmetic, C operators and precedence, `FILE_SIZE`, field
paths such as `header.length`). In addition:

- string literals, which can be compared with `==` and `!=` against string fields,
- frame parameters,
- the fields of the current block, in all of its sections. `links { }` sees header and data fields.

## Links and dispatch

Links are always lazy. A link row shows the address and the declared type, for example
`→ 0x1F80 (TX | MD)`, and nothing at the target is read until the link is followed.

Following a link to a single type `T` decodes `T` at the target. Following a link to `A | B` reads the
target's header and picks the type:

1. Decode the header section of each candidate at the target. Pick the first candidate whose header
   checks (`where`) all pass.
2. If no candidate matches, try all other blocks that use the same frames. If one matches, decode it and
   show `⚠ expected TX | MD, found CC`.
3. If nothing matches, show the target as raw bytes with `⚠ unknown block`.

Following a link uses the existing navigation: the toolbar's **←** Back (Alt+←) returns to the
previous block, with its expanded rows and scroll position.

## Using it in the app

- **Root** lists the blocks and structs. **(template declarations)** decodes `file { }`.
  **(by magic)** decodes whichever block matches at the offset, using the dispatch rules over all
  blocks. With **cursor** checked, clicking on a block's first byte decodes that block.
- **→ Next block** at the end of the tree decodes the block that starts right after the current one's
  last section, rounded up to the frame's `align`, using dispatch by magic.
- Errors and warnings work as in C templates. Section overruns are reported as
  `⚠ beyond end of data section`. Unused bytes at the end of a section are shown as
  `⋯ N unparsed bytes`.

## Implementation notes

- New parser and evaluator in their own module. They share `Node` (the tree output), the expression
  evaluator, lazy link handling and the evaluation limits with the C templates.
- The expression evaluator needs string values for `where id == "##CN"` and frame parameters.
- Dispatch only decodes headers, which are small and fixed in size, so it stays cheap even when trying
  many candidates.

## Example: MDF4 (excerpt)

Field layouts follow the asammdf block definitions (`v4_constants.py`). Check against the ASAM MDF 4.x
specification before bundling.

```rust
#lang blocks 1
endian little

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

struct Id {                              // 64 bytes at offset 0, no block frame
    file_id:          ascii(8) where file_id == "MDF"
    version:          ascii(8)
    program:          ascii(8)
    reserved0:        [u8; 4]
    version_number:   u16
    reserved1:        [u8; 30]
    unfinalized:      u16
    unfinalized_user: u16
}

block TX: Mdf4Block("##TX") { data { text: utf8 } }
block MD: Mdf4Block("##MD") { data { xml: utf8 } }
alias Text = TX | MD

block HD: Mdf4Block("##HD") {
    data {
        start_time_ns:  u64
        tz_offset_min:  i16
        dst_offset_min: i16
        time_flags:     u8
        time_class:     u8
        flags:          u8
        reserved:       u8
        start_angle:    f64
        start_distance: f64
    }
    links {
        first_dg:      -> DG
        file_history:  -> FH
        channel_tree:  -> CH
        first_at:      -> AT
        first_ev:      -> EV
        comment:       -> Text
    }
}

block DG: Mdf4Block("##DG") {
    data {
        record_id_len: u8
        reserved:      [u8; 7]
    }
    links {
        next:     -> DG
        first_cg: -> CG
        data:     -> DT | DL | DZ | HL
        comment:  -> Text
    }
}

flags CgFlags: u16 { vlsd = 0, bus_event = 1, plain_bus_event = 2, remote_master = 3, event_group = 4 }

block CG: Mdf4Block("##CG") {
    data {
        record_id:      u64
        cycle_count:    u64
        flags:          CgFlags
        path_separator: u16
        reserved:       u32
        if flags.vlsd { vlsd_size: u64 }
        else          { data_bytes: u32, inval_bytes: u32 }
    }
    links {
        next:       -> CG
        first_cn:   -> CN
        acq_name:   -> TX
        acq_source: -> SI
        first_sr:   -> SR
        comment:    -> Text
        if flags.remote_master { master: -> CG }
    }
}

flags CnFlags: u32 {
    all_invalid = 0, inval_present = 1, precision = 2, value_range = 3,
    limit_range = 4, ext_limit_range = 5, discrete = 6, calibration = 7,
    calculated = 8, virtual = 9, bus_event = 10, monotonous = 11,
    default_x = 12, event_signal = 13, vlsd = 14,
}

enum ChannelType: u8 { VALUE = 0, VLSD, MASTER, VIRTUAL_MASTER, SYNC, MLSD, VIRTUAL, VLSC }

block CN: Mdf4Block("##CN") {
    data {
        type:             ChannelType
        sync_type:        u8
        data_type:        u8
        bit_offset:       u8
        byte_offset:      u32
        bit_count:        u32
        flags:            CnFlags
        inval_bit_pos:    u32
        precision:        u8
        reserved:         u8
        attachment_count: u16
        val_range:        [f64; 2]
        limit:            [f64; 2]
        limit_ext:        [f64; 2]
    }
    links {
        next:        -> CN
        composition: -> CA | CN
        name:        -> TX
        source:      -> SI
        conversion:  -> CC
        data:        -> DT | DL | DZ | HL | SD
        unit:        -> Text
        comment:     -> Text
        attachments: [-> AT; attachment_count]
        if flags.default_x {
            default_x_dg: -> DG
            default_x_cg: -> CG
            default_x_cn: -> CN
        }
    }
}

enum ConvType: u8 { NON = 0, LIN, RAT, ALG, TABI, TAB, RTAB, TABX, RTABX, TTAB, TRANS, BITFIELD }

block CC: Mdf4Block("##CC") {
    data {
        type:      ConvType
        precision: u8
        flags:     u16
        ref_count: u16
        val_count: u16
        phy_range: [f64; 2]
        match type {
            LIN => { b: f64, a: f64 }
            RAT => { p: [f64; 6] }
            _   => { val: [f64; val_count] }
        }
    }
    links {
        name:    -> TX
        unit:    -> Text
        comment: -> Text
        inverse: -> CC
        match type {
            ALG => { formula: -> TX }
            _   => { refs: [-> TX | CC; ref_count] }
        }
    }
}

flags DlFlags: u8 { equal_length = 0, time_values = 1, angle_values = 2, distance_values = 3 }

block DL: Mdf4Block("##DL") {
    data {
        flags:    DlFlags
        reserved: [u8; 3]
        count:    u32
        if flags.equal_length { equal_length: u64 }
        else                  { offsets: [u64; count] }
    }
    links {
        next:   -> DL
        blocks: [-> DT | DZ | SD | RD; count]
    }
}

block DT: Mdf4Block("##DT") { data { records: [u8; ..] } }

file {
    id: Id
    hd: HD @ 64
}
```

## Decisions

- **No bitfields.** Single bits are covered by `flags`. Wider bit ranges are not needed for MDF4.
- **Block size.** A block ends where its last section ends. There is no separate size declaration. A
  frame can declare `align n` so that "→ Next block" skips padding (MDF4 blocks start on 8-byte
  boundaries).
- **String comparison** ignores trailing NUL and space characters on both sides, so `"MDF"` matches
  `"MDF     "`.
- **No chains.** Lists are walked one block at a time through their `next` links.
- **Checks** use `where expr` rather than `= value`, so they can't be mistaken for defaults and can
  express any condition.
