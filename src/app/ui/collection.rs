//! Collection views: artists, albums, folders, and playlists.
//!
//! The lists themselves come from [`ItemList`]; this module turns library data into rows and
//! rows back into navigation targets. That mapping is pure, so it is tested without a
//! display.

use std::rc::Rc;

use crate::app::navigation::View;
use crate::app::state::format_total;
use crate::app::ui::item_list::ItemLabel;
use crate::app::ui::item_list::ItemList;
use crate::library::model::{AlbumSummary, ArtistSummary, FolderSummary};
use crate::playlists::model::Playlist;

/// Separator inside album keys, which pair an album with its album artist.
const PAIR: char = '\u{1f}';

/// Rows for the artists view.
pub fn artist_items(artists: &[ArtistSummary]) -> Vec<ItemLabel> {
    artists
        .iter()
        .map(|artist| {
            let detail = format!(
                "{} · {}",
                plural(artist.track_count, "track"),
                plural(artist.album_count, "album")
            );
            ItemLabel::with_detail(artist_key(&artist.name), &artist.name, detail)
                .with_icon("avatar-default-symbolic")
        })
        .collect()
}

/// Rows for the albums view.
pub fn album_items(albums: &[AlbumSummary]) -> Vec<ItemLabel> {
    albums
        .iter()
        .map(|album| {
            let mut detail = album.display_album_artist().to_string();
            if let Some(year) = album.year {
                detail.push_str(&format!(" · {year}"));
            }
            detail.push_str(&format!(" · {}", plural(album.track_count, "track")));
            ItemLabel::with_detail(
                album_key(album.display_album_artist(), album.display_album()),
                album.display_album(),
                detail,
            )
            .with_icon("media-optical-symbolic")
        })
        .collect()
}

/// Rows for the folders view.
pub fn folder_items(folders: &[FolderSummary]) -> Vec<ItemLabel> {
    folders
        .iter()
        .map(|folder| {
            let name = folder
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| folder.path.to_string_lossy().into_owned());
            let detail = format!(
                "{} · {}",
                folder.path.to_string_lossy(),
                plural(folder.track_count, "track")
            );
            ItemLabel::with_detail(folder_key(&folder.path), name, detail)
                .with_icon("folder-symbolic")
        })
        .collect()
}

/// Rows for the playlists view.
pub fn playlist_items(playlists: &[Playlist]) -> Vec<ItemLabel> {
    playlists
        .iter()
        .map(|playlist| {
            ItemLabel::new(playlist_key(playlist.id), &playlist.name)
                .with_icon("view-list-symbolic")
        })
        .collect()
}

pub fn artist_key(name: &str) -> String {
    format!("artist:{name}")
}

pub fn album_key(album_artist: &str, album: &str) -> String {
    format!("album:{album_artist}{PAIR}{album}")
}

pub fn folder_key(path: &std::path::Path) -> String {
    format!("folder:{}", path.to_string_lossy())
}

pub fn playlist_key(id: i64) -> String {
    format!("playlist:{id}")
}

/// The view a row stands for.
pub fn target_view(key: &str) -> Option<View> {
    if let Some(name) = key.strip_prefix("artist:") {
        return Some(View::Artist(name.to_string()));
    }
    if let Some(pair) = key.strip_prefix("album:") {
        // An album artist may itself contain the separator, so split at the last one.
        let (album_artist, album) = pair.rsplit_once(PAIR)?;
        return Some(View::Album(album_artist.to_string(), album.to_string()));
    }
    if let Some(path) = key.strip_prefix("folder:") {
        return Some(View::Folder(path.into()));
    }
    key.strip_prefix("playlist:")
        .and_then(|id| id.parse().ok())
        .map(View::Playlist)
}

/// A collection list with the rows for whichever view is showing.
pub struct CollectionView {
    list: Rc<ItemList>,
}

impl CollectionView {
    pub fn new() -> Self {
        Self {
            list: Rc::new(ItemList::new()),
        }
    }

    pub fn widget(&self) -> &gtk::ScrolledWindow {
        self.list.widget()
    }

    pub fn items(&self) -> &[ItemLabel] {
        self.list.items()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Fill the list for a view. Views that show tracks leave it alone.
    pub fn set_view(
        &mut self,
        view: &View,
        artists: &[ArtistSummary],
        albums: &[AlbumSummary],
        folders: &[FolderSummary],
        playlists: &[Playlist],
    ) {
        let items = match view {
            View::Artists => artist_items(artists),
            View::Albums => album_items(albums),
            View::Folders => folder_items(folders),
            View::Playlists => playlist_items(playlists),
            _ => Vec::new(),
        };
        Rc::get_mut(&mut self.list)
            .expect("the collection list is not shared yet")
            .set_items(items);
    }

    pub fn connect_activate<F: Fn(View) + 'static>(&self, callback: F) {
        let list = Rc::clone(&self.list);
        self.list.connect_activate(move |index| {
            let Some(item) = list.item(index) else {
                return;
            };
            if let Some(view) = target_view(&item.key) {
                callback(view);
            }
        });
    }
}

impl Default for CollectionView {
    fn default() -> Self {
        Self::new()
    }
}

/// `1 track` / `2 tracks`, and so on.
pub fn plural(count: i64, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// Total running time of a set of albums, for the status line.
pub fn albums_total(albums: &[AlbumSummary]) -> String {
    let total: i64 = albums.iter().map(|album| album.duration_ms).sum();
    format_total(Some(total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn artist(name: &str, tracks: i64, albums: i64) -> ArtistSummary {
        ArtistSummary {
            name: name.to_string(),
            track_count: tracks,
            album_count: albums,
        }
    }

    fn album(album_artist: &str, album: &str, year: Option<i64>, tracks: i64) -> AlbumSummary {
        AlbumSummary {
            album_artist: album_artist.to_string(),
            album: album.to_string(),
            year,
            track_count: tracks,
            duration_ms: 3_000,
            artwork_reference: None,
            sample_track_id: 1,
            sample_track_path: PathBuf::from("/m/1.flac"),
        }
    }

    fn folder(id: i64, path: &str, tracks: i64) -> FolderSummary {
        FolderSummary {
            folder_id: id,
            path: PathBuf::from(path),
            track_count: tracks,
            created_at: 0,
        }
    }

    #[test]
    fn artist_rows_carry_counts() {
        let items = artist_items(&[artist("Beta", 1, 1), artist("Alpha", 12, 3)]);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Beta");
        assert_eq!(items[0].detail, "1 track · 1 album");
        assert_eq!(items[1].detail, "12 tracks · 3 albums");
        assert_eq!(items[0].key, "artist:Beta");
    }

    #[test]
    fn album_rows_carry_the_artist_and_year() {
        let items = album_items(&[album("Artist", "Record", Some(1999), 12)]);
        assert_eq!(items[0].title, "Record");
        assert_eq!(items[0].detail, "Artist · 1999 · 12 tracks");

        let undated = album_items(&[album("", "Loose", None, 3)]);
        assert_eq!(undated[0].title, "Loose");
        assert!(undated[0].detail.starts_with("Unknown Artist ·"));
    }

    #[test]
    fn folder_rows_show_the_name_and_the_path() {
        let items = folder_items(&[folder(1, "/music/albums", 12)]);
        assert_eq!(items[0].title, "albums");
        assert_eq!(items[0].detail, "/music/albums · 12 tracks");
        assert_eq!(items[0].key, "folder:/music/albums");
    }

    #[test]
    fn playlist_rows_use_the_playlist_id() {
        let playlists = vec![Playlist {
            id: 7,
            name: "Road trip".into(),
            created_at: 0,
            updated_at: 0,
        }];
        let items = playlist_items(&playlists);
        assert_eq!(items[0].title, "Road trip");
        assert_eq!(items[0].key, "playlist:7");
    }

    #[test]
    fn keys_map_back_to_views() {
        assert_eq!(
            target_view("artist:Beta"),
            Some(View::Artist("Beta".into()))
        );
        assert_eq!(
            target_view(&album_key("Artist", "Record")),
            Some(View::Album("Artist".into(), "Record".into()))
        );
        assert_eq!(
            target_view(&folder_key(std::path::Path::new("/music/x"))),
            Some(View::Folder(PathBuf::from("/music/x")))
        );
        assert_eq!(target_view("playlist:7"), Some(View::Playlist(7)));
        assert_eq!(target_view("playlist:seven"), None);
        assert_eq!(target_view("unknown:1"), None);
        assert_eq!(target_view("artist:"), Some(View::Artist(String::new())));
    }

    #[test]
    fn an_album_artist_containing_the_separator_survives() {
        let key = album_key("A\u{1f}B", "Record");
        assert_eq!(
            target_view(&key),
            Some(View::Album("A\u{1f}B".into(), "Record".into()))
        );
    }

    #[test]
    fn counts_are_pluralised() {
        assert_eq!(plural(0, "track"), "0 tracks");
        assert_eq!(plural(1, "track"), "1 track");
        assert_eq!(plural(2, "album"), "2 albums");
    }

    #[test]
    fn album_totals_are_formatted() {
        let albums = vec![album("A", "One", None, 1), album("A", "Two", None, 2)];
        assert_eq!(albums_total(&albums), "6 sec");
        assert_eq!(albums_total(&[]), "unknown length");
    }
}
