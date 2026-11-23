//! Keyboard shortcuts as text, such as `Ctrl+Shift+N` or `F2`: read, written
//! the same way back, and checked for being safe to bind while typing.

use std::fmt;

/// A key with the modifiers held. `ctrl` is the command key: Ctrl on Linux
/// and Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: Key,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A letter, digit or punctuation key, lowercase.
    Char(char),
    /// F1 to F24.
    F(u8),
}

impl Chord {
    /// Reads `Ctrl+Alt+Shift+K`, `ctrl+1`, `F5`, `Shift+F2`: modifiers in any
    /// order and case, then one key.
    pub fn parse(text: &str) -> Option<Chord> {
        let mut chord = Chord { ctrl: false, alt: false, shift: false, key: Key::Char(' ') };
        let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
        // `Ctrl++` names the plus key.
        if text.trim_end().ends_with("++") {
            parts.truncate(parts.len().saturating_sub(2));
            parts.push("+");
        }
        let key = parts.pop()?;
        for part in parts {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" => chord.ctrl = true,
                "alt" | "option" => chord.alt = true,
                "shift" => chord.shift = true,
                _ => return None,
            }
        }
        let mut chars = key.chars();
        chord.key = match (chars.next(), chars.next()) {
            (Some(c), None) if !c.is_control() && !c.is_whitespace() => Key::Char(c.to_ascii_lowercase()),
            _ => match key.strip_prefix(['F', 'f']).and_then(|n| n.parse::<u8>().ok()) {
                Some(n @ 1..=24) => Key::F(n),
                _ => return None,
            },
        };
        Some(chord)
    }

    /// Why this chord should not be bound, if it should not: it has to hold
    /// Ctrl, or be a function key, so it never fires while typing; and Ctrl+Alt
    /// alone is AltGr on many keyboards, which types letters such as ą or €.
    pub fn unusable(&self) -> Option<&'static str> {
        match self.key {
            Key::F(_) => None,
            Key::Char(_) if !self.ctrl => Some("a shortcut needs Ctrl, or a function key such as F5"),
            Key::Char(_) if self.alt && !self.shift => {
                Some("Ctrl+Alt is AltGr on many keyboards, which types letters; add Shift, or leave Alt out")
            }
            Key::Char(_) => None,
        }
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [(self.ctrl, "Ctrl+"), (self.alt, "Alt+"), (self.shift, "Shift+")] {
            if on {
                f.write_str(name)?;
            }
        }
        match self.key {
            Key::Char(c) => write!(f, "{}", c.to_ascii_uppercase()),
            Key::F(n) => write!(f, "F{n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_and_written_alike() {
        for text in ["Ctrl+P", "Ctrl+Shift+N", "F2", "Ctrl+Alt+Shift+K", "Ctrl+1", "Shift+F12", "Ctrl++"] {
            assert_eq!(Chord::parse(text).unwrap().to_string(), text);
        }
        assert_eq!(Chord::parse(" shift + ctrl + k ").unwrap().to_string(), "Ctrl+Shift+K", "any order and case");
        assert_eq!(Chord::parse("f5"), Some(Chord { ctrl: false, alt: false, shift: false, key: Key::F(5) }));
        for bad in ["", "Ctrl+", "Ctrl+Hyper+K", "Ctrl+KK", "F25", "F0", "Ctrl+ "] {
            assert_eq!(Chord::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_chords_that_cannot_type_text() {
        let usable = |text: &str| Chord::parse(text).unwrap().unusable().is_none();
        assert!(usable("Ctrl+K") && usable("Ctrl+Shift+K") && usable("Ctrl+Alt+Shift+K"));
        assert!(usable("F5") && usable("Alt+F3"));
        assert!(!usable("K") && !usable("Shift+K") && !usable("Alt+K"), "these type text");
        assert!(!usable("Ctrl+Alt+A"), "AltGr+A types ą");
    }
}
