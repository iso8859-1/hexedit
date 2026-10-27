//! Color themes loaded from TOML files in the app's config folder.
//!
//! Every theme is a file, including the standard Light and Dark ones, which are
//! installed into the folder like any other. A theme names a base (light or
//! dark); colors it leaves out come from the standard palette of that base,
//! the embedded copies of `themes/light.toml` and `themes/dark.toml`. Those
//! palettes also color the app when no Light/Dark theme is installed.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Color keys a theme can set; each is the CSS variable `--<key>`.
pub const KEYS: &[&str] = &[
    "bg", "bg-alt", "bg-panel", "fg", "muted", "faint", "border", "accent", "accent-fg", "sel", "sel-strong",
    "cursor", "hl", "error", "error-bg", "offset", "zero", "f0", "f1", "f2", "f3", "f4", "f5",
];

const LIGHT: &str = include_str!("../../themes/light.toml");
const DARK: &str = include_str!("../../themes/dark.toml");

/// Themes installed into the themes folder: (file name, content).
pub const STANDARD: &[(&str, &str)] = &[
    ("light.toml", LIGHT),
    ("dark.toml", DARK),
    ("solarized-dark.toml", include_str!("../../themes/solarized-dark.toml")),
    ("light-high-contrast.toml", include_str!("../../themes/light-high-contrast.toml")),
    ("dark-high-contrast.toml", include_str!("../../themes/dark-high-contrast.toml")),
    ("solarized-dark-high-contrast.toml", include_str!("../../themes/solarized-dark-high-contrast.toml")),
];

/// Lists the standard files already installed once, so deleted ones stay deleted
/// and files added in later versions still get installed.
const INSTALLED_LIST: &str = ".installed";

#[derive(Debug, Clone, Serialize)]
pub struct Theme {
    pub name: String,
    /// "light" or "dark": where the colors not set here come from, and the window title bar
    pub base: String,
    /// key (without "--") -> CSS color
    pub colors: BTreeMap<String, String>,
    /// the file it was loaded from
    pub file: String,
}

#[derive(Debug, Serialize)]
pub struct ThemeList {
    pub dir: String,
    /// the installed themes, sorted by name
    pub themes: Vec<Theme>,
    /// base -> complete standard palette, for colors a theme leaves out
    pub fallback: BTreeMap<&'static str, BTreeMap<String, String>>,
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
    Ok((Theme { name, base: base.into(), colors, file: file.to_string() }, warnings))
}

/// The standard palettes, from the files embedded at build time.
pub fn fallback() -> BTreeMap<&'static str, BTreeMap<String, String>> {
    [("light", LIGHT, "light.toml"), ("dark", DARK, "dark.toml")]
        .into_iter()
        .map(|(base, text, file)| {
            // checked complete and valid by the tests
            let colors = parse(text, "", file).map(|(t, _)| t.colors).unwrap_or_default();
            (base, colors)
        })
        .collect()
}

/// Every `*.toml` in `dir`. When two files use the same name, the one whose
/// file name sorts last wins.
pub fn load(dir: &Path) -> ThemeList {
    let mut themes: Vec<Theme> = Vec::new();
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
                        warnings.push(format!("{file}: replaces theme \"{}\" from {}", theme.name, existing.file));
                        *existing = theme;
                    }
                    None => themes.push(theme),
                }
            }
            Err(e) => warnings.push(e),
        }
    }
    themes.sort_by_key(|t| t.name.to_lowercase());
    ThemeList { dir: dir.display().to_string(), themes, fallback: fallback(), warnings }
}

/// Creates the folder and installs each standard theme once. A standard file
/// the user deleted is not installed again, and an existing file is never
/// overwritten.
pub fn install_standard(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let list_path = dir.join(INSTALLED_LIST);
    let mut installed: Vec<String> =
        fs::read_to_string(&list_path).unwrap_or_default().lines().map(str::to_string).collect();
    let before = installed.len();
    for (file, text) in STANDARD {
        if installed.iter().any(|f| f == file) {
            continue;
        }
        let path = dir.join(file);
        if !path.exists() {
            fs::write(&path, text)?;
        }
        installed.push(file.to_string());
    }
    if installed.len() != before {
        fs::write(&list_path, installed.join("\n") + "\n")?;
    }
    Ok(())
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
        // the standard themes are valid and complete
        for (file, text) in STANDARD {
            let (t, w) = parse(text, "", file).unwrap();
            assert!(w.is_empty(), "{file}: {w:?}");
            assert_eq!(t.colors.len(), KEYS.len(), "{file}");
        }
        let fb = fallback();
        assert_eq!((fb["light"].len(), fb["dark"].len()), (KEYS.len(), KEYS.len()));
        assert_ne!(fb["light"]["bg"], fb["dark"]["bg"]);
    }

    #[test]
    fn install_and_load() {
        let dir = std::env::temp_dir().join(format!("hexedit-themes-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        install_standard(&dir).unwrap();
        let names = |l: &ThemeList| l.themes.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(&load(&dir)),
            [
                "Dark",
                "Dark High Contrast",
                "Light",
                "Light High Contrast",
                "Solarized Dark",
                "Solarized Dark High Contrast"
            ]
        );

        fs::write(dir.join("my-dark.toml"), "name = \"dark\"\nbase = \"dark\"\n[colors]\nbg = \"#000\"").unwrap();
        fs::write(dir.join("b.toml"), "name = \"Broken").unwrap();
        fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let l = load(&dir);
        // my-dark.toml sorts after dark.toml and replaces its theme
        assert_eq!(names(&l)[..3], ["dark", "Dark High Contrast", "Light"]);
        assert_eq!(l.themes[0].colors, BTreeMap::from([("bg".to_string(), "#000".to_string())]));
        assert_eq!(l.warnings.len(), 2, "{:?}", l.warnings);
        assert!(l.warnings[0].starts_with("b.toml: "));
        assert!(l.warnings[1].starts_with("my-dark.toml: replaces theme"));

        // deleted standard themes stay deleted, edited ones are not overwritten
        fs::remove_file(dir.join("light.toml")).unwrap();
        fs::write(dir.join("dark.toml"), "name = \"Dark\"").unwrap();
        install_standard(&dir).unwrap();
        assert!(!dir.join("light.toml").exists());
        assert_eq!(fs::read_to_string(dir.join("dark.toml")).unwrap(), "name = \"Dark\"");

        // a folder from before the install list: missing standard files are added once
        fs::remove_file(dir.join(INSTALLED_LIST)).unwrap();
        install_standard(&dir).unwrap();
        assert!(dir.join("light.toml").exists());
        fs::remove_file(dir.join("light.toml")).unwrap();
        install_standard(&dir).unwrap();
        assert!(!dir.join("light.toml").exists());

        // an update adds the standard themes that are new since the last install, and only those
        for (file, _) in STANDARD {
            let _ = fs::remove_file(dir.join(file));
        }
        fs::write(dir.join(INSTALLED_LIST), "light.toml
dark.toml
solarized-dark.toml
").unwrap();
        install_standard(&dir).unwrap();
        assert!(!dir.join("light.toml").exists() && dir.join("light-high-contrast.toml").exists());
        assert_eq!(fs::read_to_string(dir.join(INSTALLED_LIST)).unwrap().lines().count(), STANDARD.len());

        // without files there are no themes, only the fallback palettes
        fs::remove_dir_all(&dir).unwrap();
        let l = load(&dir);
        assert!(l.themes.is_empty() && l.warnings.is_empty());
        assert_eq!(l.fallback.len(), 2);
    }
}
