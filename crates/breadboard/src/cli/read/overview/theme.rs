//! The colours herdr's sidebar paints with, so the printed view reads the
//! same (herdr-tones.mjs, names.mjs): whom an agent waits for (at work
//! yellow, GitLab blue, you red, free teal) from herdr's config, the
//! terminal's own without one; a worktree's name in the sidebar's pink.

use super::paint::{blue, cyan, dim, hex, red, yellow};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The colours of a herdr config, by name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Theme {
    pub yellow: Option<String>,
    pub teal: Option<String>,
    pub red: Option<String>,
    pub blue: Option<String>,
    /// The pink of branches, `[theme.custom]`'s `pink`, else herdr's own.
    pub pink: String,
}

static THEME: OnceLock<Theme> = OnceLock::new();

/// `name = "#rrggbb"` of a colour in a config's text; the last one wins.
fn colour_in(text: &str, name: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == name).then_some(())?;
            let value = value.trim().strip_prefix('"')?;
            let colour = value.get(..7)?;
            (colour.starts_with('#')
                && colour[1..].chars().all(|c| c.is_ascii_hexdigit())
                && value[7..].starts_with('"'))
            .then(|| colour.to_string())
        })
        .next_back()
}

/// The section `[name]` of a TOML text, up to the next table.
fn section_of<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let head = format!("[{name}]");
    let at = text
        .match_indices(&head)
        .find(|(at, _)| *at == 0 || text[..*at].ends_with('\n'))?
        .0;
    let rest = &text[at..];
    let end = rest[1..].find("\n[").map(|end| end + 1).unwrap_or(rest.len());
    Some(&rest[..end])
}

impl Theme {
    pub fn of(tones: &str, config: &str) -> Theme {
        Theme {
            yellow: colour_in(tones, "yellow"),
            teal: colour_in(tones, "teal"),
            red: colour_in(tones, "red"),
            blue: colour_in(tones, "blue"),
            pink: section_of(config, "theme.custom")
                .and_then(|section| colour_in(section, "pink"))
                .unwrap_or_else(|| "#FF79C6".into()),
        }
    }
}

/// The theme of the herdr this runs with, read once.
pub fn theme(main: &Path) -> &'static Theme {
    THEME.get_or_init(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let xdg = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let tones = std::fs::read_to_string(xdg.join("herdr").join("config.toml")).unwrap_or_default();
        let config = [
            std::env::var_os("HERDR_CONFIG_PATH").map(PathBuf::from),
            Some(main.join(".herdr").join("config.toml")),
            Some(home.join(".config").join("herdr").join("config.toml")),
        ]
        .into_iter()
        .flatten()
        .find(|file| file.exists())
        .and_then(|file| std::fs::read_to_string(file).ok())
        .unwrap_or_default();
        Theme::of(&tones, &config)
    })
}

/// Whom an agent waits for, painted (herdr-tones.mjs `paintOf`): `work`,
/// `gitlab`, `you`, `free`; dim for another.
pub fn paint_tone(theme: &Theme, tone: Option<&str>, text: &str) -> String {
    let (colour, fallback): (&Option<String>, fn(&str) -> String) = match tone {
        Some("work") => (&theme.yellow, yellow),
        Some("gitlab") => (&theme.blue, blue),
        Some("you") => (&theme.red, red),
        Some("free") => (&theme.teal, cyan),
        _ => return dim(text),
    };
    match colour {
        Some(colour) => hex(colour, text),
        None => fallback(text),
    }
}

/// A worktree's name in the sidebar's pink.
pub fn pink(theme: &Theme, text: &str) -> String {
    hex(&theme.pink, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_colours_of_a_config() {
        let config =
            "[theme]\nname = \"x\"\n[theme.custom]\npink = \"#ff00aa\"\nred = \"#110000\"\n[ui]\npink = \"#000000\"\n";
        let theme = Theme::of("red = \"#aa0000\"\nteal = 'x'\n", config);
        assert_eq!(theme.red.as_deref(), Some("#aa0000"));
        assert_eq!(theme.teal, None);
        assert_eq!(theme.pink, "#ff00aa");
        assert_eq!(Theme::of("", "").pink, "#FF79C6");
    }
}
