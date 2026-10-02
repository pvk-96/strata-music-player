//! Keyboard shortcut value type.
//!
//! A shortcut is a key plus a set of modifiers, stored as a human readable string such as
//! `Space`, `F`, `Ctrl+K` or `Ctrl+Shift+P`. This type has no toolkit dependency; the GTK
//! layer converts it with [`gtk::gdk`] primitives.

use std::fmt;
use std::str::FromStr;

use serde::de::{self, Deserialize, Deserializer};
use serde::{Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Modifiers(u8);

const CTRL: u8 = 1 << 0;
const ALT: u8 = 1 << 1;
const SHIFT: u8 = 1 << 2;
const SUPER: u8 = 1 << 3;

impl Modifiers {
    pub const NONE: Modifiers = Modifiers(0);
    pub const CONTROL: Modifiers = Modifiers(CTRL);
    pub const ALT: Modifiers = Modifiers(ALT);
    pub const SHIFT: Modifiers = Modifiers(SHIFT);
    pub const SUPER: Modifiers = Modifiers(SUPER);

    pub fn empty(&self) -> bool {
        self.0 == 0
    }

    pub fn contains(&self, other: Modifiers) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn insert(&mut self, other: Modifiers) {
        self.0 |= other.0;
    }

    pub fn toggle(&mut self, other: Modifiers) {
        self.0 ^= other.0;
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.contains(Modifiers(CTRL)) {
            parts.push("Ctrl");
        }
        if self.contains(Modifiers(ALT)) {
            parts.push("Alt");
        }
        if self.contains(Modifiers(SHIFT)) {
            parts.push("Shift");
        }
        if self.contains(Modifiers(SUPER)) {
            parts.push("Super");
        }
        write!(f, "{}", parts.join("+"))
    }
}

/// A single key with optional modifiers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Shortcut {
    modifiers: Modifiers,
    /// Canonical key name: `Space`, `Delete`, `F5`, `BracketLeft`, or a single character.
    key: String,
}

/// Canonical GDK key names Strata accepts, in the spelling used when writing settings.
const NAMED_KEYS: &[&str] = &[
    "Space",
    "Return",
    "Escape",
    "Delete",
    "Insert",
    "BackSpace",
    "Tab",
    "Page_Up",
    "Page_Down",
    "Home",
    "End",
    "Up",
    "Down",
    "Left",
    "Right",
    "BracketLeft",
    "BracketRight",
    "plus",
    "minus",
];

/// Keys shown with a symbol rather than their name.
const KEY_SYMBOLS: &[(&str, &str)] = &[
    ("BracketLeft", "["),
    ("BracketRight", "]"),
    ("plus", "+"),
    ("minus", "-"),
];

/// Common spellings accepted from configuration files, mapped to canonical key names.
const KEY_ALIASES: &[(&str, &str)] = &[
    ("space", "Space"),
    ("return", "Return"),
    ("enter", "Return"),
    ("esc", "Escape"),
    ("escape", "Escape"),
    ("del", "Delete"),
    ("ins", "Insert"),
    ("backspace", "BackSpace"),
    ("tab", "Tab"),
    ("up", "Up"),
    ("down", "Down"),
    ("left", "Left"),
    ("right", "Right"),
    ("home", "Home"),
    ("end", "End"),
    ("pageup", "Page_Up"),
    ("pagedown", "Page_Down"),
    ("page_up", "Page_Up"),
    ("page_down", "Page_Down"),
    ("[", "BracketLeft"),
    ("]", "BracketRight"),
    ("+", "plus"),
    ("-", "minus"),
    ("plus", "plus"),
    ("minus", "minus"),
];

impl Shortcut {
    pub fn new(modifiers: Modifiers, key: impl Into<String>) -> Result<Self, ShortcutError> {
        let raw = key.into();
        let key = canonical_key(&raw).ok_or(ShortcutError)?;
        Ok(Self { modifiers, key })
    }

    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    /// Readable form such as `Ctrl+Shift+P` or `]`.
    pub fn to_label(&self) -> String {
        let key = KEY_SYMBOLS
            .iter()
            .find(|(name, _)| *name == self.key)
            .map(|(_, symbol)| (*symbol).to_string())
            .unwrap_or_else(|| self.key.clone());
        if self.modifiers.empty() {
            key
        } else {
            format!("{}+{}", self.modifiers, key)
        }
    }
}

/// Normalise a key spelling, returning `None` when it is empty.
fn canonical_key(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    for (alias, canonical) in KEY_ALIASES {
        if lower == *alias {
            return Some((*canonical).to_string());
        }
    }
    // Named keys are accepted in any case and stored in GDK's spelling.
    for name in NAMED_KEYS {
        if lower == name.to_ascii_lowercase() {
            return Some((*name).to_string());
        }
    }
    // Single characters are stored upper case so `n` and `N` describe the same key.
    let mut chars = trimmed.chars();
    let first = chars.next()?;
    if chars.next().is_none() {
        if first.is_ascii_alphabetic() {
            return Some(first.to_ascii_uppercase().to_string());
        }
        if first.is_ascii() {
            return Some(trimmed.to_string());
        }
    }
    // Named keys are written in GDK's PascalCase / underscore style ("F5", "Page_Up").
    Some(trimmed.to_string())
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_label())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutError;

impl fmt::Display for ShortcutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("empty or unrecognised key")
    }
}

impl std::error::Error for ShortcutError {}

impl FromStr for Shortcut {
    type Err = ShortcutError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ShortcutError);
        }
        let mut parts: Vec<&str> = text.split('+').collect();
        let key_part = parts.pop().ok_or(ShortcutError)?;
        let mut modifiers = Modifiers::default();
        for part in parts {
            match part.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers.insert(Modifiers(CTRL)),
                "alt" | "meta" => modifiers.insert(Modifiers(ALT)),
                "shift" => modifiers.insert(Modifiers(SHIFT)),
                "super" | "cmd" | "win" => modifiers.insert(Modifiers(SUPER)),
                "" => return Err(ShortcutError),
                other => {
                    log::warn!("ignoring unknown key modifier '{other}' in shortcut '{text}'");
                }
            }
        }
        Shortcut::new(modifiers, key_part)
    }
}

impl Serialize for Shortcut {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_label())
    }
}

impl<'de> Deserialize<'de> for Shortcut {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Shortcut::from_str(&text)
            .map_err(|_| de::Error::custom(format!("invalid shortcut '{text}'")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcut(text: &str) -> Shortcut {
        Shortcut::from_str(text).unwrap()
    }

    #[test]
    fn plain_keys_are_accepted() {
        assert_eq!(shortcut("space").key(), "Space");
        assert_eq!(shortcut("n").key(), "N");
        assert_eq!(shortcut("N").key(), "N");
        assert_eq!(shortcut("F5").key(), "F5");
        assert_eq!(shortcut("delete").key(), "Delete");
    }

    #[test]
    fn modifiers_are_parsed_and_normalised() {
        let ctrl_k = shortcut("ctrl+k");
        assert!(ctrl_k.modifiers().contains(Modifiers::CONTROL));
        assert_eq!(ctrl_k.key(), "K");
        assert_eq!(ctrl_k.to_label(), "Ctrl+K");
        assert_eq!(shortcut("Control+Shift+P").to_label(), "Ctrl+Shift+P");
        assert_eq!(shortcut("cmd+k").to_label(), "Super+K");
    }

    #[test]
    fn bracket_aliases_map_to_key_names() {
        assert_eq!(shortcut("[").key(), "BracketLeft");
        assert_eq!(shortcut("]").key(), "BracketRight");
    }

    #[test]
    fn label_round_trips() {
        for text in [
            "Space",
            "N",
            "Ctrl+K",
            "Ctrl+Shift+P",
            "[",
            "]",
            "F5",
            "Alt+Page_Down",
        ] {
            assert_eq!(shortcut(text).to_label(), text);
        }
    }

    #[test]
    fn empty_shortcut_is_rejected() {
        assert!(Shortcut::from_str("  ").is_err());
        assert!(Shortcut::from_str("Ctrl+").is_err());
        assert!(Shortcut::from_str("+").is_err());
    }

    #[test]
    fn unknown_modifier_is_ignored_with_a_warning() {
        assert_eq!(shortcut("Hyper+K").to_label(), "K");
    }

    #[test]
    fn serde_uses_the_label() {
        let value = shortcut("Ctrl+K");
        let encoded = toml::Value::try_from(&value).unwrap();
        assert_eq!(encoded.as_str(), Some("Ctrl+K"));
        let decoded: Shortcut = encoded.try_into().unwrap();
        assert_eq!(decoded, value);
    }
}
