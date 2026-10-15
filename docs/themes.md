# Themes

HexEdit has two built-in themes, **Light** and **Dark**. You can add more, or change the built-in ones,
with TOML files. Choose a theme in the **Theme** menu at the right of the toolbar. **System** follows
the Windows light/dark setting and uses the themes named Light and Dark.

## The themes folder

Theme files live in your settings folder:

```
%APPDATA%\dev.hexedit.app\themes\
```

On Windows this is usually `C:\Users\<you>\AppData\Roaming\dev.hexedit.app\themes`. Choose
**Open themes folder…** in the Theme menu to open it in Explorer. The folder is created on the first
start, with one example, `solarized-dark.toml`. If you delete the example, it stays deleted.

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

**`base`** is the built-in theme to start from. Any color you leave out comes from it, so a theme can
change a single color. The base also sets the window title bar and the native controls (scrollbars,
checkboxes) to light or dark.

**`[colors]`** sets colors by key. Values are CSS colors: `"#rgb"`, `"#rrggbb"`, `"#rrggbbaa"`,
`"rgb(0 43 54)"`, `"hsl(...)"`, `"oklch(...)"`, or names like `"tomato"`.

**Replacing a built-in theme.** A file with `name = "Dark"` (or `"Light"`, case doesn't matter)
replaces that built-in theme, and System uses it too. For example, to keep the dark theme but use an
orange accent:

```toml
name = "Dark"
base = "dark"

[colors]
accent = "#ff8800"
```

If two files use the same name, the one whose file name sorts last wins, and HexEdit shows a warning.

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

The keys are the CSS variables in `ui/style.css` without the leading `--`. Writing `"--bg"` works too.
The example file `solarized-dark.toml` sets every key and has a comment on each one, so it's a good
starting point. Its source is in the repository at [`themes/solarized-dark.toml`](../themes/solarized-dark.toml).

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
