# Writing templates

A template is a piece of C that describes the layout of binary data: struct declarations, plus a few
extensions for file formats. HexEdit lays it over the bytes and shows the result as a tree in the
**Template** tab. Each decoded field is color-coded in the hex view, and clicking a row selects its bytes.

This guide covers:

1. [Your first template](#1-your-first-template), a step-by-step walkthrough
2. [Using templates in the app](#2-using-templates-in-the-app): editor, root, offset, files
3. [Language reference](#3-language-reference)
4. [Layout rules](#4-layout-rules): alignment, bitfields, byte order
5. [Patterns and recipes](#5-patterns-and-recipes)
6. [Limits and unsupported features](#6-limits-and-unsupported-features)
7. [Adding a template to the Examples menu](#7-adding-a-template-to-the-examples-menu)

---

## 1. Your first template

This walkthrough writes a template for Windows `.ico` files. You can follow along with the project's own
icon: open `src-tauri/icons/icon.ico` in HexEdit.

**Step 1: describe the header.** An ICO file starts with three 16-bit values. Open the Template tab,
replace the editor contents and type:

```c
#pragma pack(1)

typedef struct {
    uint16_t reserved;   // always 0
    uint16_t type;       // 1 = icon, 2 = cursor
    uint16_t count;      // number of images
} ICONDIR;
```

`#pragma pack(1)` turns off C alignment padding. File formats are almost always packed, so most
templates start with it (see [Layout rules](#4-layout-rules)).

The **Root** dropdown now offers `ICONDIR`. Leave the offset at `0x0` and press **Apply** (or
Ctrl+Enter). The tree shows `reserved = 0`, `type = 1` and `count = 1`.

**Step 2: add the directory entries.** After the header comes one 16-byte entry per image. Add:

```c
typedef struct {
    uint8_t  width;      // 0 means 256
    uint8_t  height;
    uint8_t  colorCount;
    uint8_t  reserved;
    uint16_t planes;
    uint16_t bitCount;
    uint32_t bytesInRes;
    uint32_t imageOffset;
} ICONDIRENTRY;
```

**Step 3: describe the whole file with top-level declarations.** Declarations outside of any struct
are laid out one after another, starting at the offset in the **at** field:

```c
ICONDIR      header;
ICONDIRENTRY entries[header.count];   // array size taken from a decoded field
```

The Root dropdown now has **(template declarations)** at the top, and it is selected automatically.
The tree shows `header`, followed by `entries` with one element per image.

**Step 4: jump to the data.** `@` places a declaration at an absolute offset, which can be any
expression:

```c
uint8_t png_signature[8] @ 22;        // the first image starts right after one entry
```

Arrays cannot be indexed in expressions (`entries[0].imageOffset` does not work). To reach the data
of every entry, make `imageOffset` a link (see [Lazy links](#lazy-links)):

```c
typedef uint8_t ImageHead[16];          // declare before ICONDIRENTRY

    uint32_t imageOffset -> ImageHead;  // in ICONDIRENTRY: click the value to decode 16 bytes there
```

**Step 5: save it.** Click **Save** in the template toolbar (or press Ctrl+S inside the editor) and
store it as `ico.h`. Next time, load it with **Open…**.

---

## 2. Using templates in the app

| Control | What it does |
|---|---|
| Editor | Template source. Tab inserts 4 spaces, Ctrl+Enter applies, Ctrl+S saves to a file, Ctrl+Shift+S saves as. |
| Open… / Save / Save as… | Load or store `.h` files. The editor content also persists between sessions on its own. |
| Examples… | Loads one of the bundled templates and replaces the editor content (Ctrl+Z restores it). |
| ? | Short syntax reference. |
| **Root** | What to decode. **(template declarations)** uses the template's top-level declarations. Picking a type places a single instance of it. |
| **at** | Start offset. Accepts `0x1F0`, `1F0h` or `496`. |
| **cursor** | Places the root type at the cursor and follows it as you move. Ignored for template declarations. |
| **auto** | Re-applies while you type and whenever the file changes. |
| **←** | Back: returns to the view you were in before following a link (also Alt+← or the mouse back button). |
| Links in the tree | Underlined values (`→ 0x1A40`) are [lazy links](#lazy-links). Clicking one makes its type the Root at the target offset and selects the target in the hex view. |

The Root dropdown lists named structs and unions, enum tags, and typedef names. Primitive types such
as `uint32_t` are not listed.

**Errors.** Syntax errors appear below the editor as `line L:C: message`, and that line number turns
red in the gutter. Problems found while decoding don't stop decoding. They appear inline in the tree,
marked with ⚠:

| Message | Meaning |
|---|---|
| `beyond end of data` | the field lies (partly) past the end of the file |
| `extends beyond end of data` | an array or string is cut off at the end of the file |
| `unknown identifier 'x'` | `x` is not an earlier field or a constant (or its value could not be read) |
| `negative array size -3` | the size expression evaluated to a negative number |
| `evaluation limit of 1000000 nodes reached` | a size read from garbage data is huge; check the offset |
| `link target 0x1000 is outside the file` | a link's offset points past the end of the file; it is not clickable |

---

## 3. Language reference

### Types

| Category | Names |
|---|---|
| C keywords | `char`, `short`, `int`, `long`, `long long`, `float`, `double`, `bool`/`_Bool`, combined with `signed`/`unsigned` |
| Fixed width | `int8_t` … `int64_t`, `uint8_t` … `uint64_t`, `__int8` … `__int64` |
| Short forms | `s8`/`i8` … `s64`/`i64`, `u8` … `u64`, `f32`, `f64` |
| Windows | `BYTE`, `WORD`, `DWORD`, `QWORD`, `CHAR`, `UCHAR`, `SHORT`, `USHORT`, `INT`, `UINT`, `LONG`, `ULONG`, `LONGLONG`, `ULONGLONG`, `BOOL` (4 bytes), `BOOLEAN` (1 byte), `HRESULT`, `WCHAR`, `FLOAT`, `DOUBLE` |
| Sizes | `size_t`, `uintptr_t` (8 bytes, unsigned), `ssize_t`, `intptr_t`, `ptrdiff_t`, `off_t`, `time_t` (8 bytes, signed) |
| Text | `char` (code page), `char8_t` (UTF-8), `wchar_t`/`char16_t` (UTF-16), `char32_t` (UTF-32) |
| Pointers | `T *p;` holds an address of `#pragma ptrsize` bytes (default 8), shown in hex and not followed. For file offsets, use a [lazy link](#lazy-links) |

`long` is 32 bits, as on Windows. Use `long long` or `int64_t` for 64-bit values. `const`, `volatile`
and `static` are accepted and ignored.

### Declarations

```c
struct Point { int32_t x, y; };            // tagged struct
typedef struct { uint16_t w, h; } Size;    // typedef of an anonymous struct
union Value { uint32_t u; float f; };      // all members start at the same offset
typedef char Name[16];                     // typedef with array dimensions

enum Color : uint8_t { RED = 1, GREEN, BLUE = 0x10 };   // ": type" sets the size (default int)
enum class Mode : uint16_t { READ, WRITE };             // C++ "enum class" works too

struct Shape {
    struct Point origin;     // the "struct" keyword is optional:
    Point        corner;     //   tags are usable as type names, as in C++
    enum Color   color;      // shows "GREEN (2)"
    Name         label;      // char[16], shown as a string
    uint8_t      matrix[2][3];
};
```

Nested and anonymous members work as in C11:

```c
struct Packet {
    uint8_t kind;
    union {                  // anonymous: its members appear directly in Packet
        uint32_t asInt;
        float    asFloat;
    };
    struct { uint8_t r, g, b; } rgb;   // named inline struct
};
```

A struct can contain a pointer to itself (`struct Node *next;`), but not itself by value.

### Expressions

Expressions appear in array sizes, bitfield widths, `@` placements, enum values and `#define`s.

| | |
|---|---|
| Literals | `42`, `0x2A`, `0b101010`, `052` (octal), `'A'`, `'RIFF'` (multi-char, big-endian packed); `u`/`l` suffixes ignored |
| Operators (C precedence) | `* / %` · `+ -` · `<< >>` · `< > <= >=` · `== !=` · `&` · `^` · `\|` · `&&` · `\|\|` · `?:` · unary `- ~ !` |
| Fields | `count`, or `header.info.count` to reach into nested structs |
| Constants | `#define` names and enum item names |
| Sizes | `sizeof(Type)`, `sizeof(Type[4])`, `sizeof(Type *)` |
| File | `FILE_SIZE`, the length of the file in bytes |

Arithmetic is integer only. A float field used in an expression is truncated. Comparisons return 0 or 1.

**Name lookup.** A name is looked up in this order:

1. fields of the current struct that are already decoded (fields above the current one),
2. fields of the enclosing structs,
3. the top-level declarations decoded so far,
4. constants (`#define` and enum items),
5. `FILE_SIZE` (a field or constant of that name takes precedence).

A field can only refer to data before it, never after.

### Directives

| Directive | Effect |
|---|---|
| `#pragma pack(n)` | maximum alignment `n` (1, 2, 4, 8, 16) for structs declared after it |
| `#pragma pack(push, n)` / `pack(push)` / `pack(pop)` / `pack()` | save, restore or reset the packing |
| `#pragma endian big` / `little` | byte order for the following fields |
| `#pragma endian default` | follow the toolbar's Byte order setting again (this is the initial state) |
| `#pragma ptrsize 4` / `8` | pointer size for the following declarations |
| `#define NAME expr` | integer constant; function-like macros are rejected |
| `#include`, `#ifdef`, `#pragma once`, … | ignored, so real C headers often load unchanged |

### Top-level declarations

```c
Header hdr;                           // at the start offset
Entry  table[hdr.count];              // right after hdr
uint8_t blob[hdr.blobSize] @ hdr.blobOffset;   // absolute placement
Footer footer;                        // continues right after blob
```

Without `@`, a declaration starts where the previous one ended. That includes one that was placed with
`@`. Later declarations can refer to earlier ones by name.

### Lazy links

A link is an integer field that holds the offset of another structure. Write `-> Type` after the
field's name:

```c
struct Chunk {
    uint32_t size;
    uint32_t next -> Chunk;            // offset of the next chunk
    uint8_t  data[size];
};
```

The field is decoded as usual, so `next` can still be used in expressions. In the tree its value
becomes a link such as `→ 0x1A40`, and its type reads `uint32_t -> Chunk`. Nothing at the target is
decoded until you click the link. Clicking it:

1. remembers the current view (Root, offset, expanded rows, scroll position) for **←** Back,
2. sets Root to `Chunk` and **at** to the target offset,
3. selects the target in the hex view and decodes `Chunk` there.

This keeps large files fast: a table of 100,000 offsets costs 100,000 integers, not 100,000 decoded
structures, and you open only the ones you are interested in.

**Rules:**

- The field must be an integer, enum or pointer type, and not a bitfield.
- The target must be a named type: a typedef name, a struct, union or enum tag, or a fixed-width type
  such as `uint32_t`. Inline `-> struct { ... }` is not allowed. A struct can link to itself, as in the
  example above. Use the tag form `struct Chunk { ... }` for that, since a typedef name only exists
  after its declaration.
- Link targets are added to the Root dropdown.
- A value of 0 is shown as `null` and is not a link, since 0 usually means "none".
- A target past the end of the file is reported as an error and is not clickable.
- Each element of an array of links is its own link: `uint32_t offsets[count] -> Entry;`.

**Computed targets.** If the stored value is not an absolute file offset, add `@ expression`. In that
expression, the field's own name stands for its value (the element's value, for arrays). All names
that are visible to the field can be used too:

```c
uint32_t nameOfs -> NameEntry @ hdr.stringTable + nameOfs;   // relative to a table
uint16_t blocks[n] -> Block @ blocks * 512;                  // block numbers
uint32_t first -> Entry @ first;                             // offset 0 is a valid link here
```

With `@`, a value of 0 is not treated as null. The tree shows the computed target, followed by
`(raw N)` when it differs from the stored value.

**Top-level links.** A top-level declaration can be both placed and linked. The placement comes first:

```c
uint32_t rootOffset @ 0x10 -> Directory;
```

`uint32_t rootOffset -> Directory @ 0x10;` would instead mean "the link target is 0x10".

**Paging through records.** When a single type is the Root, the tree ends with a
`→ Next Chunk at 0x…` row that decodes the instance directly after the current one. Arrays that hit
the 2000-element display cap end with `→ … N more elements`, which decodes the first element that is
not shown (if the element type can be a Root). With these, you can walk through files with many
records one structure at a time.

---

## 4. Layout rules

**Alignment.** Without `#pragma pack`, structs follow normal C alignment. Each field is aligned to its
own size (structs to their largest member), and the struct size is padded to a multiple of its
alignment. `struct { char c; int i; }` is therefore 8 bytes. `#pragma pack(n)` caps the alignment at `n`
for structs declared after it. `pack(1)` removes all padding.

The pack value is captured when a struct's `{` is reached. Changing it inside a struct body doesn't
affect that struct.

**Bitfields** follow MSVC rules. A bitfield is placed in a storage unit the size of its declared type.
Consecutive bitfields share a unit while they fit and have the same type size. Otherwise a new unit
starts. In little-endian data the bits are filled from the least significant bit; in big-endian data
from the most significant bit. `type : 0;` closes the current unit, and unnamed bitfields such as
`uint16_t : 4;` are padding. The tree shows the bit range as `[bits 4..11]`.

```c
typedef struct {
    uint16_t seconds2 : 5;   // bits 0..4
    uint16_t minute   : 6;   // bits 5..10
    uint16_t hour     : 5;   // bits 11..15
} DosTime;                   // 2 bytes
```

**Byte order.** Every field records the `#pragma endian` setting in effect where it is declared.
Fields declared under `#pragma endian default` (the initial state) follow the toolbar setting. To make
a whole format big-endian, put `#pragma endian big` at the top. You can mix byte orders within one
template.

**Strings.** Arrays of `char`, `char8_t`, `wchar_t`/`char16_t` or `char32_t` are shown as text up to the
first zero character. `char` uses the code page chosen in the toolbar. Arrays of `uint8_t`,
`unsigned char` or `signed char` are shown as hex bytes instead.

---

## 5. Patterns and recipes

**Optional field.** An array of size 0 or 1 acts as a conditional field:

```c
uint32_t BaseOfData[Magic == 0x10b];     // present only in PE32
uint8_t  padding[size & 1];              // RIFF: pad odd-sized chunks
```

**Variant selected by a tag.** Combine conditional arrays:

```c
struct Record {
    uint8_t type;
    struct IntRec  asInt[type == 1];
    struct TextRec asText[type == 2];
};
```

**Length-prefixed data:**

```c
struct PString { uint8_t len; char text[len]; };
```

**Data at an offset stored in a header:**

```c
Header hdr;
Entry  entries[hdr.entryCount] @ hdr.entryTableOffset;
```

**Padding to an alignment boundary:**

```c
uint8_t pad[(4 - (nameLength % 4)) % 4];
```

**Multi-char magic values.** `'RIFF'` is 0x52494646. A uint32 holding the bytes `52 49 46 46` in
little-endian data reads as 0x46464952, so compare against `'FFIR'` there. Alternatively, declare the
magic as `char id[4]`, which shows `"RIFF"` as a string.

**A fixed number of repeated records.** There is no "repeat until end of file". Declare as many as you
need, and extra ones past the end are simply flagged:

```c
struct Chunk chunk1;
struct Chunk chunk2;
struct Chunk chunk3;
```

**Offset tables and linked lists.** Use [lazy links](#lazy-links). The bundled ZIP example
decodes the central directory and links each entry to its local file header:

```c
uint32_t localHeader -> LocalFileHeader @ localHeader;   // "@" keeps offset 0 a link

EndOfCentralDir end @ FILE_SIZE - 22;                    // the end record is the last 22 bytes
CentralDirEntry dir[end.entries] @ end.dirOffset;
```

**Decoding at the cursor.** Select a struct type as Root and check **cursor**. The tree then decodes
wherever you click, which is handy for records scattered through a file.

---

## 6. Limits and unsupported features

- No loops, `if` blocks or functions. Use the conditional-array patterns above.
- No "repeat until end of file" arrays. Every array needs a computable size.
- Array elements cannot be referenced in expressions (`entries[2].offset`). To follow per-element
  offsets, make the element's field a lazy link.
- No floating-point arithmetic in expressions.
- Function-like macros, `#if` evaluation and `#include` are not supported (`#include` and `#ifdef` lines
  are ignored).
- Display caps: the tree shows the first 2000 elements of an array (the rest are counted in its size,
  and the `… N more elements` row continues from there), strings show up to 512 characters, and
  evaluation stops after 1,000,000 nodes. For very large tables, prefer lazy links over arrays of
  structs.

---

## 7. Adding a template to the Examples menu

Bundled templates are compiled into the binary.

1. **Write the template** as `templates/<name>.h`. Start with a comment that says what it decodes and
   which root to use, like the existing files:

   ```c
   // Windows icon (.ico): header and directory entries.
   // Root: "(template declarations)", offset 0.
   #pragma pack(1)
   ...
   ```

2. **Register it** in `template_examples()` in `src-tauri/src/lib.rs`. The order of the list is the
   order of the menu:

   ```rust
   Example { name: "ICO icon", text: include_str!("../../templates/ico.h") },
   ```

3. **Add it to the test** `bundled_examples` in `src-tauri/src/template/eval.rs`, which checks that
   every bundled template parses and that all its root types resolve:

   ```rust
   ("ico", include_str!("../../../templates/ico.h")),
   ```

4. **Test and rebuild:**

   ```sh
   cd src-tauri
   cargo test
   cargo run
   ```

   The new entry appears under **Examples…** in the Template tab.

You can also add a decoding test next to `bundled_examples` that applies the template to a few
hand-made bytes and checks field values. The ZIP check in that test is an example.
