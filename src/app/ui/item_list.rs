//! A plain list of named items: artists, albums, folders, playlists.
//!
//! The collection views all need the same thing — a scrollable list where activating a row
//! means "go there" — so they share this widget. Rows are strings in a `StringList`, which
//! keeps scrolling a large library cheap; a row's string holds its detail line and its title
//! separated by [`SEPARATOR`], so the factory can build a row without any shared state.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gio;
use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Label, ListView, Orientation, ScrolledWindow};

use crate::app::ui::widgets::{hint_label, text_label};

/// Separator between a row's detail line and its title inside the model's string.
pub const SEPARATOR: char = '\u{1f}';

/// One row: a stable key, a name, a detail line, and an optional icon name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemLabel {
    /// Stable key, unique within one list: the core uses it to work out what was chosen.
    pub key: String,
    pub title: String,
    pub detail: String,
    pub icon: Option<String>,
}

impl ItemLabel {
    pub fn new(key: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            detail: String::new(),
            icon: None,
        }
    }

    pub fn with_detail(
        key: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            detail: detail.into(),
            icon: None,
        }
    }

    pub fn with_icon(mut self, icon: &str) -> Self {
        self.icon = Some(icon.to_string());
        self
    }

    /// The string stored in the model for this row.
    pub fn row_text(&self) -> String {
        format!("{}{SEPARATOR}{}", self.detail, self.title)
    }
}

/// Split a row string back into its detail line and title.
pub fn split_row_text(text: &str) -> (String, String) {
    // The detail line is written first and may itself contain the separator, so the split is
    // made at the last one.
    match text.rsplit_once(SEPARATOR) {
        Some((detail, title)) => (detail.to_string(), title.to_string()),
        None => (String::new(), text.to_string()),
    }
}

type ActivateCallback = Box<dyn Fn(usize)>;
type SelectionCallback = Box<dyn Fn(Option<usize>)>;

pub struct ItemList {
    root: ScrolledWindow,
    list: ListView,
    selection: gtk::SingleSelection,
    store: gio::ListStore,
    /// Row text and the item behind it. Both are reached through `Rc` clones held by the
    /// callbacks below, so they are shared state rather than owned fields: the window fills the
    /// list with `set_items` long after a callback has taken its own handle.
    rows: RefCell<Vec<String>>,
    items: RefCell<Vec<ItemLabel>>,
    on_activate: Rc<RefCell<Option<ActivateCallback>>>,
    on_selection_changed: Rc<RefCell<Option<SelectionCallback>>>,
}

impl ItemList {
    pub fn new() -> Self {
        let store = gio::ListStore::new::<gtk::StringObject>();

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
            let Some(object) = item.item().and_downcast::<gtk::StringObject>() else {
                return;
            };
            let Some(row) = item.child().and_downcast::<GtkBox>() else {
                return;
            };
            let (detail, title) = split_row_text(&object.string());
            bind_row(&row, &title, &detail);
        });

        let selection = gtk::SingleSelection::new(Some(store.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);

        let list = ListView::new(Some(selection.clone()), Some(factory));
        list.set_single_click_activate(false);

        let scroller = ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&list)
            .build();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_has_frame(false);

        let item_list = Self {
            root: scroller,
            list,
            selection,
            store,
            rows: RefCell::new(Vec::new()),
            items: RefCell::new(Vec::new()),
            on_activate: Rc::new(RefCell::new(None)),
            on_selection_changed: Rc::new(RefCell::new(None)),
        };

        let on_activate = Rc::clone(&item_list.on_activate);
        item_list.list.connect_activate(move |_, position| {
            if let Some(callback) = on_activate.borrow().as_ref() {
                callback(position as usize);
            }
        });

        item_list
    }

    pub fn widget(&self) -> &ScrolledWindow {
        &self.root
    }

    pub fn list(&self) -> &ListView {
        &self.list
    }

    pub fn item(&self, index: usize) -> Option<ItemLabel> {
        self.items.borrow().get(index).cloned()
    }

    /// Replace the list.
    pub fn set_items(&self, items: Vec<ItemLabel>) {
        self.store.remove_all();
        let rows = items.iter().map(ItemLabel::row_text).collect::<Vec<_>>();
        for row in &rows {
            self.store.append(&gtk::StringObject::new(row));
        }
        *self.rows.borrow_mut() = rows;
        *self.items.borrow_mut() = items;
    }

    pub fn set_selected(&self, index: Option<usize>) {
        self.selection
            .set_selected(index.map_or(gtk::INVALID_LIST_POSITION, |index| index as u32));
    }

    /// Index of the selected row, if any.
    pub fn selected(&self) -> Option<usize> {
        let text = self
            .selection
            .selected_item()?
            .downcast_ref::<gtk::StringObject>()?
            .string();
        self.rows.borrow().iter().position(|row| *row == text)
    }

    pub fn connect_activate<F: Fn(usize) + 'static>(&self, callback: F) {
        *self.on_activate.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_selection_changed<F: Fn(Option<usize>) + 'static>(&self, callback: F) {
        *self.on_selection_changed.borrow_mut() = Some(Box::new(callback));
    }
}

impl Default for ItemList {
    fn default() -> Self {
        Self::new()
    }
}

fn build_row() -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 8);
    row.set_margin_top(4);
    row.set_margin_bottom(4);
    row.set_margin_start(8);
    row.set_margin_end(8);

    let icon = gtk::Image::from_icon_name("audio-x-generic-symbolic");
    let title = text_label("", Align::Start);
    let detail_holder = GtkBox::new(Orientation::Horizontal, 6);
    detail_holder.append(&hint_label(""));

    for widget in [
        icon.upcast_ref::<gtk::Widget>(),
        title.upcast_ref(),
        detail_holder.upcast_ref(),
    ] {
        row.append(widget);
    }
    row
}

fn bind_row(row: &GtkBox, title_text: &str, detail_text: &str) {
    let children: Vec<gtk::Widget> = row
        .first_child()
        .into_iter()
        .flat_map(|child| std::iter::successors(Some(child), |widget| widget.next_sibling()))
        .collect();
    if children.len() < 3 {
        return;
    }

    if let Some(title) = children[1].downcast_ref::<Label>() {
        title.set_text(title_text);
        title.set_hexpand(true);
    }
    if let Some(holder) = children[2].downcast_ref::<GtkBox>() {
        if let Some(detail) = holder.first_child().and_downcast::<Label>() {
            detail.set_text(detail_text);
        }
        holder.set_visible(!detail_text.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_carry_a_key_title_and_detail() {
        let label = ItemLabel::with_detail("album:Artist:Name", "Name", "Artist · 2001");
        assert_eq!(label.key, "album:Artist:Name");
        assert_eq!(label.title, "Name");
        assert_eq!(label.detail, "Artist · 2001");
        assert_eq!(label.icon, None);

        let with_icon = ItemLabel::new("folder:1", "Music").with_icon("folder-symbolic");
        assert_eq!(with_icon.icon.as_deref(), Some("folder-symbolic"));
    }

    #[test]
    fn a_row_text_round_trips_through_the_model() {
        let label = ItemLabel::with_detail("k", "Title", "Detail line");
        let (detail, title) = split_row_text(&label.row_text());
        assert_eq!(title, "Title");
        assert_eq!(detail, "Detail line");
    }

    #[test]
    fn a_row_text_without_a_detail_still_has_a_title() {
        let label = ItemLabel::new("k", "Title");
        let (detail, title) = split_row_text(&label.row_text());
        assert_eq!(title, "Title");
        assert!(detail.is_empty());

        let (detail, title) = split_row_text("bare title");
        assert_eq!(title, "bare title");
        assert!(detail.is_empty());
    }

    #[test]
    fn a_detail_line_may_contain_the_separator() {
        let (detail, title) = split_row_text(&format!("a{SEPARATOR}b{SEPARATOR}Title"));
        assert_eq!(title, "Title");
        assert_eq!(detail, format!("a{SEPARATOR}b"));
    }
}
