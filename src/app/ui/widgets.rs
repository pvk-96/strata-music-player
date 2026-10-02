//! Small widget helpers.
//!
//! Kept in one place so the views build the same kinds of widget in the same way, and so
//! the look is consistent without a custom CSS file full of one-off rules.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Label, Orientation};

/// A label with sensible defaults for lists and dialogs.
pub fn text_label(text: &str, align: Align) -> Label {
    let label = Label::new(Some(text));
    label.set_xalign(match align {
        Align::Start => 0.0,
        Align::End => 1.0,
        Align::Center => 0.5,
        _ => 0.0,
    });
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label
}

/// A dimmed label for hints and footers.
pub fn hint_label(text: &str) -> Label {
    let label = text_label(text, Align::Start);
    label.add_css_class("dim-label");
    label
}

/// A bold label used as a dialog section title.
pub fn section_label(text: &str) -> Label {
    let label = text_label(text, Align::Start);
    label.add_css_class("heading");
    label
}

/// A vertical stack with consistent spacing.
pub fn vstack(spacing: i32) -> GtkBox {
    GtkBox::new(Orientation::Vertical, spacing)
}

/// A horizontal stack with consistent spacing.
pub fn hstack(spacing: i32) -> GtkBox {
    GtkBox::new(Orientation::Horizontal, spacing)
}

/// A labelled section: title above content, with the content appended by the caller.
pub fn section(title: &str) -> GtkBox {
    let box_ = vstack(6);
    box_.append(&section_label(title));
    box_
}

/// An icon button with a tooltip, used for the transport controls.
pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("flat");
    button.set_tooltip_text(Some(tooltip));
    button
}

/// A toggle button that shows whether it is on.
pub fn toggle_button(icon: &str, tooltip: &str) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::new();
    button.set_icon_name(icon);
    button.add_css_class("flat");
    button.set_tooltip_text(Some(tooltip));
    button
}

/// A horizontal bar of buttons.
pub fn button_bar(buttons: &[gtk::Widget]) -> GtkBox {
    let bar = hstack(6);
    bar.add_css_class("linked");
    for button in buttons {
        bar.append(button);
    }
    bar
}

/// The children of a box, in order.
///
/// GTK's own `children` helper is not available in this binding version, so boxes are walked
/// by hand wherever a view needs to look inside one.
pub fn children(box_: &GtkBox) -> Vec<gtk::Widget> {
    box_.first_child()
        .into_iter()
        .flat_map(|child| std::iter::successors(Some(child), |widget| widget.next_sibling()))
        .collect()
}

/// A separator for use inside vertical stacks.
pub fn separator() -> gtk::Separator {
    gtk::Separator::new(Orientation::Horizontal)
}

/// A left-aligned row: a fixed-width leading widget and a label that fills the rest.
pub fn labelled_row(label: &str) -> (gtk::Widget, Label) {
    let caption = Label::new(Some(label));
    caption.set_width_chars(22);
    caption.set_xalign(0.0);
    caption.add_css_class("dim-label");
    let value = text_label("", Align::Start);
    value.set_hexpand(true);
    (caption.upcast(), value)
}

/// Copy text to the clipboard.
pub fn copy_to_clipboard(text: &str) {
    if let Some(display) = gdk::Display::default() {
        display.clipboard().set_text(text);
    }
}

/// Show a file in the desktop's file manager.
///
/// GTK 4.11 deprecated launching the default handler for a file, so the containing folder
/// is handed to the desktop's own opener instead.
pub fn show_in_file_manager(path: &std::path::Path) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(file_manager_target(path, path.is_dir()))
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// What a file-manager request should open for `path`: the path itself when it is a folder,
/// otherwise the folder holding it.
pub fn file_manager_target(path: &std::path::Path, is_folder: bool) -> std::path::PathBuf {
    if is_folder {
        return path.to_path_buf();
    }
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| path.to_path_buf(), std::path::Path::to_path_buf)
}

/// Show a plain message dialog.
///
/// GTK 4.10 deprecated `MessageDialog` in favour of `AlertDialog`, which also means one
/// response callback instead of a nested loop.
pub fn message_dialog<F>(parent: Option<&gtk::Window>, title: &str, message: &str, on_response: F)
where
    F: FnOnce(gtk::ResponseType) + 'static,
{
    let dialog = gtk::AlertDialog::builder()
        .message(title)
        .detail(message)
        .modal(true)
        .build();
    dialog.choose(parent, None::<&gtk::gio::Cancellable>, move |response| {
        on_response(if matches!(response, Ok(0)) {
            gtk::ResponseType::Close
        } else {
            gtk::ResponseType::DeleteEvent
        });
    });
}

/// Show an error dialog.
pub fn error_dialog(parent: Option<&gtk::Window>, title: &str, message: &str) {
    message_dialog(parent, title, message, |_| {});
}

/// Ask a yes-or-no question, then continue if the user agrees.
pub fn confirm_dialog<F>(parent: Option<&gtk::Window>, title: &str, message: &str, on_confirmed: F)
where
    F: FnOnce(bool) + 'static,
{
    let dialog = gtk::AlertDialog::builder()
        .message(title)
        .detail(message)
        .modal(true)
        .build();
    dialog.choose(parent, None::<&gtk::gio::Cancellable>, move |response| {
        on_confirmed(matches!(response, Ok(0)));
    });
}

/// Ask for one piece of information in a line of text. The callback runs with the typed
/// text, or an empty string when the prompt was cancelled or dismissed.
pub fn text_prompt<F>(
    parent: Option<&gtk::Window>,
    title: &str,
    prompt: &str,
    initial: &str,
    on_text: F,
) where
    F: FnOnce(String) + 'static,
{
    let window = gtk::Window::builder()
        .title(title)
        .modal(true)
        .resizable(false)
        .default_width(320)
        .build();
    if let Some(parent) = parent {
        window.set_transient_for(Some(parent));
    }

    let entry = gtk::Entry::new();
    entry.set_text(initial);
    entry.set_activates_default(true);
    entry.set_placeholder_text(Some(prompt));

    let cancel = gtk::Button::with_label("_Cancel");
    let accept = gtk::Button::with_label("_OK");
    accept.add_css_class("suggested-action");
    let header = gtk::HeaderBar::new();
    header.pack_start(&cancel);
    header.pack_end(&accept);
    window.set_titlebar(Some(&header));

    let content = gtk::Box::new(Orientation::Vertical, 0);
    content.set_spacing(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&entry);
    window.set_child(Some(&content));

    let answer: PromptAnswer = Rc::new(RefCell::new(Some(Box::new(on_text))));

    let dismiss_window = window.clone();
    let dismiss_answer = Rc::clone(&answer);
    cancel.connect_clicked(move |_| {
        dismiss_window.set_visible(false);
        answer_once(&dismiss_answer, String::new());
    });

    let accept_window = window.clone();
    let accept_entry = entry.clone();
    let accept_answer = Rc::clone(&answer);
    accept.connect_clicked(move |_| {
        let text = accept_entry.text().trim().to_string();
        accept_window.set_visible(false);
        answer_once(&accept_answer, text);
    });

    let activate_entry = entry.clone();
    let activate_answer = Rc::clone(&answer);
    entry.connect_activate(move |_| {
        answer_once(&activate_answer, activate_entry.text().trim().to_string())
    });

    let close_answer = Rc::clone(&answer);
    window.connect_close_request(move |_| {
        answer_once(&close_answer, String::new());
        gtk::glib::Propagation::Proceed
    });
    window.set_visible(true);
}

/// Where a text prompt keeps its one-shot answer callback.
type PromptAnswer = Rc<RefCell<Option<Box<dyn FnOnce(String)>>>>;

/// Hand the answer to the prompt callback, at most once.
fn answer_once(answer: &PromptAnswer, text: String) {
    if let Some(callback) = answer.borrow_mut().take() {
        callback(text);
    }
}

/// What a file chooser should return.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChooserKind {
    /// One existing folder.
    Folder,
    /// One or more existing files.
    Files,
    /// One existing file, with optional name filters.
    File { filters: Vec<String> },
    /// A file name to write, which may not exist yet.
    SaveFile { suggested_name: String },
}

impl ChooserKind {
    fn title(&self) -> &str {
        match self {
            Self::Folder => "Choose a folder",
            Self::Files | Self::File { .. } => "Choose a file",
            Self::SaveFile { .. } => "Save as",
        }
    }
}

/// Ask the user for paths. GTK file dialogs are asynchronous, so the answer arrives in
/// `on_chosen`; a cancelled dialog calls it with `None`.
pub fn choose_paths<F>(
    parent: Option<&gtk::Window>,
    kind: ChooserKind,
    start: Option<&std::path::Path>,
    on_chosen: F,
) where
    F: FnOnce(Option<Vec<std::path::PathBuf>>) + 'static,
{
    let dialog = gtk::FileDialog::new();
    dialog.set_title(kind.title());
    dialog.set_modal(true);
    if let Some(folder) = default_folder(start) {
        dialog.set_initial_folder(Some(&folder));
    }
    match &kind {
        ChooserKind::SaveFile { suggested_name } => dialog.set_initial_name(Some(suggested_name)),
        ChooserKind::File { filters } => {
            let names: Vec<&str> = filters.iter().map(String::as_str).collect();
            dialog.set_default_filter(Some(&name_filter(&names)));
        }
        _ => {}
    }

    let cancellable: Option<&gtk::gio::Cancellable> = None;
    match kind {
        ChooserKind::Folder => dialog.select_folder(parent, cancellable, move |file| {
            on_chosen(paths_from(file));
        }),
        ChooserKind::File { .. } => dialog.open(parent, cancellable, move |file| {
            on_chosen(paths_from(file));
        }),
        ChooserKind::Files => dialog.open_multiple(parent, cancellable, move |files| {
            on_chosen(paths_from_model(files));
        }),
        ChooserKind::SaveFile { .. } => dialog.save(parent, cancellable, move |file| {
            on_chosen(paths_from(file));
        }),
    }
}

/// One file, or nothing when the user cancelled.
fn paths_from(file: Result<gtk::gio::File, gtk::glib::Error>) -> Option<Vec<std::path::PathBuf>> {
    file.ok()
        .and_then(|file| file.path())
        .map(|path| vec![path])
}

/// Every file of a multiple selection.
fn paths_from_model(
    files: Result<gtk::gio::ListModel, gtk::glib::Error>,
) -> Option<Vec<std::path::PathBuf>> {
    let files = files.ok()?;
    Some(
        (0..files.n_items())
            .filter_map(|index| files.item(index))
            .filter_map(|item| item.downcast::<gtk::gio::File>().ok())
            .filter_map(|file| file.path())
            .collect(),
    )
}

/// The folder a chooser should open in.
fn default_folder(hint: Option<&std::path::Path>) -> Option<gtk::gio::File> {
    let hint = hint?;
    if hint.as_os_str().is_empty() {
        return None;
    }
    let folder = if hint.is_dir() {
        hint.to_path_buf()
    } else {
        hint.parent().unwrap_or(hint).to_path_buf()
    };
    folder.is_dir().then(|| gtk::gio::File::for_path(folder))
}

/// A filter matching any of several glob patterns.
fn name_filter(names: &[&str]) -> gtk::FileFilter {
    let filter = gtk::FileFilter::new();
    for pattern in glob_patterns(names) {
        filter.add_pattern(&pattern);
    }
    filter.set_name(Some("Supported files"));
    filter
}

/// Turn bare extensions such as `m3u8` into the glob patterns GTK wants.
fn glob_patterns(names: &[&str]) -> Vec<String> {
    names
        .iter()
        .map(|name| {
            if name.contains('*') || name.contains('?') {
                name.to_string()
            } else {
                format!("*.{name}")
            }
        })
        .collect()
}

/// Ask for one folder.
pub fn choose_folder<F>(parent: Option<&gtk::Window>, start: Option<&std::path::Path>, on_chosen: F)
where
    F: FnOnce(Option<std::path::PathBuf>) + 'static,
{
    choose_paths(parent, ChooserKind::Folder, start, move |paths| {
        on_chosen(paths.and_then(|paths| paths.into_iter().next()));
    });
}

/// Ask for one file, optionally filtering by extension.
pub fn choose_file<F>(
    parent: Option<&gtk::Window>,
    start: Option<&std::path::Path>,
    filters: Vec<String>,
    on_chosen: F,
) where
    F: FnOnce(Option<std::path::PathBuf>) + 'static,
{
    let kind = ChooserKind::File { filters };
    choose_paths(parent, kind, start, move |paths| {
        on_chosen(paths.and_then(|paths| paths.into_iter().next()));
    });
}

/// Ask for a name to save a file as.
pub fn choose_save_file<F>(
    parent: Option<&gtk::Window>,
    suggested_name: &str,
    start: Option<&std::path::Path>,
    on_chosen: F,
) where
    F: FnOnce(Option<std::path::PathBuf>) + 'static,
{
    let kind = ChooserKind::SaveFile {
        suggested_name: suggested_name.to_string(),
    };
    choose_paths(parent, kind, start, move |paths| {
        on_chosen(paths.and_then(|paths| paths.into_iter().next()));
    });
}

/// Ask GTK to run `callback` once, on the main loop.
pub fn later(callback: impl FnOnce() + 'static) {
    let _ = glib::idle_add_local_once(callback);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn a_file_manager_request_opens_the_folder() {
        assert_eq!(
            file_manager_target(Path::new("/music/albums/track.mp3"), false),
            PathBuf::from("/music/albums")
        );
        assert_eq!(
            file_manager_target(Path::new("/music/albums"), true),
            PathBuf::from("/music/albums")
        );
        assert_eq!(
            file_manager_target(Path::new("track.mp3"), false),
            PathBuf::from("track.mp3")
        );
    }

    #[test]
    fn chooser_kinds_name_themselves() {
        assert_eq!(ChooserKind::Folder.title(), "Choose a folder");
        assert_eq!(ChooserKind::Files.title(), "Choose a file");
        assert_eq!(
            ChooserKind::SaveFile {
                suggested_name: "out.m3u".into()
            }
            .title(),
            "Save as"
        );
    }

    #[test]
    fn extensions_become_glob_patterns() {
        assert_eq!(glob_patterns(&["m3u", "m3u8"]), vec!["*.m3u", "*.m3u8"]);
        assert_eq!(glob_patterns(&["*.flac"]), vec!["*.flac"]);
        assert!(glob_patterns(&["a?c"]).contains(&"a?c".to_string()));
    }
}
