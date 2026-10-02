//! Application actions.
//!
//! Every user-visible operation in Strata is an [`Action`]. Buttons, menus, keyboard
//! shortcuts and media keys all produce the same action values, so behaviour is not
//! duplicated per input method. The GTK-facing dispatch lives in [`dispatch`] and is
//! implemented by the UI layer through [`ActionTarget`].

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    PlayPause,
    NextTrack,
    PreviousTrack,
    SeekForward,
    SeekBackward,
    VolumeUp,
    VolumeDown,
    Mute,
    ToggleShuffle,
    CycleRepeat,
    FocusSearch,
    AddToQueue,
    PlayNext,
    AddToPlaylist,
    ToggleFavorite,
    AddLibraryFolder,
    RescanLibrary,
    RescanFolder,
    OpenSettings,

    // Navigation and view commands.
    GoToArtists,
    GoToAlbums,
    GoToTracks,
    GoToFolders,
    GoToFavorites,
    GoToPlaylists,
    ToggleSidebar,
    SelectAll,

    // Track and playlist commands.
    ShowTrackInfo,
    ShowInFileManager,
    CopyPath,
    OpenTrackFile,
    ImportPlaylist,
    NewPlaylist,
    SavePlaylist,
    Quit,
}

impl Action {
    pub fn name(&self) -> &'static str {
        match self {
            Action::PlayPause => "play_pause",
            Action::NextTrack => "next_track",
            Action::PreviousTrack => "previous_track",
            Action::SeekForward => "seek_forward",
            Action::SeekBackward => "seek_backward",
            Action::VolumeUp => "volume_up",
            Action::VolumeDown => "volume_down",
            Action::Mute => "mute",
            Action::ToggleShuffle => "toggle_shuffle",
            Action::CycleRepeat => "cycle_repeat",
            Action::FocusSearch => "focus_search",
            Action::AddToQueue => "add_to_queue",
            Action::PlayNext => "play_next",
            Action::AddToPlaylist => "add_to_playlist",
            Action::ToggleFavorite => "toggle_favorite",
            Action::AddLibraryFolder => "add_library_folder",
            Action::RescanLibrary => "rescan_library",
            Action::RescanFolder => "rescan_folder",
            Action::OpenSettings => "open_settings",
            Action::GoToArtists => "go_to_artists",
            Action::GoToAlbums => "go_to_albums",
            Action::GoToTracks => "go_to_tracks",
            Action::GoToFolders => "go_to_folders",
            Action::GoToFavorites => "go_to_favorites",
            Action::GoToPlaylists => "go_to_playlists",
            Action::ToggleSidebar => "toggle_sidebar",
            Action::SelectAll => "select_all",
            Action::ShowTrackInfo => "show_track_info",
            Action::ShowInFileManager => "show_in_file_manager",
            Action::CopyPath => "copy_path",
            Action::OpenTrackFile => "open_track_file",
            Action::ImportPlaylist => "import_playlist",
            Action::NewPlaylist => "new_playlist",
            Action::SavePlaylist => "save_playlist",
            Action::Quit => "quit",
        }
    }

    /// Human readable label, used in menus, settings and tooltips.
    pub fn label(&self) -> &'static str {
        match self {
            Action::PlayPause => "Play / Pause",
            Action::NextTrack => "Next Track",
            Action::PreviousTrack => "Previous Track",
            Action::SeekForward => "Seek Forward",
            Action::SeekBackward => "Seek Backward",
            Action::VolumeUp => "Volume Up",
            Action::VolumeDown => "Volume Down",
            Action::Mute => "Mute",
            Action::ToggleShuffle => "Shuffle",
            Action::CycleRepeat => "Repeat",
            Action::FocusSearch => "Search",
            Action::AddToQueue => "Add to Queue",
            Action::PlayNext => "Play Next",
            Action::AddToPlaylist => "Add to Playlist",
            Action::ToggleFavorite => "Favourite",
            Action::AddLibraryFolder => "Add Music Folder",
            Action::RescanLibrary => "Rescan Library",
            Action::RescanFolder => "Rescan Folder",
            Action::OpenSettings => "Settings",
            Action::GoToArtists => "Artists",
            Action::GoToAlbums => "Albums",
            Action::GoToTracks => "Tracks",
            Action::GoToFolders => "Folders",
            Action::GoToFavorites => "Favourites",
            Action::GoToPlaylists => "Playlists",
            Action::ToggleSidebar => "Toggle Sidebar",
            Action::SelectAll => "Select All",
            Action::ShowTrackInfo => "Track Information",
            Action::ShowInFileManager => "Show in File Manager",
            Action::CopyPath => "Copy Path",
            Action::OpenTrackFile => "Open File",
            Action::ImportPlaylist => "Import Playlist",
            Action::NewPlaylist => "New Playlist",
            Action::SavePlaylist => "Save Playlist As",
            Action::Quit => "Quit",
        }
    }

    /// Actions that make sense only while at least one track is selected.
    pub fn needs_selection(&self) -> bool {
        matches!(
            self,
            Action::AddToQueue
                | Action::PlayNext
                | Action::AddToPlaylist
                | Action::ToggleFavorite
                | Action::ShowTrackInfo
                | Action::ShowInFileManager
                | Action::CopyPath
                | Action::OpenTrackFile
        )
    }

    pub fn parse(name: &str) -> Option<Action> {
        ALL_ACTIONS
            .iter()
            .copied()
            .find(|action| action.name() == name)
    }
}

pub const ALL_ACTIONS: &[Action] = &[
    Action::PlayPause,
    Action::NextTrack,
    Action::PreviousTrack,
    Action::SeekForward,
    Action::SeekBackward,
    Action::VolumeUp,
    Action::VolumeDown,
    Action::Mute,
    Action::ToggleShuffle,
    Action::CycleRepeat,
    Action::FocusSearch,
    Action::AddToQueue,
    Action::PlayNext,
    Action::AddToPlaylist,
    Action::ToggleFavorite,
    Action::AddLibraryFolder,
    Action::RescanLibrary,
    Action::RescanFolder,
    Action::OpenSettings,
    Action::GoToArtists,
    Action::GoToAlbums,
    Action::GoToTracks,
    Action::GoToFolders,
    Action::GoToFavorites,
    Action::GoToPlaylists,
    Action::ToggleSidebar,
    Action::SelectAll,
    Action::ShowTrackInfo,
    Action::ShowInFileManager,
    Action::CopyPath,
    Action::OpenTrackFile,
    Action::ImportPlaylist,
    Action::NewPlaylist,
    Action::SavePlaylist,
    Action::Quit,
];

/// Receiver for actions. Implemented by the GTK layer; keeps this module free of any
/// toolkit dependency so it can be tested and reused.
pub trait ActionTarget {
    fn perform(&mut self, action: Action);
}

/// Perform `action` on `target`.
pub fn dispatch<T: ActionTarget + ?Sized>(target: &mut T, action: Action) {
    target.perform(action);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn action_names_are_unique() {
        let mut names: Vec<&str> = ALL_ACTIONS.iter().map(|action| action.name()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate action name in ALL_ACTIONS");
    }

    #[test]
    fn every_action_round_trips_through_its_name() {
        for action in ALL_ACTIONS {
            assert_eq!(Action::parse(action.name()), Some(*action));
            let serialised = toml::Value::try_from(*action).unwrap();
            assert_eq!(serialised.as_str(), Some(action.name()));
            assert_eq!(toml::Value::try_from(action).unwrap(), serialised);
        }
    }

    #[test]
    fn all_actions_are_listed() {
        let listed: HashSet<Action> = ALL_ACTIONS.iter().copied().collect();
        let parsed: HashSet<Action> = [
            "play_pause",
            "next_track",
            "previous_track",
            "seek_forward",
            "seek_backward",
            "volume_up",
            "volume_down",
            "mute",
            "toggle_shuffle",
            "cycle_repeat",
            "focus_search",
            "add_to_queue",
            "play_next",
            "add_to_playlist",
            "toggle_favorite",
            "add_library_folder",
            "rescan_library",
            "rescan_folder",
            "open_settings",
            "go_to_artists",
            "go_to_albums",
            "go_to_tracks",
            "go_to_folders",
            "go_to_favorites",
            "go_to_playlists",
            "toggle_sidebar",
            "select_all",
            "show_track_info",
            "show_in_file_manager",
            "copy_path",
            "open_track_file",
            "import_playlist",
            "new_playlist",
            "save_playlist",
            "quit",
        ]
        .iter()
        .map(|name| Action::parse(name).expect("action name parses"))
        .collect();
        assert_eq!(listed, parsed);
    }

    #[test]
    fn unknown_action_name_is_rejected() {
        assert_eq!(Action::parse("teleport"), None);
    }

    #[test]
    fn selection_actions_are_flagged() {
        assert!(Action::ToggleFavorite.needs_selection());
        assert!(!Action::PlayPause.needs_selection());
    }
}
