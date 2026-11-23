//! Sage & Paper: design tokens, fonts and widget styles.
//!
//! Every colour in the app comes from [`Tokens`]. Widgets are styled with the
//! functions in this module and never read iced's generated palette, which is
//! how blue, purple, gradients and shadows are kept out by construction.

use iced::overlay;
use iced::widget::{button, container, text_editor, text_input};
use iced::widget::{checkbox as check_box, pick_list as drop};
use iced::{font, Background, Border, Color, Font, Shadow, Theme};

const fn hex(rgb: u32) -> Color {
    Color {
        r: ((rgb >> 16) & 0xFF) as f32 / 255.0,
        g: ((rgb >> 8) & 0xFF) as f32 / 255.0,
        b: (rgb & 0xFF) as f32 / 255.0,
        a: 1.0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    /// Page background.
    pub bg: Color,
    /// Code blocks, inline code, tags, selected rows.
    pub surface: Color,
    /// Primary text and headings.
    pub text: Color,
    /// Secondary text, metadata, inactive navigation.
    pub text_2: Color,
    /// Links, current state, focus ring, highlight rule.
    pub accent: Color,
    /// Accent hover and active.
    pub accent_2: Color,
    /// Hairline dividers, always 1px.
    pub border: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
}

impl Tokens {
    pub const LIGHT: Self = Self {
        bg: hex(0xFAF8F5),
        surface: hex(0xF1EDE7),
        text: hex(0x1C1917),
        text_2: hex(0x57534E),
        accent: hex(0x8A1C2B),
        accent_2: hex(0x6E1422),
        border: hex(0xDDD6CE),
        success: hex(0x2E6B3F),
        warning: hex(0x8A5300),
        error: hex(0xB42318),
    };

    pub const DARK: Self = Self {
        bg: hex(0x161413),
        surface: hex(0x211E1C),
        text: hex(0xEDE8E3),
        text_2: hex(0xB8B2AC),
        accent: hex(0xEE9AA3),
        accent_2: hex(0xF6BCC2),
        border: hex(0x3A3532),
        success: hex(0x6FBF82),
        warning: hex(0xE0A84A),
        error: hex(0xF07A6A),
    };

    pub const fn of(dark: bool) -> Self {
        if dark {
            Self::DARK
        } else {
            Self::LIGHT
        }
    }

    /// The iced theme that backs widgets we do not style by hand (scrollbars).
    /// Its palette is fed from the tokens so nothing falls back to iced's blue.
    pub fn iced_theme(self, dark: bool) -> Theme {
        Theme::custom(
            if dark { "Sage & Paper (dark)" } else { "Sage & Paper (light)" },
            iced::theme::Palette {
                background: self.bg,
                text: self.text,
                primary: self.accent,
                success: self.success,
                warning: self.warning,
                danger: self.error,
            },
        )
    }
}

/// The only corner radius in the app. Never pills.
pub const RADIUS: f32 = 3.0;

// Fonts are compiled into the binary so the app looks the same on every machine.
// They live in assets/fonts, with their OFL licences in assets/fonts/licenses.
// The text engine does not
// slant an upright face, so emphasis in the preview needs the italic files.
pub const FONT_FILES: [&[u8]; 8] = [
    include_bytes!("../../../assets/fonts/SourceSerif4-Regular.ttf"),
    include_bytes!("../../../assets/fonts/SourceSerif4-Semibold.ttf"),
    include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf"),
    include_bytes!("../../../assets/fonts/SourceSans3-Semibold.ttf"),
    include_bytes!("../../../assets/fonts/SourceSans3-It.ttf"),
    include_bytes!("../../../assets/fonts/SourceSans3-SemiboldIt.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../../../assets/fonts/NotoEmoji.ttf"),
];

/// Headings.
pub const SERIF: Font = Font {
    family: font::Family::Name("Source Serif 4"),
    weight: font::Weight::Semibold,
    ..Font::DEFAULT
};
/// Body and UI.
pub const SANS: Font = Font::with_name("Source Sans 3");
pub const SANS_BOLD: Font = Font {
    family: font::Family::Name("Source Sans 3"),
    weight: font::Weight::Semibold,
    ..Font::DEFAULT
};
/// Editor and code.
pub const MONO: Font = Font::with_name("JetBrains Mono");

fn flat(background: Option<Color>, text: Color) -> container::Style {
    container::Style {
        text_color: Some(text),
        background: background.map(Background::Color),
        border: Border::default(),
        shadow: Shadow::default(),
        snap: true,
    }
}

/// A plain region on the page background.
pub fn page(t: Tokens) -> container::Style {
    flat(Some(t.bg), t.text)
}

/// A 1px divider. Give the container a fixed width or height of 1.
pub fn hairline(t: Tokens) -> container::Style {
    flat(Some(t.border), t.text)
}

/// Code blocks and other inset surfaces.
pub fn surface(t: Tokens) -> container::Style {
    container::Style {
        border: Border { radius: RADIUS.into(), ..Border::default() },
        ..flat(Some(t.surface), t.text)
    }
}

/// A list item of the properties: a hairline box that holds its field and
/// its remove button.
pub fn chip(t: Tokens) -> container::Style {
    container::Style {
        border: Border { color: t.border, width: 1.0, radius: RADIUS.into() },
        ..flat(Some(t.surface), t.text)
    }
}

/// The field inside a chip, which draws the border itself.
pub fn chip_input(t: Tokens, status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border { width: 0.0, ..Border::default() },
        ..input(t, status)
    }
}

/// A floating panel: page colour with a hairline border and no shadow.
pub fn panel(t: Tokens) -> container::Style {
    container::Style {
        border: Border { color: t.border, width: 1.0, radius: RADIUS.into() },
        ..flat(Some(t.bg), t.text)
    }
}

/// Dims the workspace behind the command palette.
pub fn scrim(t: Tokens) -> container::Style {
    flat(Some(Color { a: 0.55, ..t.bg }), t.text)
}

/// The 2px rule that marks the current tab or row.
pub fn accent_rule(t: Tokens, on: bool) -> container::Style {
    flat(on.then_some(t.accent), t.text)
}

/// A solid block of one colour, such as the rule beside a callout.
pub fn fill(color: Color) -> container::Style {
    container::Style { background: Some(Background::Color(color)), ..container::Style::default() }
}

/// The background of `==highlighted==` text: the warning colour, washed out.
pub fn highlight(t: Tokens) -> Color {
    Color { a: 0.28, ..t.warning }
}

/// Selected text, in every editor and input.
pub fn selection(t: Tokens) -> Color {
    Color { a: 0.22, ..t.accent }
}

/// The find match in live preview: stronger than a highlight, so a match
/// inside highlighted text still shows.
pub fn find_match(t: Tokens) -> Color {
    Color { a: 0.35, ..t.warning }
}

/// A task checkbox: a hairline square, filled with the accent once done.
pub fn checkbox(t: Tokens, done: bool) -> container::Style {
    container::Style {
        background: done.then_some(Background::Color(t.accent)),
        border: Border { color: if done { t.accent } else { t.text_2 }, width: 1.0, radius: 2.0.into() },
        ..container::Style::default()
    }
}

/// A text-only button: navigation rows, tabs, palette results.
/// `current` renders it in the accent colour; hover uses the surface colour.
pub fn nav_button(t: Tokens, current: bool, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: (hovered || current).then_some(Background::Color(t.surface)),
        text_color: match (current, hovered) {
            (true, true) => t.accent_2,
            (true, false) => t.accent,
            (false, true) => t.text,
            (false, false) => t.text_2,
        },
        border: Border { radius: RADIUS.into(), ..Border::default() },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// An inline link: accent text, no background.
pub fn link_button(t: Tokens, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: None,
        text_color: if hovered { t.accent_2 } else { t.accent },
        border: Border::default(),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn editor(t: Tokens, _status: text_editor::Status) -> text_editor::Style {
    text_editor::Style {
        background: Background::Color(t.bg),
        border: Border::default(),
        placeholder: t.text_2,
        value: t.text,
        selection: selection(t),
    }
}

/// A drop-down choice: an input's frame, the accent once open.
pub fn drop_down(t: Tokens, status: drop::Status) -> drop::Style {
    let open = matches!(status, drop::Status::Opened { .. });
    drop::Style {
        text_color: t.text,
        placeholder_color: t.text_2,
        handle_color: t.text_2,
        background: Background::Color(t.bg),
        border: Border { color: if open { t.accent } else { t.border }, width: 1.0, radius: RADIUS.into() },
    }
}

/// The list a drop-down opens: a panel, the choice under the pointer on the surface.
pub fn menu(t: Tokens) -> overlay::menu::Style {
    overlay::menu::Style {
        background: Background::Color(t.bg),
        border: Border { color: t.border, width: 1.0, radius: RADIUS.into() },
        text_color: t.text,
        selected_text_color: t.accent,
        selected_background: Background::Color(t.surface),
        shadow: Shadow::default(),
    }
}

/// A settings checkbox, drawn like a task checkbox.
pub fn check(t: Tokens, status: check_box::Status) -> check_box::Style {
    let checked = match status {
        check_box::Status::Active { is_checked }
        | check_box::Status::Hovered { is_checked }
        | check_box::Status::Disabled { is_checked } => is_checked,
    };
    check_box::Style {
        background: Background::Color(if checked { t.accent } else { t.bg }),
        icon_color: t.bg,
        border: Border { color: if checked { t.accent } else { t.text_2 }, width: 1.0, radius: 2.0.into() },
        text_color: Some(t.text),
    }
}

pub fn input(t: Tokens, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(t.bg),
        border: Border {
            color: if focused { t.accent } else { t.border },
            width: 1.0,
            radius: RADIUS.into(),
        },
        icon: t.text_2,
        placeholder: t.text_2,
        value: t.text,
        selection: selection(t),
    }
}
