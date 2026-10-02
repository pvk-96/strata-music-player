//! Keyboard configuration: which actions have which shortcuts.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::app::actions::{Action, ALL_ACTIONS};
use crate::settings::shortcut::Shortcut;

/// Default shortcuts.
///
/// Playback follows the specified defaults. Volume uses `Ctrl+Up` / `Ctrl+Down` instead of
/// bare arrows so that list navigation keeps working in every view (see `docs/keyboard.md`).
pub fn default_bindings() -> BTreeMap<Action, Shortcut> {
    let pairs: &[(Action, &str)] = &[
        (Action::PlayPause, "Space"),
        (Action::NextTrack, "N"),
        (Action::PreviousTrack, "P"),
        (Action::SeekForward, "]"),
        (Action::SeekBackward, "["),
        (Action::VolumeUp, "Ctrl+Up"),
        (Action::VolumeDown, "Ctrl+Down"),
        (Action::Mute, "M"),
        (Action::ToggleShuffle, "S"),
        (Action::CycleRepeat, "R"),
        (Action::ToggleFavorite, "F"),
        (Action::FocusSearch, "Ctrl+K"),
        (Action::SelectAll, "Ctrl+A"),
        (Action::AddLibraryFolder, "Ctrl+O"),
        (Action::RescanLibrary, "Ctrl+R"),
        (Action::RescanFolder, "Ctrl+Shift+R"),
        (Action::ImportPlaylist, "Ctrl+Shift+I"),
        (Action::NewPlaylist, "Ctrl+N"),
        (Action::OpenSettings, "Ctrl+,"),
        (Action::ToggleSidebar, "Ctrl+B"),
        (Action::GoToTracks, "Ctrl+1"),
        (Action::GoToAlbums, "Ctrl+2"),
        (Action::GoToArtists, "Ctrl+3"),
        (Action::GoToFolders, "Ctrl+4"),
        (Action::GoToFavorites, "Ctrl+5"),
        (Action::GoToPlaylists, "Ctrl+6"),
        (Action::ShowTrackInfo, "Ctrl+I"),
        (Action::CopyPath, "Ctrl+Shift+C"),
        (Action::ShowInFileManager, "Ctrl+Shift+E"),
        (Action::Quit, "Ctrl+Q"),
    ];
    let pairs: Vec<(Action, &str)> = pairs.to_vec();

    pairs
        .iter()
        .filter_map(|(action, text)| {
            Shortcut::from_str(text)
                .map(|shortcut| (*action, shortcut))
                .ok()
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyboardSettings {
    #[serde(default = "default_bindings", with = "bindings_table")]
    bindings: BTreeMap<Action, Shortcut>,
}

/// Serialise bindings as a flat table of `action_name = "shortcut"` pairs, which keeps
/// the settings file readable and editable by hand.
mod bindings_table {
    use super::BTreeMap;
    use crate::app::actions::Action;
    use crate::settings::shortcut::Shortcut;
    use serde::de::{Deserialize, Deserializer};
    use serde::ser::{SerializeMap, Serializer};

    type Names = BTreeMap<String, String>;

    pub fn serialize<S: Serializer>(
        bindings: &BTreeMap<Action, Shortcut>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        let mut pairs: Vec<(String, String)> = bindings
            .iter()
            .map(|(action, shortcut)| (action.name().to_string(), shortcut.to_label()))
            .collect();
        pairs.sort();
        let mut map = s.serialize_map(Some(pairs.len()))?;
        for (name, label) in pairs {
            map.serialize_entry(&name, &label)?;
        }
        map.end()
    }

    /// Bindings are merged over the defaults, so editing one entry in the file does not
    /// silently drop every other shortcut. An empty string unbinds the action.
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<Action, Shortcut>, D::Error> {
        let raw = Names::deserialize(deserializer)?;
        let mut bindings = crate::settings::keyboard::default_bindings();
        for (name, label) in raw {
            let action = match Action::parse(&name) {
                Some(action) => action,
                None => {
                    log::warn!("ignoring keyboard binding for unknown action '{name}'");
                    continue;
                }
            };
            let trimmed = label.trim();
            if trimmed.is_empty() {
                bindings.remove(&action);
                continue;
            }
            match trimmed.parse::<Shortcut>() {
                Ok(shortcut) => {
                    bindings.insert(action, shortcut);
                }
                Err(err) => {
                    log::warn!("ignoring shortcut for '{name}': {err}");
                }
            }
        }
        Ok(bindings)
    }
}

/// Two actions bound to the same shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub action: Action,
    pub clashes_with: Action,
    pub shortcut: Shortcut,
}

impl Conflict {
    pub fn message(&self) -> String {
        format!(
            "{} and {} use the same shortcut {}",
            self.action.label(),
            self.clashes_with.label(),
            self.shortcut.to_label()
        )
    }
}

impl KeyboardSettings {
    pub fn new(bindings: BTreeMap<Action, Shortcut>) -> Self {
        Self { bindings }
    }

    pub fn defaults() -> Self {
        Self {
            bindings: default_bindings(),
        }
    }

    pub fn binding(&self, action: Action) -> Option<&Shortcut> {
        self.bindings.get(&action)
    }

    /// All actions in display order with their current binding (may be unbound).
    pub fn display_rows(&self) -> Vec<(Action, Option<String>)> {
        ALL_ACTIONS
            .iter()
            .map(|action| (*action, self.binding(*action).map(|s| s.to_label())))
            .collect()
    }

    /// Assign a shortcut. `None` clears the binding for the action.
    pub fn set_binding(&mut self, action: Action, shortcut: Option<Shortcut>) {
        match shortcut {
            Some(shortcut) => {
                self.bindings.insert(action, shortcut);
            }
            None => {
                self.bindings.remove(&action);
            }
        }
    }

    /// Find the action bound to `shortcut`, ignoring unbound actions.
    pub fn action_for(&self, shortcut: &Shortcut) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(_, bound)| *bound == shortcut)
            .map(|(action, _)| *action)
    }

    /// Actions that would become unreachable, i.e. bound shortcuts used twice.
    pub fn conflicts(&self) -> Vec<Conflict> {
        let mut seen: BTreeMap<&Shortcut, Action> = BTreeMap::new();
        let mut conflicts = Vec::new();
        for (action, shortcut) in &self.bindings {
            match seen.get(shortcut) {
                Some(first) => conflicts.push(Conflict {
                    action: *action,
                    clashes_with: *first,
                    shortcut: shortcut.clone(),
                }),
                None => {
                    seen.insert(shortcut, *action);
                }
            }
        }
        conflicts
    }

    pub fn reset_defaults(&mut self) {
        self.bindings = default_bindings();
    }
}

impl Default for KeyboardSettings {
    fn default() -> Self {
        Self::defaults()
    }
}

impl fmt::Display for KeyboardSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (action, shortcut) in self.display_rows() {
            match shortcut {
                Some(label) => writeln!(f, "{:<22} {label}", action.name())?,
                None => writeln!(f, "{:<22} --", action.name())?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::shortcut::Modifiers;

    fn shortcut(text: &str) -> Shortcut {
        Shortcut::from_str(text).unwrap()
    }

    #[test]
    fn defaults_are_unique() {
        let settings = KeyboardSettings::defaults();
        assert!(settings.conflicts().is_empty(), "default bindings conflict");
    }

    #[test]
    fn defaults_match_the_specification() {
        let settings = KeyboardSettings::defaults();
        let expected = [
            (Action::PlayPause, "Space"),
            (Action::NextTrack, "N"),
            (Action::PreviousTrack, "P"),
            (Action::SeekForward, "]"),
            (Action::SeekBackward, "["),
            (Action::Mute, "M"),
            (Action::ToggleShuffle, "S"),
            (Action::CycleRepeat, "R"),
            (Action::ToggleFavorite, "F"),
            (Action::FocusSearch, "Ctrl+K"),
            (Action::SelectAll, "Ctrl+A"),
        ];
        for (action, label) in expected {
            assert_eq!(
                settings.binding(action).map(|s| s.to_label()).as_deref(),
                Some(label),
                "unexpected default for {}",
                action.name()
            );
        }
    }

    #[test]
    fn arrow_keys_are_not_stolen_from_lists() {
        let settings = KeyboardSettings::defaults();
        for reserved in ["Up", "Down", "Left", "Right"] {
            let bound = reserved.parse::<Shortcut>().unwrap();
            assert!(
                !settings
                    .bindings
                    .values()
                    .any(|b| b.key() == bound.key() && b.modifiers().empty()),
                "bare {reserved} must not be bound"
            );
        }
    }

    #[test]
    fn volume_uses_control_arrows_by_default() {
        let settings = KeyboardSettings::defaults();
        assert!(settings
            .binding(Action::VolumeUp)
            .unwrap()
            .modifiers()
            .contains(Modifiers::CONTROL));
    }

    #[test]
    fn conflicts_are_detected() {
        let mut settings = KeyboardSettings::defaults();
        settings.set_binding(Action::ToggleFavorite, Some(shortcut("Ctrl+N")));
        let conflicts = settings.conflicts();
        assert_eq!(conflicts.len(), 1);
        // Reported once, naming the two actions involved.
        assert_eq!(conflicts[0].action, Action::NewPlaylist);
        assert_eq!(conflicts[0].clashes_with, Action::ToggleFavorite);
        assert!(conflicts[0].message().contains("Ctrl+N"));
    }

    #[test]
    fn clearing_a_binding_works() {
        let mut settings = KeyboardSettings::defaults();
        settings.set_binding(Action::ToggleFavorite, None);
        assert!(settings.binding(Action::ToggleFavorite).is_none());
        assert!(settings.action_for(&shortcut("F")).is_none());
    }

    #[test]
    fn lookup_by_shortcut_round_trips() {
        let settings = KeyboardSettings::defaults();
        assert_eq!(
            settings.action_for(&shortcut("Ctrl+K")),
            Some(Action::FocusSearch)
        );
        assert_eq!(settings.action_for(&shortcut("Ctrl+9")), None);
    }

    #[test]
    fn reset_restores_defaults() {
        let mut settings = KeyboardSettings::defaults();
        settings.set_binding(Action::PlayPause, Some(shortcut("Z")));
        settings.reset_defaults();
        assert_eq!(
            settings.binding(Action::PlayPause),
            Some(&shortcut("Space"))
        );
    }

    #[test]
    fn every_action_appears_in_display_rows() {
        let settings = KeyboardSettings::defaults();
        assert_eq!(settings.display_rows().len(), ALL_ACTIONS.len());
    }
}
