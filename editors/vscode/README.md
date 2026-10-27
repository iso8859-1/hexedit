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

Or package it and install the package:

```sh
cd editors/vscode
npx @vscode/vsce package          # makes hexedit-hbt-<version>.vsix
code --install-extension hexedit-hbt-0.1.0.vsix
```

## Notes

The grammar is a TextMate grammar (`syntaxes/hbt.tmLanguage.json`), which matches regular expressions
line by line. A few words are keywords only in some places (`size`, `align`, `little`, `big`), and they
are colored wherever they appear in those places' patterns. Highlighting does not check the template;
HexEdit reports errors when it parses it.
