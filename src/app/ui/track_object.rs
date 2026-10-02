//! GObject row for the track list.
//!
//! One instance per row. Properties mirror [`TrackRow`] so the list view's factory and
//! any sorter can bind to them.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::app::state::{format_duration, TrackRow};

mod imp {
    use super::*;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::TrackObject)]
    pub struct TrackObject {
        #[property(get, set)]
        pub track_id: Cell<i64>,
        #[property(get, set)]
        pub number: RefCell<String>,
        #[property(get, set)]
        pub title: RefCell<String>,
        #[property(get, set)]
        pub artist: RefCell<String>,
        #[property(get, set)]
        pub album: RefCell<String>,
        #[property(get, set)]
        pub duration: RefCell<String>,
        /// Duration in milliseconds; zero when unknown.
        #[property(get, set)]
        pub duration_ms: Cell<i64>,
        #[property(get, set)]
        pub filename: RefCell<String>,
        #[property(get, set)]
        pub path: RefCell<String>,
        #[property(get, set)]
        pub favorite: Cell<bool>,
        #[property(get, set)]
        pub missing: Cell<bool>,
        /// Cached artwork to show instead of the placeholder icon.
        #[property(get, set, nullable)]
        pub artwork_path: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TrackObject {
        const NAME: &'static str = "StrataTrackObject";
        type Type = super::TrackObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for TrackObject {}
}

glib::wrapper! {
    pub struct TrackObject(ObjectSubclass<imp::TrackObject>);
}

impl TrackObject {
    pub fn new(row: &TrackRow) -> Self {
        let object: Self = glib::Object::new();
        object.set_track_id(row.track_id);
        object.set_number(row.track_number_label.clone());
        object.set_title(row.title.clone());
        object.set_artist(row.artist.clone());
        object.set_album(row.album.clone());
        object.set_duration(format_duration(row.duration_ms));
        object.set_duration_ms(row.duration_ms.unwrap_or_default());
        object.set_filename(row.filename.clone());
        object.set_path(row.path.to_string_lossy().into_owned());
        object.set_favorite(row.favorite);
        object.set_missing(row.missing);
        object
    }

    pub fn row(&self) -> TrackRow {
        TrackRow {
            track_id: self.track_id(),
            title: self.title(),
            artist: self.artist(),
            album: self.album(),
            duration_ms: Some(self.duration_ms()).filter(|ms| *ms > 0),
            filename: self.filename(),
            path: self.path().into(),
            favorite: self.favorite(),
            missing: self.missing(),
            track_number_label: self.number(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn row() -> TrackRow {
        TrackRow {
            track_id: 7,
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            duration_ms: Some(125_000),
            filename: "song.mp3".to_string(),
            path: PathBuf::from("/m/song.mp3"),
            favorite: true,
            missing: false,
            track_number_label: "1-2".to_string(),
        }
    }

    #[test]
    fn a_row_becomes_a_track_object() {
        let object = TrackObject::new(&row());
        assert_eq!(object.track_id(), 7);
        assert_eq!(object.title(), "Song");
        assert_eq!(object.artist(), "Artist");
        assert_eq!(object.album(), "Album");
        assert_eq!(object.duration(), "2:05");
        assert_eq!(object.number(), "1-2");
        assert_eq!(object.filename(), "song.mp3");
        assert!(object.favorite());
        assert!(!object.missing());
        assert_eq!(object.path(), "/m/song.mp3");
        assert_eq!(object.duration_ms(), 125_000);
    }

    #[test]
    fn a_row_round_trips_through_an_object() {
        assert_eq!(TrackObject::new(&row()).row(), row());
    }

    #[test]
    fn an_unknown_duration_is_shown_as_placeholders() {
        let mut row = row();
        row.duration_ms = None;
        row.missing = true;
        row.track_number_label = String::new();
        let object = TrackObject::new(&row);
        assert_eq!(object.duration(), "--:--");
        assert!(object.missing());
        assert_eq!(object.number(), "");
        assert_eq!(object.row().duration_ms, None);
    }

    #[test]
    fn artwork_is_null_until_it_is_loaded() {
        let object = TrackObject::new(&row());
        assert_eq!(object.artwork_path(), None);
        object.set_artwork_path(Some("/cache/art.png".to_string()));
        assert_eq!(object.artwork_path().as_deref(), Some("/cache/art.png"));
    }
}
