//! Converting configured shortcuts into GTK accelerator strings.
//!
//! Settings use Strata's own spelling (`Ctrl+Up`, `Space`, `[`). GTK wants `<Ctrl>Up`,
//! `space`, `bracketleft`: a bare keyval nick, prefixed by `<Modifier>` parts. The mapping is
//! pure, so it is tested without a display.

use crate::settings::shortcut::{Modifiers, Shortcut};

/// Translate a stored key name into the keyval nick GTK accelerators use.
///
/// Named keys keep their capitalisation (`Up`, `F5`, `Page_Up`), punctuation uses the GDK
/// spelling (`comma`, `bracketleft`), and single letters are stored upper case but given to
/// GTK lower case.
fn gtk_key(key: &str) -> String {
    match key {
        "Space" | " " => "space".to_string(),
        "BracketLeft" | "[" => "bracketleft".to_string(),
        "BracketRight" | "]" => "bracketright".to_string(),
        "plus" | "+" => "plus".to_string(),
        "minus" | "-" => "minus".to_string(),
        "comma" | "," => "comma".to_string(),
        "period" | "." => "period".to_string(),
        "slash" | "/" => "slash".to_string(),
        "backslash" | "\\" => "backslash".to_string(),
        "semicolon" | ";" => "semicolon".to_string(),
        "apostrophe" | "'" => "apostrophe".to_string(),
        "grave" | "`" => "grave".to_string(),
        other => match other.chars().next() {
            Some(first) if other.chars().count() == 1 => first.to_lowercase().collect(),
            _ => other.to_string(),
        },
    }
}

/// Accelerator string for a shortcut, in the form `set_accels_for_action` expects.
pub fn accelerator(shortcut: &Shortcut) -> String {
    let mut parts: Vec<String> = Vec::new();
    let modifiers = shortcut.modifiers();
    if modifiers.contains(Modifiers::CONTROL) {
        parts.push("<Ctrl>".to_string());
    }
    if modifiers.contains(Modifiers::ALT) {
        parts.push("<Alt>".to_string());
    }
    if modifiers.contains(Modifiers::SHIFT) {
        parts.push("<Shift>".to_string());
    }
    if modifiers.contains(Modifiers::SUPER) {
        parts.push("<Super>".to_string());
    }
    // GTK writes modifiers as `<Ctrl>` immediately followed by the keyval, with no
    // separator between them.
    parts.push(gtk_key(shortcut.key()));
    parts.concat()
}

/// Accelerator string parsed from user input, for the preferences window.
pub fn accelerator_for(action_label: &str) -> Option<String> {
    None.or_else(|| {
        use std::str::FromStr;
        Shortcut::from_str(action_label)
            .ok()
            .map(|shortcut| accelerator(&shortcut))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn accel(text: &str) -> String {
        accelerator(&Shortcut::from_str(text).expect("valid shortcut"))
    }

    #[test]
    fn single_letters_are_lowercased_and_named_keys_are_kept() {
        assert_eq!(accel("N"), "n");
        assert_eq!(accel("f"), "f");
        assert_eq!(accel("F5"), "F5");
        assert_eq!(accel("F12"), "F12");
        assert_eq!(accel("Up"), "Up");
        assert_eq!(accel("Down"), "Down");
        assert_eq!(accel("Delete"), "Delete");
        assert_eq!(accel("Page_Up"), "Page_Up");
        assert_eq!(accel("Escape"), "Escape");
    }

    #[test]
    fn named_keys_use_the_gdk_spelling() {
        assert_eq!(accel("Space"), "space");
        assert_eq!(accel("["), "bracketleft");
        assert_eq!(accel("]"), "bracketright");
        assert_eq!(accel("plus"), "plus");
        assert_eq!(accel("minus"), "minus");
        assert_eq!(accel(","), "comma");
        assert_eq!(accel("."), "period");
        assert_eq!(accel("/"), "slash");
        assert_eq!(accel("Ctrl+,"), "<Ctrl>comma");
        assert_eq!(accel("Ctrl+Space"), "<Ctrl>space");
    }

    #[test]
    fn modifiers_come_first_and_in_a_stable_order() {
        assert_eq!(accel("Ctrl+Up"), "<Ctrl>Up");
        assert_eq!(accel("Ctrl+Down"), "<Ctrl>Down");
        assert_eq!(accel("Ctrl+Shift+R"), "<Ctrl><Shift>r");
        assert_eq!(accel("Alt+Ctrl+S"), "<Ctrl><Alt>s");
    }

    #[test]
    fn every_modifier_is_rendered() {
        let mut modifiers = Modifiers::default();
        modifiers.insert(Modifiers::CONTROL);
        modifiers.insert(Modifiers::ALT);
        modifiers.insert(Modifiers::SHIFT);
        modifiers.insert(Modifiers::SUPER);
        let shortcut = Shortcut::new(modifiers, "q").expect("valid shortcut");
        assert_eq!(accelerator(&shortcut), "<Ctrl><Alt><Shift><Super>q");
    }

    #[test]
    fn a_shortcut_round_trips_through_text() {
        let shortcut = Shortcut::from_str("Ctrl+Up").expect("valid shortcut");
        let text = shortcut.to_label();
        assert_eq!(text, "Ctrl+Up");
        assert_eq!(accel(&text), "<Ctrl>Up");
    }

    #[test]
    fn invalid_text_has_no_accelerator() {
        assert_eq!(accelerator_for("Ctrl+"), None);
        assert_eq!(accelerator_for("Ctrl+Up").as_deref(), Some("<Ctrl>Up"));
    }
}
