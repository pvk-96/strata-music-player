//! The track list.
//!
//! A `ListView` with a header row, artwork thumbnails loaded in the background, and
//! drag-and-drop reordering for the queue and playlists. All behaviour comes from
//! [`AppCore`]; this module only keeps the widgets in sync with it.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Image, Label, ListView, Orientation, ScrolledWindow};

use crate::app::core::AppCore;
use crate::app::state::TrackRow;
use crate::app::ui::track_object::TrackObject;
use crate::app::ui::widgets;
use crate::app::ui::widgets::{hint_label, text_label};
use crate::artwork::loader::ArtworkResult;

/// Height of one row, used to turn a drop position into a row index.
pub const ROW_HEIGHT: i32 = 44;
pub const THUMBNAIL_SIZE: u32 = 40;
/// How many thumbnails are requested when a list is filled.
pub const ARTWORK_PREFETCH: usize = 120;

type ActivateCallback = Box<dyn Fn(i64)>;
type ActivateSelectionCallback = Box<dyn Fn(Vec<i64>)>;
type ReorderCallback = Box<dyn Fn(usize, usize)>;
type SelectionCallback = Box<dyn Fn(Vec<usize>)>;

pub struct TrackList {
    root: ScrolledWindow,
    list: ListView,
    header: GtkBox,
    store: gio::ListStore,
    selection: gtk::SingleSelection,
    rows: Rc<RefCell<Vec<TrackObject>>>,
    selected: Rc<RefCell<Vec<usize>>>,
    drag_origin: Rc<Cell<i64>>,
    on_activate: Rc<RefCell<Option<ActivateCallback>>>,
    on_activate_selection: Rc<RefCell<Option<ActivateSelectionCallback>>>,
    on_reordered: Rc<RefCell<Option<ReorderCallback>>>,
    on_selection_changed: Rc<RefCell<Option<SelectionCallback>>>,
}

impl TrackList {
    pub fn new() -> Self {
        let store = gio::ListStore::new::<TrackObject>();
        let rows = Rc::new(RefCell::new(Vec::new()));
        let selected = Rc::new(RefCell::new(Vec::new()));
        let drag_origin = Rc::new(Cell::new(-1i64));

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                item.set_child(Some(&build_row()));
            }
        });
        factory.connect_bind(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(object) = item.item().and_downcast::<TrackObject>() else {
                return;
            };
            let Some(row) = item.child().and_downcast::<GtkBox>() else {
                return;
            };
            bind_row(&row, &object);
        });

        let selection_model = gtk::SingleSelection::new(Some(store.clone()));
        selection_model.set_autoselect(false);
        selection_model.set_can_unselect(true);

        let list = ListView::new(Some(selection_model.clone()), Some(factory));
        list.set_single_click_activate(false);
        list.set_tab_behavior(gtk::ListTabBehavior::Item);

        let header = build_header();
        let scroller = ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&list)
            .build();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_has_frame(false);

        let track_list = Self {
            root: scroller,
            list,
            header,
            store,
            selection: selection_model,
            rows,
            selected,
            drag_origin,
            on_activate: Rc::new(RefCell::new(None)),
            on_activate_selection: Rc::new(RefCell::new(None)),
            on_reordered: Rc::new(RefCell::new(None)),
            on_selection_changed: Rc::new(RefCell::new(None)),
        };
        track_list.setup_signals();
        track_list.setup_drag_and_drop();
        track_list
    }

    pub fn widget(&self) -> &ScrolledWindow {
        &self.root
    }

    pub fn header(&self) -> &GtkBox {
        &self.header
    }

    pub fn list(&self) -> &ListView {
        &self.list
    }

    pub fn len(&self) -> usize {
        self.rows.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn row_at(&self, index: usize) -> Option<TrackObject> {
        self.rows.borrow().get(index).cloned()
    }

    pub fn selected_indexes(&self) -> Vec<usize> {
        self.selected.borrow().clone()
    }

    pub fn selected_track_ids(&self) -> Vec<i64> {
        self.selected_indexes()
            .iter()
            .filter_map(|index| self.row_at(*index))
            .map(|object| object.track_id())
            .collect()
    }

    /// Replace the model with new rows.
    pub fn set_rows(&mut self, rows: &[TrackRow]) {
        self.store.remove_all();
        let mut objects = Vec::with_capacity(rows.len());
        for row in rows {
            let object = TrackObject::new(row);
            self.store.append(&object);
            objects.push(object);
        }
        *self.rows.borrow_mut() = objects;
        self.selected.borrow_mut().clear();
        self.selection.set_selected(gtk::INVALID_LIST_POSITION);
    }

    pub fn set_selected(&self, indexes: &[usize]) {
        let indexes = normalize_selection(indexes);
        *self.selected.borrow_mut() = indexes.clone();
        self.selection.set_selected(
            indexes
                .first()
                .map_or(gtk::INVALID_LIST_POSITION, |index| *index as u32),
        );
    }

    pub fn select_row(&self, index: usize) {
        self.set_selected(&[index]);
    }

    pub fn connect_activate<F: Fn(i64) + 'static>(&self, callback: F) {
        *self.on_activate.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_activate_selection<F: Fn(Vec<i64>) + 'static>(&self, callback: F) {
        *self.on_activate_selection.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_reordered<F: Fn(usize, usize) + 'static>(&self, callback: F) {
        *self.on_reordered.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_selection_changed<F: Fn(Vec<usize>) + 'static>(&self, callback: F) {
        *self.on_selection_changed.borrow_mut() = Some(Box::new(callback));
    }

    /// Play the selected rows.
    pub fn activate_selected(&self) {
        let selected = self.selected_track_ids();
        if selected.is_empty() {
            return;
        }
        if let Some(callback) = self.on_activate_selection.borrow().as_ref() {
            callback(selected);
        }
    }

    /// Ask the loader for thumbnails for the rows near the top of the list.
    ///
    /// Rows already loaded are updated in place; the rest are queued, and the caller
    /// passes finished results back to [`Self::apply_artwork`].
    pub fn request_artwork(&self, core: &AppCore) {
        let loader = core.artwork();
        for object in self.rows.borrow().iter().take(ARTWORK_PREFETCH) {
            if object.artwork_path().is_some() {
                continue;
            }
            match loader.ready(object.track_id(), THUMBNAIL_SIZE) {
                Some(Some(path)) => {
                    object.set_artwork_path(Some(path.to_string_lossy().into_owned()));
                    continue;
                }
                // Known to have no artwork: do not ask again.
                Some(None) => continue,
                None => {}
            }
            let path = PathBuf::from(object.path());
            loader.request(object.track_id(), &path, THUMBNAIL_SIZE);
        }
    }

    /// Apply finished artwork by re-binding the affected rows.
    pub fn apply_artwork(&self, results: &[ArtworkResult]) {
        for result in results {
            let ArtworkResult::Ready { track_id, path, .. } = result else {
                continue;
            };
            let position = self
                .rows
                .borrow()
                .iter()
                .position(|object| object.track_id() == *track_id);
            if let Some(position) = position {
                if let Some(object) = self.row_at(position) {
                    object.set_artwork_path(Some(path.to_string_lossy().into_owned()));
                    self.store.items_changed(position as u32, 1, 1);
                }
            }
        }
    }

    /// Scroll a row into view.
    pub fn scroll_to(&self, index: usize) {
        self.list
            .scroll_to(index as u32, gtk::ListScrollFlags::FOCUS, None);
    }

    fn setup_signals(&self) {
        let callback = Rc::clone(&self.on_activate);
        self.list.connect_activate(move |_, position| {
            if let Some(callback) = callback.borrow().as_ref() {
                callback(i64::from(position));
            }
        });

        // The selection model is where a click's selection is reported.
        let selected = Rc::clone(&self.selected);
        let rows = Rc::clone(&self.rows);
        let on_selection_changed = Rc::clone(&self.on_selection_changed);
        self.selection.connect_selected_item_notify(move |model| {
            let indexes: Vec<usize> = model
                .selected_item()
                .and_downcast::<TrackObject>()
                .and_then(|object| rows.borrow().iter().position(|row| *row == object))
                .into_iter()
                .collect();
            *selected.borrow_mut() = indexes.clone();
            if let Some(callback) = on_selection_changed.borrow().as_ref() {
                callback(indexes);
            }
        });
    }

    fn setup_drag_and_drop(&self) {
        let origin_begin = Rc::clone(&self.drag_origin);
        let selected = Rc::clone(&self.selected);
        let origin = Rc::clone(&self.drag_origin);

        let source = gtk::DragSource::new();
        // The dragged row is the selected one; GTK 4.14 no longer reports the pointer
        // position when a drag starts.
        source.connect_drag_begin(move |_, _drag| {
            origin_begin.set(selected.borrow().first().map_or(-1, |index| *index as i64));
        });
        source.connect_prepare(move |_, _x, _y| {
            if origin.get() < 0 {
                return None;
            }
            let value = origin.get().to_value();
            Some(gdk::ContentProvider::for_value(&value))
        });
        let origin_end = Rc::clone(&self.drag_origin);
        source.connect_drag_end(move |_, _drag, _delete| origin_end.set(-1));
        self.list.add_controller(source);

        let target = gtk::DropTarget::new(glib::Type::I64, gdk::DragAction::MOVE);
        let origin_cell = Rc::clone(&self.drag_origin);
        let rows = Rc::clone(&self.rows);
        let store = self.store.clone();
        let selected = Rc::clone(&self.selected);
        let reordered = Rc::clone(&self.on_reordered);
        target.connect_drop(move |_, value, _x, y| {
            let Ok(from) = value.get::<i64>() else {
                return false;
            };
            let to = row_at_point(y);
            if from < 0 || to < 0 {
                return false;
            }
            if !reorder(&rows, &store, from as usize, to as usize) {
                return false;
            }
            origin_cell.set(-1);
            *selected.borrow_mut() = vec![to as usize];
            if let Some(callback) = reordered.borrow().as_ref() {
                callback(from as usize, to as usize);
            }
            true
        });
        self.list.add_controller(target);
    }
}

impl Default for TrackList {
    fn default() -> Self {
        Self::new()
    }
}

/// Sorted, duplicate-free copy of a set of row indexes.
pub fn normalize_selection(indexes: &[usize]) -> Vec<usize> {
    let mut sorted = indexes.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    sorted
}

/// Take an item out of `items` and put it back at `to`, returning what was moved.
///
/// Returns `None` for indexes outside the list or a move onto itself, so callers can treat
/// an out-of-range drop as "nothing happened".
pub fn move_item<T: Clone>(items: &mut Vec<T>, from: usize, to: usize) -> Option<T> {
    if from >= items.len() || to >= items.len() || from == to {
        return None;
    }
    let item = items.remove(from);
    items.insert(to, item.clone());
    Some(item)
}

/// Move a row and rebuild the model, so the store and the bookkeeping stay in step.
fn reorder(
    rows: &Rc<RefCell<Vec<TrackObject>>>,
    store: &gio::ListStore,
    from: usize,
    to: usize,
) -> bool {
    let mut objects = rows.borrow_mut();
    let Some(_) = move_item(&mut objects, from, to) else {
        return false;
    };
    store.remove_all();
    for object in objects.iter() {
        store.append(object);
    }
    true
}

/// Row under a vertical pointer position. Rows have a fixed height, so this is accurate
/// enough for dropping.
pub fn row_at_point(y: f64) -> i32 {
    ((y / f64::from(ROW_HEIGHT)).round() as i32).max(0)
}

/// One row: thumbnail, track number, title, artist, album, length, favorite, status.
fn build_row() -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 8);
    row.set_margin_top(2);
    row.set_margin_bottom(2);
    row.set_margin_start(6);
    row.set_margin_end(6);
    row.set_height_request(ROW_HEIGHT);

    let artwork = Image::from_icon_name("audio-x-generic-symbolic");
    artwork.set_pixel_size(THUMBNAIL_SIZE as i32);
    artwork.set_tooltip_text(Some("Artwork"));

    let number = text_label("", Align::End);
    number.set_width_chars(6);
    let title = text_label("", Align::Start);
    let artist = text_label("", Align::Start);
    let album = text_label("", Align::Start);
    let duration = text_label("", Align::End);
    duration.set_width_chars(7);
    let favorite = Image::from_icon_name("starred-symbolic");
    favorite.set_tooltip_text(Some("Favorite"));
    let status = hint_label("");

    for widget in [
        artwork.upcast_ref::<gtk::Widget>(),
        number.upcast_ref(),
        title.upcast_ref(),
        artist.upcast_ref(),
        album.upcast_ref(),
        duration.upcast_ref(),
        favorite.upcast_ref(),
        status.upcast_ref(),
    ] {
        row.append(widget);
    }

    row
}

/// Push an object's values into an already-built row.
fn bind_row(row: &GtkBox, object: &TrackObject) {
    let children = widgets::children(row);
    if children.len() < 8 {
        return;
    }

    if let Some(image) = children[0].downcast_ref::<Image>() {
        match object.artwork_path() {
            Some(path) => {
                image.set_pixel_size(THUMBNAIL_SIZE as i32);
                image.set_from_file(Some(path));
            }
            None => {
                image.set_pixel_size(THUMBNAIL_SIZE as i32);
                image.set_icon_name(Some("audio-x-generic-symbolic"));
            }
        }
    }
    if let Some(label) = children[1].downcast_ref::<Label>() {
        label.set_text(&object.number());
    }
    if let Some(label) = children[2].downcast_ref::<Label>() {
        label.set_text(&object.title());
        label.set_hexpand(true);
    }
    if let Some(label) = children[3].downcast_ref::<Label>() {
        label.set_text(&object.artist());
        label.set_hexpand(true);
    }
    if let Some(label) = children[4].downcast_ref::<Label>() {
        label.set_text(&object.album());
        label.set_hexpand(true);
    }
    if let Some(label) = children[5].downcast_ref::<Label>() {
        label.set_text(&object.duration());
    }
    if let Some(image) = children[6].downcast_ref::<Image>() {
        image.set_visible(object.favorite());
    }
    if let Some(label) = children[7].downcast_ref::<Label>() {
        label.set_text(if object.missing() { "Unavailable" } else { "" });
    }
}

fn build_header() -> GtkBox {
    let header = GtkBox::new(Orientation::Horizontal, 0);
    header.add_css_class("strata-track-header");
    for (label, width) in [
        ("#", 60usize),
        ("Title", 240),
        ("Artist", 180),
        ("Album", 180),
        ("Length", 90),
    ] {
        let button = gtk::Button::with_label(label);
        button.set_size_request(width as i32, -1);
        button.add_css_class("flat");
        header.append(&button);
    }
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pointer_position_maps_to_a_row() {
        assert_eq!(row_at_point(0.0), 0);
        assert_eq!(row_at_point(20.0), 0);
        assert_eq!(row_at_point(50.0), 1);
        assert_eq!(row_at_point(-10.0), 0);
    }

    #[test]
    fn selection_is_sorted_and_deduplicated() {
        assert_eq!(normalize_selection(&[3, 1, 1]), vec![1, 3]);
        assert_eq!(normalize_selection(&[]), Vec::<usize>::new());
    }

    #[test]
    fn a_move_takes_the_row_out_and_puts_it_back() {
        let mut items = vec!['a', 'b', 'c', 'd'];
        assert_eq!(move_item(&mut items, 0, 2), Some('a'));
        assert_eq!(items, vec!['b', 'c', 'a', 'd']);
        assert_eq!(move_item(&mut items, 3, 0), Some('d'));
        assert_eq!(items, vec!['d', 'b', 'c', 'a']);
    }

    #[test]
    fn a_move_outside_the_list_is_ignored() {
        let mut items = vec!['a', 'b'];
        assert_eq!(move_item(&mut items, 0, 9), None);
        assert_eq!(move_item(&mut items, 5, 0), None);
        assert_eq!(move_item(&mut items, 1, 1), None);
        assert_eq!(items, vec!['a', 'b']);
    }
}
