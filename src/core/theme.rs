use ratatui::style::Color;

/// Complete color/style theme for the TUI.
///
/// All fields are `ratatui::style::Color`. Truecolor values are stored as
/// `Color::Rgb(r, g, b)`. The lane palette is a fixed-length `Vec` that
/// wraps cyclically when rendered.
#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub dim: Color,

    /// Cycling lane colors for the commit graph (at least 5 entries).
    pub lane: Vec<Color>,

    // Diff colors
    pub added: Color,
    pub removed: Color,
    pub hunk: Color,

    // Branch / ref colors
    pub head: Color,
    pub staged: Color,
    pub unstaged: Color,
    pub untracked: Color,

    // Border colors
    pub focus_border: Color,
    pub border: Color,

    /// Background for the selected list row. Selection is indicated by
    /// background color ALONE — no prefix glyph, no bold — so rows never shift.
    /// Kept distinct from the focus-border hue and the graph lane palette.
    pub selection_bg: Color,
}

impl Theme {
    /// Tokyo Night — the default built-in theme.
    pub fn tokyonight() -> Self {
        Self {
            bg: Color::Rgb(0x1a, 0x1b, 0x26),
            fg: Color::Rgb(0xc0, 0xca, 0xf5),
            dim: Color::Rgb(0x6b, 0x73, 0x94),
            lane: vec![
                Color::Rgb(0x7a, 0xa2, 0xf7), // blue
                Color::Rgb(0x9e, 0xce, 0x6a), // green
                Color::Rgb(0xbb, 0x9a, 0xf7), // purple
                Color::Rgb(0x7d, 0xcf, 0xff), // cyan
                Color::Rgb(0xff, 0x9e, 0x64), // orange
            ],
            added: Color::Rgb(0x9e, 0xce, 0x6a),
            removed: Color::Rgb(0xf7, 0x76, 0x8e),
            hunk: Color::Rgb(0xbb, 0x9a, 0xf7),
            head: Color::Rgb(0xe0, 0xaf, 0x68),
            staged: Color::Rgb(0x9e, 0xce, 0x6a),
            unstaged: Color::Rgb(0xe0, 0xaf, 0x68),
            // Untracked = a clearly-readable muted blue-gray, distinctly lighter
            // than `dim` so new files don't disappear into the background.
            untracked: Color::Rgb(0x82, 0x8b, 0xb8),
            // Focus border = warm amber, deliberately distinct from the cool
            // blue lane[0] so a focused graph panel doesn't blend into the lanes.
            focus_border: Color::Rgb(0xe0, 0xaf, 0x68),
            border: Color::Rgb(0x6b, 0x73, 0x94),
            selection_bg: Color::Rgb(0x28, 0x34, 0x57),
        }
    }

    /// Catppuccin Mocha — warm dark theme.
    ///
    /// Palette from <https://github.com/catppuccin/catppuccin>.
    pub fn catppuccin() -> Self {
        Self {
            bg: Color::Rgb(0x1e, 0x1e, 0x2e),  // base
            fg: Color::Rgb(0xcd, 0xd6, 0xf4),  // text
            dim: Color::Rgb(0x6c, 0x70, 0x86), // overlay0 (lighter than surface2)
            lane: vec![
                Color::Rgb(0x89, 0xb4, 0xfa), // blue
                Color::Rgb(0xa6, 0xe3, 0xa1), // green
                Color::Rgb(0xcb, 0xa6, 0xf7), // mauve
                Color::Rgb(0x89, 0xdc, 0xeb), // sky
                Color::Rgb(0xfa, 0xb3, 0x87), // peach
            ],
            added: Color::Rgb(0xa6, 0xe3, 0xa1),        // green
            removed: Color::Rgb(0xf3, 0x8b, 0xa8),      // red
            hunk: Color::Rgb(0xcb, 0xa6, 0xf7),         // mauve
            head: Color::Rgb(0xf9, 0xe2, 0xaf),         // yellow
            staged: Color::Rgb(0xa6, 0xe3, 0xa1),       // green
            unstaged: Color::Rgb(0xf9, 0xe2, 0xaf),     // yellow
            untracked: Color::Rgb(0x93, 0x99, 0xb2),    // overlay2 — readable muted gray
            focus_border: Color::Rgb(0xf9, 0xe2, 0xaf), // yellow — distinct from blue lane[0]
            border: Color::Rgb(0x6c, 0x70, 0x86),       // overlay0
            selection_bg: Color::Rgb(0x45, 0x47, 0x5a), // surface1
        }
    }

    /// Nord — arctic, north-bluish theme.
    ///
    /// Palette from <https://www.nordtheme.com/docs/colors-and-palettes>.
    pub fn nord() -> Self {
        Self {
            bg: Color::Rgb(0x2e, 0x34, 0x40),  // nord0
            fg: Color::Rgb(0xec, 0xef, 0xf4),  // nord6
            dim: Color::Rgb(0x61, 0x6e, 0x88), // nord3, brightened
            lane: vec![
                Color::Rgb(0x81, 0xa1, 0xc1), // nord9  – blue
                Color::Rgb(0xa3, 0xbe, 0x8c), // nord14 – green
                Color::Rgb(0xb4, 0x8e, 0xad), // nord15 – purple
                Color::Rgb(0x88, 0xc0, 0xd0), // nord8  – cyan
                Color::Rgb(0xd0, 0x87, 0x70), // nord12 – orange
            ],
            added: Color::Rgb(0xa3, 0xbe, 0x8c),   // nord14 green
            removed: Color::Rgb(0xbf, 0x61, 0x6a), // nord11 red
            hunk: Color::Rgb(0xb4, 0x8e, 0xad),    // nord15 purple
            head: Color::Rgb(0xeb, 0xcb, 0x8b),    // nord13 yellow
            staged: Color::Rgb(0xa3, 0xbe, 0x8c),  // nord14 green
            unstaged: Color::Rgb(0xeb, 0xcb, 0x8b), // nord13 yellow
            untracked: Color::Rgb(0x8a, 0x93, 0xa8), // light gray — readable
            focus_border: Color::Rgb(0xeb, 0xcb, 0x8b), // nord13 yellow — distinct from blue/cyan lanes
            border: Color::Rgb(0x61, 0x6e, 0x88),       // nord3, brightened
            selection_bg: Color::Rgb(0x43, 0x4c, 0x5e), // nord2
        }
    }

    /// Gruvbox Dark — retro groove theme.
    ///
    /// Palette from <https://github.com/morhetz/gruvbox>.
    pub fn gruvbox() -> Self {
        Self {
            bg: Color::Rgb(0x28, 0x28, 0x28),  // bg (hard)
            fg: Color::Rgb(0xeb, 0xdb, 0xb2),  // fg1
            dim: Color::Rgb(0x7c, 0x6f, 0x64), // between bg4 and gray — lighter
            lane: vec![
                Color::Rgb(0x83, 0xa5, 0x98), // blue4
                Color::Rgb(0xb8, 0xbb, 0x26), // green bright
                Color::Rgb(0xd3, 0x86, 0x9b), // purple
                Color::Rgb(0x8e, 0xc0, 0x7c), // aqua/green
                Color::Rgb(0xfe, 0x80, 0x19), // orange
            ],
            added: Color::Rgb(0xb8, 0xbb, 0x26),   // green bright
            removed: Color::Rgb(0xcc, 0x24, 0x1d), // red bright
            hunk: Color::Rgb(0xd3, 0x86, 0x9b),    // purple
            head: Color::Rgb(0xfa, 0xbd, 0x2f),    // yellow bright
            staged: Color::Rgb(0xb8, 0xbb, 0x26),  // green bright
            unstaged: Color::Rgb(0xfa, 0xbd, 0x2f), // yellow bright
            untracked: Color::Rgb(0xa8, 0x99, 0x84), // gruvbox gray — readable
            focus_border: Color::Rgb(0xfa, 0xbd, 0x2f), // yellow — distinct from blue lane[0]/orange lane[4]
            border: Color::Rgb(0x7c, 0x6f, 0x64),       // between bg4 and gray
            selection_bg: Color::Rgb(0x3c, 0x38, 0x36), // bg1
        }
    }

    /// Look up a built-in theme by name (case-insensitive).
    ///
    /// Unknown names fall back to Tokyo Night.
    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "tokyonight" | "tokyo-night" | "tokyo_night" => Self::tokyonight(),
            "catppuccin" | "catppuccin-mocha" | "catppuccin_mocha" => Self::catppuccin(),
            "nord" => Self::nord(),
            "gruvbox" | "gruvbox-dark" | "gruvbox_dark" => Self::gruvbox(),
            _ => Self::tokyonight(),
        }
    }

    /// Return the canonical names of all built-in themes.
    pub fn theme_names() -> Vec<&'static str> {
        vec!["tokyonight", "catppuccin", "nord", "gruvbox"]
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::tokyonight()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each named theme must produce a distinct background color.
    #[test]
    fn theme_bg_colors_are_distinct() {
        let names = Theme::theme_names();
        let bgs: Vec<Color> = names.iter().map(|n| Theme::from_name(n).bg).collect();

        // All pairs must differ.
        for i in 0..bgs.len() {
            for j in (i + 1)..bgs.len() {
                assert_ne!(
                    bgs[i], bgs[j],
                    "themes '{}' and '{}' share the same bg color",
                    names[i], names[j]
                );
            }
        }
    }

    /// Unknown names fall back to Tokyo Night.
    #[test]
    fn from_name_unknown_falls_back_to_tokyonight() {
        let fallback = Theme::from_name("not-a-real-theme");
        let expected = Theme::tokyonight();
        assert_eq!(fallback.bg, expected.bg);
        assert_eq!(fallback.fg, expected.fg);
    }

    /// Case-insensitive matching works.
    #[test]
    fn from_name_is_case_insensitive() {
        assert_eq!(Theme::from_name("NORD").bg, Theme::nord().bg);
        assert_eq!(Theme::from_name("Gruvbox").bg, Theme::gruvbox().bg);
        assert_eq!(Theme::from_name("Catppuccin").bg, Theme::catppuccin().bg);
        assert_eq!(Theme::from_name("TokyoNight").bg, Theme::tokyonight().bg);
    }

    /// Alternate alias spellings resolve to the same theme.
    #[test]
    fn from_name_aliases() {
        assert_eq!(Theme::from_name("tokyo-night").bg, Theme::tokyonight().bg);
        assert_eq!(
            Theme::from_name("catppuccin-mocha").bg,
            Theme::catppuccin().bg
        );
        assert_eq!(Theme::from_name("gruvbox-dark").bg, Theme::gruvbox().bg);
    }

    /// Every theme has at least 5 lane colors, each distinct from the others.
    #[test]
    fn lane_colors_are_distinct_within_each_theme() {
        for name in Theme::theme_names() {
            let theme = Theme::from_name(name);
            assert!(
                theme.lane.len() >= 5,
                "theme '{}' has fewer than 5 lane colors",
                name
            );
            for i in 0..theme.lane.len() {
                for j in (i + 1)..theme.lane.len() {
                    assert_ne!(
                        theme.lane[i], theme.lane[j],
                        "theme '{}': lane[{}] == lane[{}]",
                        name, i, j
                    );
                }
            }
        }
    }

    /// theme_names returns exactly the four expected canonical names.
    #[test]
    fn theme_names_list() {
        let names = Theme::theme_names();
        assert_eq!(names, vec!["tokyonight", "catppuccin", "nord", "gruvbox"]);
    }

    /// The focus-border hue must differ from EVERY graph lane color — otherwise a
    /// focused graph panel's border blends into the lanes (user-reported clash).
    #[test]
    fn focus_border_distinct_from_lanes() {
        for name in Theme::theme_names() {
            let t = Theme::from_name(name);
            for (i, lane) in t.lane.iter().enumerate() {
                assert_ne!(
                    &t.focus_border, lane,
                    "theme '{}': focus_border must differ from lane[{}]",
                    name, i
                );
            }
        }
    }
}
