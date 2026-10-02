//! Navigation: which view is showing, and how the user got there.
//!
//! Back and forward are implemented here rather than in the widgets, so the behaviour is
//! testable without a display.

use std::collections::VecDeque;

/// A screen in the app. Values are stored in the window state so the last view can be
/// restored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum View {
    #[default]
    Library,
    Artists,
    Artist(String),
    Albums,
    Album(String, String),
    Tracks,
    Folders,
    Folder(std::path::PathBuf),
    Favorites,
    Playlists,
    Playlist(i64),
    Search(String),
    Queue,
}

impl View {
    /// Top level entry in the sidebar, when this view belongs to one.
    pub fn section(&self) -> Option<Section> {
        Some(match self {
            View::Library | View::Tracks => Section::Library,
            View::Favorites => Section::Favorites,
            View::Artists | View::Artist(_) => Section::Artists,
            View::Albums | View::Album(..) => Section::Albums,
            View::Folders | View::Folder(_) => Section::Folders,
            View::Playlists | View::Playlist(_) => Section::Playlists,
            View::Search(_) => Section::Search,
            View::Queue => Section::Queue,
        })
    }

    pub fn title(&self) -> String {
        match self {
            View::Library => "Library".to_string(),
            View::Artists => "Artists".to_string(),
            View::Artist(name) => name.clone(),
            View::Albums => "Albums".to_string(),
            View::Album(album_artist, album) => {
                if album_artist.is_empty() || album_artist == album {
                    album.clone()
                } else {
                    format!("{album} — {album_artist}")
                }
            }
            View::Tracks => "All Tracks".to_string(),
            View::Folders => "Folders".to_string(),
            View::Folder(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
            View::Favorites => "Favorites".to_string(),
            View::Playlists => "Playlists".to_string(),
            View::Playlist(id) => format!("Playlist {id}"),
            View::Search(term) => format!("Search: {term}"),
            View::Queue => "Queue".to_string(),
        }
    }

    /// True for views that show a list of tracks.
    pub fn shows_tracks(&self) -> bool {
        matches!(
            self,
            View::Library
                | View::Tracks
                | View::Favorites
                | View::Album(..)
                | View::Artist(_)
                | View::Folder(_)
                | View::Playlist(_)
                | View::Search(_)
                | View::Queue
        )
    }

    /// Key used for persisted state, without library row ids.
    pub fn stable_key(&self) -> String {
        match self {
            View::Library => "library".to_string(),
            View::Tracks => "tracks".to_string(),
            View::Artists => "artists".to_string(),
            View::Albums => "albums".to_string(),
            View::Folders => "folders".to_string(),
            View::Favorites => "favorites".to_string(),
            View::Playlists => "playlists".to_string(),
            View::Queue => "queue".to_string(),
            View::Artist(name) => format!("artist:{name}"),
            View::Album(album_artist, album) => format!("album:{album_artist}\u{1f}{album}"),
            View::Folder(path) => format!("folder:{}", path.to_string_lossy()),
            View::Playlist(id) => format!("playlist:{id}"),
            View::Search(term) => format!("search:{term}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Library,
    Artists,
    Albums,
    Folders,
    Favorites,
    Playlists,
    Search,
    Queue,
}

impl Section {
    pub const ORDER: [Section; 8] = [
        Section::Library,
        Section::Artists,
        Section::Albums,
        Section::Folders,
        Section::Favorites,
        Section::Playlists,
        Section::Queue,
        Section::Search,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Section::Library => "Library",
            Section::Artists => "Artists",
            Section::Albums => "Albums",
            Section::Folders => "Folders",
            Section::Favorites => "Favorites",
            Section::Playlists => "Playlists",
            Section::Queue => "Queue",
            Section::Search => "Search",
        }
    }

    /// Keyboard number that jumps to this section, 1 to 8.
    pub fn shortcut_number(&self) -> Option<u8> {
        Section::ORDER
            .iter()
            .position(|section| section == self)
            .map(|index| index as u8 + 1)
    }

    pub fn view(&self) -> View {
        match self {
            Section::Library => View::Library,
            Section::Artists => View::Artists,
            Section::Albums => View::Albums,
            Section::Folders => View::Folders,
            Section::Favorites => View::Favorites,
            Section::Playlists => View::Playlists,
            Section::Queue => View::Queue,
            Section::Search => View::Search(String::new()),
        }
    }
}

/// Back/forward history with a bounded number of entries.
#[derive(Debug, Clone, Default)]
pub struct Navigation {
    current: View,
    back: VecDeque<View>,
    forward: VecDeque<View>,
    limit: usize,
}

impl Navigation {
    pub fn new(start: View) -> Self {
        Self {
            current: start,
            back: VecDeque::new(),
            forward: VecDeque::new(),
            limit: 50,
        }
    }

    pub fn current(&self) -> &View {
        &self.current
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Move to a new view, recording the previous one.
    pub fn go_to(&mut self, view: View) {
        if view == self.current {
            return;
        }
        self.back.push_back(self.current.clone());
        if self.back.len() > self.limit {
            self.back.pop_front();
        }
        self.forward.clear();
        self.current = view;
    }

    /// Re-selecting the section the user is already in returns to its root view.
    pub fn go_to_section(&mut self, section: Section) {
        self.go_to(section.view());
    }

    pub fn back(&mut self) -> Option<&View> {
        let previous = self.back.pop_back()?;
        self.forward.push_back(self.current.clone());
        self.current = previous.clone();
        Some(&self.current)
    }

    pub fn forward(&mut self) -> Option<&View> {
        let next = self.forward.pop_back()?;
        self.back.push_back(self.current.clone());
        self.current = next.clone();
        Some(&self.current)
    }

    /// History depth, for the window title and tests.
    pub fn depth(&self) -> usize {
        self.back.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn navigation() -> Navigation {
        Navigation::new(View::Library)
    }

    #[test]
    fn the_first_view_does_not_create_history() {
        let mut navigation = navigation();
        navigation.go_to(View::Library);
        assert!(!navigation.can_go_back());
    }

    #[test]
    fn going_back_and_forward_returns_to_the_same_views() {
        let mut navigation = navigation();
        navigation.go_to(View::Albums);
        navigation.go_to(View::Album("Artist".into(), "Album".into()));

        assert_eq!(navigation.back(), Some(&View::Albums));
        assert_eq!(
            navigation.current(),
            &View::Albums,
            "back returned to albums"
        );
        assert_eq!(
            navigation.forward(),
            Some(&View::Album("Artist".into(), "Album".into()))
        );
        assert_eq!(
            navigation.current(),
            &View::Album("Artist".into(), "Album".into()),
            "forward returned to the album"
        );
    }

    #[test]
    fn back_at_the_start_is_a_no_op() {
        let mut navigation = navigation();
        assert_eq!(navigation.back(), None);
        assert_eq!(navigation.current(), &View::Library);
    }

    #[test]
    fn a_new_navigation_clears_the_forward_history() {
        let mut navigation = navigation();
        navigation.go_to(View::Albums);
        navigation.back();
        assert!(navigation.can_go_forward());
        navigation.go_to(View::Queue);
        assert!(!navigation.can_go_forward());
    }

    #[test]
    fn history_is_bounded() {
        let mut navigation = navigation();
        for index in 0..100 {
            navigation.go_to(View::Search(format!("term {index}")));
        }
        assert_eq!(navigation.depth(), 50);
    }

    #[test]
    fn sections_map_to_their_views() {
        assert_eq!(Section::Library.view(), View::Library);
        assert_eq!(Section::Queue.view(), View::Queue);
        assert_eq!(Section::Favorites.view(), View::Favorites);
    }

    #[test]
    fn every_view_belongs_to_a_section() {
        for section in Section::ORDER {
            assert_eq!(section.view().section(), Some(section));
        }
        assert_eq!(
            View::Album("a".into(), "b".into()).section(),
            Some(Section::Albums)
        );
        assert_eq!(
            View::Folder(PathBuf::from("/m")).section(),
            Some(Section::Folders)
        );
    }

    #[test]
    fn section_shortcuts_are_numbered_in_order() {
        assert_eq!(Section::Library.shortcut_number(), Some(1));
        assert_eq!(Section::Artists.shortcut_number(), Some(2));
        assert_eq!(Section::Albums.shortcut_number(), Some(3));
        assert_eq!(Section::Folders.shortcut_number(), Some(4));
        assert_eq!(Section::Favorites.shortcut_number(), Some(5));
        assert_eq!(Section::Playlists.shortcut_number(), Some(6));
        assert_eq!(Section::Queue.shortcut_number(), Some(7));
        assert_eq!(Section::Search.shortcut_number(), Some(8));
    }

    #[test]
    fn titles_are_human_readable() {
        assert_eq!(View::Library.title(), "Library");
        assert_eq!(
            View::Album("Pink Floyd".into(), "Animals".into()).title(),
            "Animals — Pink Floyd"
        );
        assert_eq!(View::Album("Same".into(), "Same".into()).title(), "Same");
        assert_eq!(View::Folder(PathBuf::from("/m/Music/Rock")).title(), "Rock");
        assert_eq!(View::Search("blue".into()).title(), "Search: blue");
    }

    #[test]
    fn track_views_are_recognised() {
        assert!(View::Album("a".into(), "b".into()).shows_tracks());
        assert!(View::Queue.shows_tracks());
        assert!(!View::Albums.shows_tracks());
        assert!(!View::Folders.shows_tracks());
    }

    #[test]
    fn stable_keys_are_distinct() {
        let views = vec![
            View::Library,
            View::Tracks,
            View::Artists,
            View::Artist("X".into()),
            View::Albums,
            View::Album("A".into(), "B".into()),
            View::Folders,
            View::Folder(PathBuf::from("/m/a")),
            View::Favorites,
            View::Playlists,
            View::Playlist(3),
            View::Search("q".into()),
            View::Queue,
        ];
        let mut keys: Vec<String> = views.iter().map(View::stable_key).collect();
        let count = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), count);
    }

    #[test]
    fn re_selecting_a_section_goes_to_its_root() {
        let mut navigation = Navigation::new(View::Album("a".into(), "b".into()));
        navigation.go_to_section(Section::Albums);
        assert_eq!(navigation.current(), &View::Albums);
    }
}
