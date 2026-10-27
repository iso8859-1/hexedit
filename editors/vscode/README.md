# HexEdit block templates for VS Code

Syntax highlighting for HexEdit's block templates (`.hbt` files, first line `#lang blocks 1`): frames,
blocks, structs, enums, flags and aliases, field and section names, primitive and string types, links
(`-> A | B`), `@` placements, `where` checks, `if` / `match`, numbers, strings and comments. It also
sets up `//` and `/* */` comments, bracket matching and auto-closing for the editor.

The language is described in [docs/block-templates.md](../../docs/block-templates.md) and
[docs/block-templates-spec.md](../../docs/block-templates-spec.md). C templates (`.h`) need nothing
extra: VS Code's C highlighting covers them.

## Install

Copy this folder into the VS Code extensions folder and restart VS Code:

```sh
# Windows (PowerShell)
Copy-Item -Recurse editors\vscode "$env:USERPROFILE\.vscode\extensions\hexedit-hbt"
# Linux / macOS
cp -r editors/vscode ~/.vscode/extensions/hexedit-hbt
```

Or install a `.vsix` package (see below):

```sh
code --install-extension editors/vscode/hexedit-hbt-0.1.0.vsix
```

## Building the .vsix

A `.vsix` is a single file that can be installed with `code --install-extension`, from the
Extensions view (**…** > **Install from VSIX…**), or shared with others. Building it needs
[Node.js](https://nodejs.org) (for `npx`); HexEdit itself does not.

```sh
cd editors/vscode
npx @vscode/vsce package
```

`npx` fetches [vsce](https://github.com/microsoft/vscode-vsce), the official packaging tool, on first
use. The package is written to `editors/vscode/hexedit-hbt-<version>.vsix`, and git ignores `*.vsix`
files. `--out <path>` writes it elsewhere.

The version is `version` in `package.json`. Raise it when the grammar changes, so that installing the
new package replaces the old one. The package contains only the files listed under `files` in
`package.json` (the grammar, the language configuration, this README and the license).

## Notes

The grammar is a TextMate grammar (`syntaxes/hbt.tmLanguage.json`), which matches regular expressions
line by line. A few words are keywords only in some places (`size`, `align`, `little`, `big`), and they
are colored wherever they appear in those places' patterns. Highlighting does not check the template;
HexEdit reports errors when it parses it.
