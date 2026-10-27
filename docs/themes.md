# Themes

Every HexEdit theme is a TOML file, including the standard **Light** and **Dark**. You can edit,
replace or delete any of them, and add new ones. Choose a theme in the **Theme** menu at the right of
the toolbar. **System** follows the Windows light/dark setting and uses the themes named Light and
Dark.

If no theme files are installed, the menu offers only **System**. HexEdit then uses its standard light
or dark palette, which is built into the program, so it always has colors.

## The themes folder

Theme files live in your settings folder:

```
%APPDATA%\dev.hexedit.app\themes\
```

On Windows this is usually `C:\Users\<you>\AppData\Roaming\dev.hexedit.app\themes`. Choose
**Open themes folder…** in the Theme menu to open it in Explorer.

On the first start, HexEdit creates the folder and installs its standard themes there: `light.toml`,
`dark.toml` and `solarized-dark.toml`, plus a high-contrast version of each (`light-high-contrast.toml`,
`dark-high-contrast.toml`, `solarized-dark-high-contrast.toml`). The high-contrast themes keep text
at a contrast ratio of at least 7:1 (WCAG AAA) and 00 bytes and secondary text at 4.5:1 or more.
Each standard theme is installed only once. The hidden file
`.installed` in the folder records which ones were installed, so:

- a standard theme you delete stays deleted,
- a standard theme you edit is never overwritten,
- standard themes added in later versions of HexEdit still get installed.

To get the original standard themes back, delete `.installed` (and the files you want replaced), then
restart HexEdit.

- **Install a theme:** copy its `.toml` file into the folder, then choose **Reload themes** in the Theme menu.
- **Change a theme:** edit the file and choose **Reload themes**. If that theme is selected, the new
  colors apply right away.
- **Remove a theme:** delete the file and reload.

The folder sits outside the program directory, so themes survive reinstalls and updates.

## File format

```toml
name = "Solarized Dark"   # shown in the menu; defaults to the file name
base = "dark"             # "light" or "dark"; defaults to "light"

[colors]
bg     = "#002b36"
fg     = "#93a1a1"
accent = "#268bd2"
```

**`base`** picks the standard palette to start from. Any color you leave out comes from it, so a theme
can change a single color. The standard palettes are the original `light.toml` and `dark.toml`, built
into the program; editing your installed copies doesn't change them. The base also sets the window
title bar and the native controls (scrollbars, checkboxes) to light or dark.

**`[colors]`** sets colors by key. Values are CSS colors: `"#rgb"`, `"#rrggbb"`, `"#rrggbbaa"`,
`"rgb(0 43 54)"`, `"hsl(...)"`, `"oklch(...)"`, or names like `"tomato"`.

**Changing Light or Dark.** Edit `light.toml` or `dark.toml` directly. Or replace one with a smaller
file of your own that has the same name. For example, to keep the dark theme but use an orange
accent, replace `dark.toml` with:

```toml
name = "Dark"
base = "dark"

[colors]
accent = "#ff8800"
```

System uses whatever themes are named Light and Dark (case doesn't matter). If two files use the same
name, the one whose file name sorts last wins, and HexEdit shows a warning.

## Color keys

| Key | Used for |
|---|---|
| `bg` | main background: hex view, template editor, inputs |
| `bg-alt` | buttons, selects, hovered rows |
| `bg-panel` | toolbar, side panel, status bar, tree header, dialogs |
| `fg` | main text |
| `muted` | secondary text: labels, tree types, offsets and sizes |
| `faint` | de-emphasized text: bytes past the end, unavailable inspector values |
| `border` | borders and separators |
| `accent` | primary button, active tab, focus ring, links, splitter hover |
| `accent-fg` | text on the accent color |
| `sel` | selected bytes, selected tree row |
| `sel-strong` | the cursor byte while the hex view has focus |
| `cursor` | cursor outline (filled in insert mode) |
| `hl` | bytes of the hovered inspector value or tree row |
| `error` | error text |
| `error-bg` | background of template error messages |
| `offset` | offset column and column header of the hex view |
| `zero` | `00` bytes and non-printable characters |
| `f0` … `f5` | template field colors, used in turn for decoded fields (backgrounds in the hex view) |

The keys are CSS variables (`--bg`, ...) used by `ui/style.css`, which has no colors of its own.
Writing `"--bg"` in a theme works too. The standard theme files set every key and have a comment on
each one, so copying one is a good way to start a new theme. Their sources are in the repository's
[`themes/`](../themes/) folder.

## Errors

Problems are shown in a message at the bottom right after loading or reloading, each one naming the
file:

| Message | Result |
|---|---|
| `broken.toml: line 3: invalid basic string …` | the file isn't valid TOML and is skipped |
| `x.toml: base must be "light" or "dark" …` | the file is skipped |
| ``x.toml: line 1: unknown field `bg` …`` | a key outside `[colors]`; the file is skipped |
| `x.toml: unknown color "bogus"` | the key is ignored; the rest of the theme applies |
| `x.toml: "bg" is not a color: …` | the value is ignored; the rest of the theme applies |

If the selected theme's file is removed, the menu shows System. The choice is kept, so the theme comes
back if the file returns.
