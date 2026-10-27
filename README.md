# HexEdit

A hex editor built with Rust and [Tauri 2](https://tauri.app). It has a data inspector and C-struct templates.

- **Hex view**: virtualized, so multi-GB files scroll smoothly. Hex and text panes, overwrite/insert mode
  (`Insert`), selection with mouse or Shift+keys, copy/paste as hex or text, undo/redo, go to (`Ctrl+G`),
  and find for hex bytes, code page text, UTF-8, or UTF-16 (`Ctrl+F`, `F3`). **Recent…** next to Open
  lists the last 10 files opened or saved.
- **Data inspector**: decodes the bytes at the cursor as binary, (u)int8/16/24/32/64, float16/32/64,
  (S/U)LEB128, characters and strings in the selected code page, UTF-8 and UTF-16, Unix time (32/64),
  FILETIME, DOS date/time, OLE date, and GUID. Double-click a numeric value to edit it. Byte order and
  code page are set in the toolbar. "Local" is the system ANSI code page.
- **Templates**: C declarations decoded into a tree. Fields are color-coded in the hex view, and clicking
  a tree row selects its bytes. Lazy links (`uint32_t ofs -> Type;`) show an offset as a link that
  decodes the target only when clicked, with Back navigation, so large files with many structures stay
  fast. Examples are bundled for BMP, PNG, WAV, PE, and ZIP.

- **Themes**: every theme is a TOML file in `%APPDATA%\dev.hexedit.app\themes`, including the standard
  Light and Dark, so they can be edited, replaced or removed, and new ones added. Choose one in the
  toolbar; **System** follows the Windows setting. The window title bar switches along with the theme.
  See **[docs/themes.md](docs/themes.md)**.

## Build and run

Requires a Rust toolchain. On Windows, WebView2 must also be installed (it ships with Windows 10/11).
The frontend is plain HTML/JS in `ui/`, so no Node.js is needed.

```sh
cd src-tauri
cargo run                      # or: cargo run -- path\to\file.bin
cargo test                     # template engine, inspector, document model
cargo build --release          # binary in src-tauri/target/release
```

The UI is embedded at compile time. If an edit in `ui/` doesn't show up, touch `src-tauri/src/lib.rs`
before rebuilding.

## Building the installer

HexEdit ships as an NSIS setup program that installs per user, so it needs no administrator rights.
It requires the Tauri CLI, a one-time install:

```sh
cargo install tauri-cli --version "^2" --locked
```

Then, in `src-tauri`:

```sh
cargo tauri build
```

This makes a release build and packages it as
`src-tauri/target/release/bundle/nsis/HexEdit_<version>_x64-setup.exe`. The first run downloads the
NSIS tools, so it needs internet access.

**What the installer does:**

- Installs to `%LOCALAPPDATA%\HexEdit` and adds a Start menu entry and an entry under
  *Settings > Apps*, all for the current user only.
- Adds **Open with HexEdit** to the Explorer context menu of every file (on Windows 11 under
  *Show more options*), registered for the current user in `HKCU\Software\Classes\*\shell\HexEdit`.
  The installer hooks in `src-tauri/windows/hooks.nsh` add it, and uninstalling removes it.
- Running a newer setup over an existing install updates it in place.
- If WebView2 is missing (rare on Windows 10/11), it downloads and installs it silently, which needs
  internet access.
- Uninstalling removes the program and the context menu entry, but keeps the settings and themes in
  `%APPDATA%\dev.hexedit.app` unless *Delete the application data* is checked in the uninstaller.
- For scripted installs, `HexEdit_..._x64-setup.exe /S` installs silently, and
  `%LOCALAPPDATA%\HexEdit\uninstall.exe /S` uninstalls silently.

**Configuration** is the `bundle` section of `src-tauri/tauri.conf.json`: publisher, descriptions,
icons, and the NSIS settings (`installMode: currentUser`, `installerHooks`). The version comes from `version` in
`src-tauri/Cargo.toml`, so bump it there before building a release.

**Icons** in `src-tauri/icons/` were generated from `icon.png`. To change the icon, replace `icon.png`
(ideally 1024×1024) and regenerate the desktop sizes:

```sh
cargo tauri icon icons/icon.png -o <temp folder>
# copy icon.ico, icon.icns, 32x32.png, 128x128.png, 128x128@2x.png back into icons/
```

The installer is not code-signed, so Windows SmartScreen shows a warning the first time it runs.
`cargo tauri build` warns that the identifier `dev.hexedit.app` ends in `.app`. That only matters for
macOS builds. Changing the identifier would move the settings folder, so it is left as is.

## Code coverage

Coverage is measured with [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov). It needs a
one-time setup:

```sh
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --locked
```

Then, in `src-tauri`, these aliases (defined in `src-tauri/.cargo/config.toml`) run the tests with
instrumentation:

```sh
cargo cov          # per-file summary table in the terminal
cargo cov-html     # HTML report with line-by-line highlighting, opened in the browser
cargo cov-lcov     # target/lcov.info, for editor gutters (e.g. VS Code "Coverage Gutters") or CI
```

Any `cargo llvm-cov` option can be added, e.g. `cargo cov -- template` to run only matching tests.
`lib.rs` and `main.rs` contain the Tauri commands and app setup. The unit tests don't exercise them, so
they show as uncovered. The frontend (`ui/`) has no tests and is not measured.

## Template syntax

**[docs/templates.md](docs/templates.md)** is the full guide. It covers a step-by-step first template,
the language reference, layout rules, common patterns, and adding templates to the Examples menu.
A quick overview:

```c
#pragma pack(1)               // also pack(push, n) / pack(pop); default: natural C alignment
#pragma endian big            // big | little | default (toolbar); applies to following fields
#pragma ptrsize 4             // pointer size (default 8)
#define COUNT 4

enum Kind : uint16_t { A = 1, B, C = 0x10 };

typedef struct {
    char     magic[4];        // char arrays decode as text in the selected code page
    wchar_t  title[8];        // wchar_t/char16_t: UTF-16, char8_t: UTF-8, char32_t: UTF-32
    uint32_t count;
    enum Kind kind;           // shows "B (2)", or "X | Y (3)" for flag combinations
    uint8_t  flag : 1, mode : 3;      // bitfields
    union { uint32_t u; float f; };   // anonymous members
    uint16_t items[count];    // sizes may reference earlier fields...
    uint8_t  opt[kind == B];  // ...so 0/1-sized arrays act as optional fields
    uint32_t next -> Header;  // lazy link: decoded only when clicked (0 = null)
    uint32_t rel -> Header @ rel + 0x200;   // link target computed from the value
} Header;

Header hdr;                             // top-level declarations: laid out from the start offset
uint8_t data[hdr.count * 2] @ 0x100;    // @ places at an absolute offset (any expression)
uint8_t tail[16] @ FILE_SIZE - 16;      // FILE_SIZE is the length of the file
```

In the Template tab, choose either **(template declarations)** or a single type as the root. A type is
placed at the given offset, or at the cursor when **cursor** is checked. With **auto**, the template
re-applies as you type and whenever the data changes. Clicking a link makes its type the root at the
target offset; **←** (or Alt+←) returns to the previous view.

Supported: `struct`, `union`, `enum` (with a C++ `: type` base), `typedef`, nested and anonymous
aggregates, multi-dimensional arrays, bitfields, pointers, and `sizeof(Type)`. Expressions support
`+ - * / % << >> & | ^ ~ ! == != < > <= >= && || ?:` and dotted paths such as `hdr.count`. Struct and
enum tags can be used as type names, as in C++. `long` is 32 bits (Windows LLP64).

Not supported: loops or "repeat until EOF" arrays, `if` blocks, function-like macros.

## Layout

```
src-tauri/src/
  lib.rs           Tauri commands, dialogs, window handling
  document.rs      in-memory buffer, undo/redo with typing coalescing, atomic save
  inspector.rs     basic type decoding/encoding
  encodings.rs     code pages (encoding_rs), search patterns
  template/        lexer.rs -> parser.rs (AST in mod.rs) -> eval.rs (layout over the data)
  themes.rs        theme files (TOML) from the config folder
ui/                index.html, style.css, main.js (wiring/keys), hexview.js, inspector.js, template.js
templates/         example templates, embedded into the binary
themes/            standard themes (Light, Dark, Solarized Dark), installed into the themes folder
```

## Notes

- Files are loaded completely into memory, so very large files need matching RAM.
- Saving writes a temporary file next to the target and renames it over the original.
- With **Backup** checked (toolbar, next to Save as), the first save over an existing file in a session
  keeps its original content as `<name>.bak`, e.g. `data.bin.bak`. Later saves in the same session leave
  that backup alone, so it always holds the file as it was before HexEdit changed it.
- For UI debugging on Windows, start the app with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`
  and attach Chrome/Edge DevTools to `localhost:9333`.

## License

MIT, see [LICENSE](LICENSE).
