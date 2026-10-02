//! The navigation sidebar: sections, playlists, and a library summary.

/// Called with a playlist id when the user right-clicks one.
type PlaylistMenuCallback = Rc<RefCell<Option<Box<dyn Fn(i64)>>>>;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Align, ListBox, Orientation};

use crate::app::navigation::{Section, View};
use crate::app::ui::widgets;
use crate::playlists::model::Playlist;

/// Sections, playlists and a summary line, in that order.
pub struct Sidebar {
    root: gtk::ScrolledWindow,
    sections: ListBox,
    playlists: ListBox,
    heading: gtk::Label,
    summary: gtk::Label,
    /// Sections in row order; `ListBox` rows do not carry their section themselves.
    section_rows: Vec<Section>,
    /// Playlist ids in row order, so an activated row can be mapped back.
    playlist_ids: Rc<RefCell<Vec<i64>>>,
    on_playlist_menu: PlaylistMenuCallback,
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}

impl Sidebar {
    pub fn new() -> Self {
        let sections = ListBox::new();
        sections.set_selection_mode(gtk::SelectionMode::Single);

        let heading = widgets::text_label("Playlists", Align::Start);
        heading.add_css_class("heading");
        heading.set_margin_top(12);
        heading.set_margin_start(8);

        let playlists = ListBox::new();
        playlists.set_selection_mode(gtk::SelectionMode::Single);

        let summary = widgets::hint_label("");
        summary.set_margin_start(8);
        summary.set_margin_end(8);
        summary.set_margin_top(12);
        summary.set_margin_bottom(8);
        summary.set_xalign(0.0);
        summary.set_yalign(0.0);
        summary.set_wrap(true);

        let stack = gtk::Box::new(Orientation::Vertical, 0);
        stack.set_margin_top(8);
        let mut section_rows = Vec::with_capacity(Section::ORDER.len());
        for section in Section::ORDER {
            let row = section_row(section);
            sections.append(&row);
            section_rows.push(section);
        }
        stack.append(&sections);
        stack.append(&heading);
        stack.append(&playlists);
        stack.append(&summary);

        let root = gtk::ScrolledWindow::builder()
            .hexpand(false)
            .vexpand(true)
            .child(&stack)
            .build();
        root.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        root.set_size_request(180, -1);

        Self {
            root,
            sections,
            playlists,
            heading,
            summary,
            section_rows,
            playlist_ids: Rc::new(RefCell::new(Vec::new())),
            on_playlist_menu: Rc::new(RefCell::new(None)),
        }
    }

    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.root
    }

    /// Rebuild the playlist list.
    pub fn set_playlists(&self, playlists: &[Playlist]) {
        while let Some(row) = self.playlists.first_child() {
            self.playlists.remove(&row);
        }
        self.heading.set_visible(!playlists.is_empty());
        let mut ids = Vec::with_capacity(playlists.len());
        for playlist in playlists {
            let row = playlist_row(playlist);
            row.set_activatable(true);
            self.playlists.append(&row);
            ids.push(playlist.id);
        }
        *self.playlist_ids.borrow_mut() = ids;
    }

    /// Show what the library holds, for example `12 albums · 340 tracks`.
    pub fn set_summary(&self, summary: String) {
        self.summary.set_text(&summary);
    }

    /// Highlight the row for the current view.
    pub fn select(&self, view: &View) {
        let section = view.section();
        if let Some(row) = self.section_row(section) {
            self.sections.select_row(Some(&row));
        } else {
            self.sections.unselect_all();
        }
        let playlist_row = match view {
            View::Playlist(id) => self
                .playlist_ids
                .borrow()
                .iter()
                .position(|candidate| candidate == id)
                .and_then(|index| row_at(&self.playlists, index)),
            _ => None,
        };
        if let Some(row) = playlist_row {
            self.playlists.select_row(Some(&row));
        } else {
            self.playlists.unselect_all();
        }
    }

    pub fn connect_section_activate<F: Fn(Section) + 'static>(&self, callback: F) {
        let callback = Rc::new(callback);
        let sections = self.section_rows.clone();
        self.sections.connect_row_activated(move |list, row| {
            let index = row.index();
            if index < 0 {
                return;
            }
            if let Some(section) = sections.get(index as usize) {
                let _ = list;
                callback(*section);
            }
        });
    }

    pub fn connect_playlist_activate<F: Fn(i64) + 'static>(&self, callback: F) {
        let callback = Rc::new(callback);
        let ids = Rc::clone(&self.playlist_ids);
        self.playlists.connect_row_activated(move |_, row| {
            let index = row.index();
            if index < 0 {
                return;
            }
            if let Some(id) = ids.borrow().get(index as usize).copied() {
                callback(id);
            }
        });
    }

    /// Right-clicking a playlist asks the window to offer rename and delete.
    pub fn connect_playlist_menu<F: Fn(i64) + 'static>(&self, callback: F) {
        *self.on_playlist_menu.borrow_mut() = Some(Box::new(callback));
        let gesture = gtk::GestureClick::new();
        gesture.set_button(3);
        let ids = Rc::clone(&self.playlist_ids);
        let menu = Rc::clone(&self.on_playlist_menu);
        gesture.connect_pressed(move |gesture, _button, x, y| {
            let Some(list) = gesture.widget().and_downcast::<ListBox>() else {
                return;
            };
            let Some(row) = list.row_at_y(y.round() as i32) else {
                return;
            };
            let index = row.index();
            if index < 0 {
                return;
            }
            list.select_row(Some(&row));
            if let Some(id) = ids.borrow().get(index as usize).copied() {
                if let Some(callback) = menu.borrow().as_ref() {
                    callback(id);
                }
            }
            let _ = x;
        });
        self.playlists.add_controller(gesture);
    }

    /// The playlist the user last activated or clicked, if any.
    pub fn selected_playlist(&self) -> Option<i64> {
        let index = self.playlists.selected_row().map(|row| row.index())?;
        if index < 0 {
            return None;
        }
        self.playlist_ids.borrow().get(index as usize).copied()
    }

    /// The row widget for the selected playlist, so menus can anchor to it.
    pub fn selected_playlist_row(&self) -> Option<gtk::ListBoxRow> {
        let index = self.playlists.selected_row().map(|row| row.index())?;
        if index < 0 {
            return None;
        }
        row_at(&self.playlists, index as usize)
    }

    pub fn sections(&self) -> &ListBox {
        &self.sections
    }

    pub fn playlists(&self) -> &ListBox {
        &self.playlists
    }

    fn section_row(&self, section: Option<Section>) -> Option<gtk::ListBoxRow> {
        let index = self
            .section_rows
            .iter()
            .position(|candidate| Some(*candidate) == section)?;
        row_at(&self.sections, index)
    }
}

fn section_row(section: Section) -> gtk::ListBoxRow {
    let row = widgets::hstack(8);
    row.set_margin_top(4);
    row.set_margin_bottom(4);
    row.set_margin_start(8);
    row.set_margin_end(8);
    let icon = gtk::Image::from_icon_name(section_icon(section));
    icon.set_pixel_size(16);
    row.append(&icon);
    let label = widgets::text_label(section.label(), Align::Start);
    label.set_hexpand(true);
    row.append(&label);
    let number = widgets::hint_label(&section.shortcut_number().unwrap_or(0).to_string());
    number.set_width_chars(2);
    number.set_xalign(1.0);
    row.append(&number);

    gtk::ListBoxRow::builder()
        .activatable(true)
        .child(&row)
        .build()
}

fn playlist_row(playlist: &Playlist) -> gtk::ListBoxRow {
    let row = widgets::hstack(8);
    row.set_margin_top(4);
    row.set_margin_bottom(4);
    row.set_margin_start(8);
    row.set_margin_end(8);
    let icon = gtk::Image::from_icon_name("view-list-symbolic");
    icon.set_pixel_size(16);
    row.append(&icon);
    let label = widgets::text_label(&playlist.name, Align::Start);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_hexpand(true);
    row.append(&label);
    gtk::ListBoxRow::builder()
        .activatable(true)
        .child(&row)
        .build()
}

fn row_at(list: &ListBox, index: usize) -> Option<gtk::ListBoxRow> {
    list.row_at_index(i32::try_from(index).ok()?)
}

/// Icon name for a section.
pub fn section_icon(section: Section) -> &'static str {
    match section {
        Section::Library => "folder-music-symbolic",
        Section::Artists => "avatar-default-symbolic",
        Section::Albums => "media-optical-symbolic",
        Section::Folders => "folder-symbolic",
        Section::Favorites => "starred-symbolic",
        Section::Playlists => "view-list-symbolic",
        Section::Search => "system-search-symbolic",
        Section::Queue => "media-playlist-repeat-symbolic",
    }
}

/// The row a view belongs to, for tests and for the window title.
pub fn section_for(view: &View) -> Option<Section> {
    view.section()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn every_section_has_an_icon() {
        for section in Section::ORDER {
            assert!(!section_icon(section).is_empty());
        }
    }

    #[test]
    fn sections_map_to_a_row_position() {
        let section = Section::ORDER[3];
        let index = Section::ORDER
            .iter()
            .position(|candidate| *candidate == section)
            .unwrap();
        assert_eq!(section.shortcut_number(), Some(index as u8 + 1));
    }

    #[test]
    fn views_report_their_section() {
        assert_eq!(section_for(&View::Library), Some(Section::Library));
        assert_eq!(section_for(&View::Tracks), Some(Section::Library));
        assert_eq!(section_for(&View::Favorites), Some(Section::Favorites));
        assert_eq!(
            section_for(&View::Artist("Beta".into())),
            Some(Section::Artists)
        );
        assert_eq!(
            section_for(&View::Album("Artist".into(), "Record".into())),
            Some(Section::Albums)
        );
        assert_eq!(
            section_for(&View::Folder(PathBuf::from("/music"))),
            Some(Section::Folders)
        );
        assert_eq!(section_for(&View::Playlist(3)), Some(Section::Playlists));
        assert_eq!(
            section_for(&View::Search("hit".into())),
            Some(Section::Search)
        );
    }
}
