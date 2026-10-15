//! Color themes loaded from TOML files in the app's config folder.
//!
//! The built-in "Light" and "Dark" themes live in `ui/style.css`. A theme
//! file names a base (light or dark) and overrides any of its colors. A file
//! whose name matches a built-in theme replaces it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Color keys a theme can set; each is the CSS variable `--<key>`.
pub const KEYS: &[&str] = &[
    "bg", "bg-alt", "bg-panel", "fg", "muted", "faint", "border", "accent", "accent-fg", "sel", "sel-strong",
    "cursor", "hl", "error", "error-bg", "offset", "zero", "f0", "f1", "f2", "f3", "f4", "f5",
];

/// Written to a newly created themes folder.
pub const EXAMPLE_FILE: &str = "solarized-dark.toml";
pub const EXAMPLE: &str = include_str!("../../themes/solarized-dark.toml");

#[derive(Debug, Clone, Serialize)]
pub struct Theme {
    pub name: String,
    /// "light" or "dark": the colors not set here, and the window title bar
    pub base: String,
    /// key (without "--") -> CSS color
    pub colors: BTreeMap<String, String>,
    /// the file it was loaded from; none for built-in themes
    pub file: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ThemeList {
    pub dir: String,
    pub keys: Vec<&'static str>,
    pub themes: Vec<Theme>,
    /// problems found while loading, one per line, naming the file
    pub warnings: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    name: Option<String>,
    base: Option<String>,
    #[serde(default)]
    colors: BTreeMap<String, toml::Value>,
}

fn builtin(name: &str, base: &str) -> Theme {
    Theme { name: name.into(), base: base.into(), colors: BTreeMap::new(), file: None }
}

/// Accepts hex colors, functional notations (`rgb(...)`, `hsl(...)`, ...) and
/// named colors. The page ignores values the browser cannot parse anyway, this
/// only gives the author feedback.
fn valid_color(v: &str) -> bool {
    let v = v.trim();
    if let Some(h) = v.strip_prefix('#') {
        return matches!(h.len(), 3 | 4 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit());
    }
    if let Some((func, rest)) = v.split_once('(') {
        return matches!(func, "rgb" | "rgba" | "hsl" | "hsla" | "hwb" | "lab" | "lch" | "oklab" | "oklch")
            && rest.ends_with(')')
            && rest[..rest.len() - 1].chars().all(|c| c.is_ascii_alphanumeric() || " .,%/+-".contains(c));
    }
    !v.is_empty() && v.chars().all(|c| c.is_ascii_alphabetic())
}

/// Parses one theme file. `stem` is the file name without extension, used when
/// the file has no `name`. Problems with single colors are warnings, so the
/// rest of the theme still applies.
pub fn parse(text: &str, stem: &str, file: &str) -> Result<(Theme, Vec<String>), String> {
    let f: ThemeFile = toml::from_str(text).map_err(|e| {
        let line = e.span().map(|r| text[..r.start.min(text.len())].matches('\n').count() + 1);
        match line {
            Some(l) => format!("{file}: line {l}: {}", e.message()),
            None => format!("{file}: {}", e.message()),
        }
    })?;
    let base = match f.base.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("light") => "light",
        Some("dark") => "dark",
        Some(other) => return Err(format!("{file}: base must be \"light\" or \"dark\", not \"{other}\"")),
    };
    let mut warnings = Vec::new();
    let mut colors = BTreeMap::new();
    for (key, value) in f.colors {
        let key = key.trim_start_matches("--").to_string();
        if !KEYS.contains(&key.as_str()) {
            warnings.push(format!("{file}: unknown color \"{key}\""));
            continue;
        }
        match value.as_str() {
            Some(c) if valid_color(c) => {
                colors.insert(key, c.trim().to_string());
            }
            _ => warnings.push(format!("{file}: \"{key}\" is not a color: {value}")),
        }
    }
    let name = f.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| stem.to_string());
    Ok((Theme { name, base: base.into(), colors, file: Some(file.to_string()) }, warnings))
}

/// Built-in themes plus every `*.toml` in `dir`, sorted by file name.
pub fn load(dir: &Path) -> ThemeList {
    let mut themes = vec![builtin("Light", "light"), builtin("Dark", "dark")];
    let mut warnings = Vec::new();
    let mut files: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml")) && p.is_file())
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    for path in files {
        let file = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!("{file}: {e}"));
                continue;
            }
        };
        match parse(&text, &stem, &file) {
            Ok((theme, w)) => {
                warnings.extend(w);
                match themes.iter_mut().find(|t| t.name.eq_ignore_ascii_case(&theme.name)) {
                    Some(existing) => {
                        if let Some(other) = &existing.file {
                            warnings.push(format!("{file}: replaces theme \"{}\" from {other}", theme.name));
                        }
                        *existing = theme;
                    }
                    None => themes.push(theme),
                }
            }
            Err(e) => warnings.push(e),
        }
    }
    ThemeList { dir: dir.display().to_string(), keys: KEYS.to_vec(), themes, warnings }
}

/// Creates the folder with an example theme the first time; afterwards the
/// user's deletions stick.
pub fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    if dir.exists() {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    fs::write(dir.join(EXAMPLE_FILE), EXAMPLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_validate() {
        let (t, w) = parse(
            "name = \"Mine\"\nbase = \"Dark\"\n[colors]\nbg = \"#112233\"\n\"--fg\" = \"rgb(1, 2, 3)\"\n\
             accent = \"tomato\"\nnope = \"#fff\"\nsel = \"#12345\"\nhl = 5\n",
            "mine",
            "mine.toml",
        )
        .unwrap();
        assert_eq!((t.name.as_str(), t.base.as_str()), ("Mine", "dark"));
        assert_eq!(t.colors["bg"], "#112233");
        assert_eq!(t.colors["fg"], "rgb(1, 2, 3)");
        assert_eq!(t.colors["accent"], "tomato");
        assert_eq!(t.colors.len(), 3);
        assert_eq!(w.len(), 3, "{w:?}"); // nope, sel, hl
        assert!(w[0].starts_with("mine.toml: "));

        // name defaults to the file name, base to light
        let (t, _) = parse("[colors]\nbg = \"#fff\"", "plain", "plain.toml").unwrap();
        assert_eq!((t.name.as_str(), t.base.as_str()), ("plain", "light"));

        assert!(parse("base = \"blue\"", "x", "x.toml").is_err());
        let e = parse("bg = \"#fff\"", "x", "x.toml").unwrap_err(); // colors outside [colors]
        assert!(e.starts_with("x.toml: line 1: unknown field `bg`"), "{e}");
        let e = parse("# c\nname = ", "x", "x.toml").unwrap_err();
        assert!(e.starts_with("x.toml: line 2: ") && !e.contains('\n'), "{e}");
        assert!(!valid_color("red; x: y") && !valid_color("url(a)") && !valid_color("#ggg"));
        // the bundled example is valid and complete
        let (t, w) = parse(EXAMPLE, "solarized-dark", EXAMPLE_FILE).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(t.colors.len(), KEYS.len());
    }

    #[test]
    fn load_folder() {
        let dir = std::env::temp_dir().join(format!("hexedit-themes-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();
        fs::write(dir.join("a.toml"), "name = \"dark\"\nbase = \"dark\"\n[colors]\nbg = \"#000\"").unwrap();
        fs::write(dir.join("b.toml"), "name = \"Broken").unwrap();
        fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let l = load(&dir);
        let names: Vec<&str> = l.themes.iter().map(|t| t.name.as_str()).collect();
        // the file replaces the built-in Dark in place
        assert_eq!(names, ["Light", "dark", "Solarized Dark"]);
        assert_eq!(l.themes[1].colors["bg"], "#000");
        assert_eq!(l.warnings.len(), 1);
        assert!(l.warnings[0].starts_with("b.toml: "));
        // an existing folder is left alone
        fs::remove_file(dir.join(EXAMPLE_FILE)).unwrap();
        ensure_dir(&dir).unwrap();
        assert!(!dir.join(EXAMPLE_FILE).exists());
        fs::remove_dir_all(&dir).unwrap();
        // a missing folder only gives the built-ins
        assert_eq!(load(&dir).themes.len(), 2);
    }
}
