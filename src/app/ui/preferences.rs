//! Settings window.
//!
//! The dialog edits a copy of the settings. Nothing is applied until the user accepts,
//! and the edited values go through the same validation as the settings file, so the
//! window and the file cannot drift apart.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::glib::Propagation;
use gtk::prelude::*;
use gtk::{Align, EventControllerKey, Window};

use crate::app::ui::widgets;
use crate::settings::keyboard::KeyboardSettings;
use crate::settings::model::{Density, Settings, Theme};
use crate::settings::shortcut::Shortcut;

/// Called with the edited settings when the dialog is accepted.
pub type ApplyCallback = Rc<dyn Fn(Settings)>;

/// Everything the response handler needs; shared so the `'static` closures can reach it.
struct Editor {
    base: Settings,
    theme: gtk::DropDown,
    density: gtk::DropDown,
    sidebar_visible: gtk::Switch,
    resume_session: gtk::Switch,
    show_track_artwork: gtk::Switch,
    seek_seconds: gtk::SpinButton,
    keyboard: RefCell<KeyboardSettings>,
    actions: RefCell<Vec<crate::app::actions::Action>>,
    selected: RefCell<Option<usize>>,
    list: gtk::ListBox,
    hint: gtk::Label,
}

/// The settings window. Keep it alive for as long as the dialog should exist; dropping it
/// ends the dialog.
pub struct Preferences {
    window: Window,
    editor: Rc<Editor>,
}

impl Preferences {
    pub fn new(parent: Option<&gtk::Window>, settings: &Settings, on_apply: ApplyCallback) -> Self {
        let mut builder = Window::builder()
            .title("Strata Settings")
            .modal(true)
            .resizable(true)
            .default_width(560)
            .default_height(600);
        if let Some(parent) = parent {
            builder = builder.transient_for(parent);
        }
        let window = builder.build();

        let editor = Rc::new(Editor {
            base: settings.clone(),
            theme: gtk::DropDown::from_strings(&["System", "Light", "Dark"]),
            density: gtk::DropDown::from_strings(&["Comfortable", "Compact"]),
            sidebar_visible: gtk::Switch::new(),
            resume_session: gtk::Switch::new(),
            show_track_artwork: gtk::Switch::new(),
            seek_seconds: gtk::SpinButton::with_range(1.0, 600.0, 1.0),
            keyboard: RefCell::new(settings.keyboard.clone()),
            actions: RefCell::new(Vec::new()),
            selected: RefCell::new(None),
            list: gtk::ListBox::new(),
            hint: gtk::Label::new(None),
        });
        load(editor.as_ref(), settings);

        build(editor.clone(), &window);
        wire_shortcuts(editor.clone(), &window);
        wire_buttons(editor.clone(), on_apply, &window);

        window.connect_close_request(|window| {
            window.set_visible(false);
            Propagation::Proceed
        });

        Self { window, editor }
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn show(&self) {
        self.window.set_visible(true);
        self.window.present();
    }

    /// Read the widgets back into a settings value, validated the same way the file is.
    pub fn collect(&self) -> Settings {
        collect(self.editor.as_ref())
    }
}

/// Header bar buttons: cancel, apply, restore defaults, close.
fn wire_buttons(editor: Rc<Editor>, on_apply: ApplyCallback, window: &Window) {
    let header = gtk::HeaderBar::new();
    let cancel = gtk::Button::with_label("_Cancel");
    let apply = gtk::Button::with_label("_Apply");
    apply.add_css_class("suggested-action");
    let defaults = gtk::Button::with_label("Restore _Defaults");
    let close = gtk::Button::with_label("_Close");
    header.pack_start(&cancel);
    header.pack_start(&apply);
    header.pack_end(&defaults);
    header.pack_end(&close);
    window.set_titlebar(Some(&header));

    let close_window = window.clone();
    cancel.connect_clicked(move |_| close_window.set_visible(false));
    let close_window = window.clone();
    close.connect_clicked(move |_| close_window.set_visible(false));

    let apply_editor = Rc::clone(&editor);
    let callback = Rc::clone(&on_apply);
    apply.connect_clicked(move |_| {
        callback(collect(apply_editor.as_ref()));
    });

    let defaults_editor = Rc::clone(&editor);
    defaults.connect_clicked(move |_| {
        defaults_editor.keyboard.borrow_mut().reset_defaults();
        rebuild_rows(defaults_editor.as_ref());
    });
}

fn load(editor: &Editor, settings: &Settings) {
    editor.theme.set_selected(match settings.appearance.theme {
        Theme::System => 0,
        Theme::Light => 1,
        Theme::Dark => 2,
    });
    editor
        .density
        .set_selected(u32::from(settings.appearance.density == Density::Compact));
    editor
        .sidebar_visible
        .set_active(settings.appearance.sidebar_visible);
    editor
        .resume_session
        .set_active(settings.playback.resume_session);
    editor
        .show_track_artwork
        .set_active(settings.playback.show_track_artwork_in_player);
    editor
        .seek_seconds
        .set_value(f64::from(settings.playback.seek_seconds));
}

fn collect(editor: &Editor) -> Settings {
    let mut settings = editor.base.clone();
    settings.appearance.theme = match editor.theme.selected() {
        0 => Theme::System,
        1 => Theme::Light,
        _ => Theme::Dark,
    };
    settings.appearance.density = if editor.density.selected() == 1 {
        Density::Compact
    } else {
        Density::Comfortable
    };
    settings.appearance.sidebar_visible = editor.sidebar_visible.is_active();
    settings.playback.resume_session = editor.resume_session.is_active();
    settings.playback.show_track_artwork_in_player = editor.show_track_artwork.is_active();
    settings.playback.seek_seconds = editor.seek_seconds.value().clamp(1.0, 600.0) as u32;
    settings.keyboard = editor.keyboard.borrow().clone();
    settings.validate();
    settings
}

fn build(editor: Rc<Editor>, window: &Window) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_spacing(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    content.append(&widgets::section_label("Appearance"));
    let appearance = grid();
    appearance.attach(&label("Theme"), 0, 0, 1, 1);
    appearance.attach(&editor.theme, 1, 0, 1, 1);
    appearance.attach(&label("Row density"), 0, 1, 1, 1);
    appearance.attach(&editor.density, 1, 1, 1, 1);
    appearance.attach(
        &switch_row(&editor.sidebar_visible, "Show the sidebar"),
        1,
        2,
        1,
        1,
    );
    content.append(&appearance);

    content.append(&widgets::section_label("Playback"));
    let playback = grid();
    playback.attach(&label("Seek amount"), 0, 0, 1, 1);
    editor
        .seek_seconds
        .set_tooltip_text(Some("Seconds skipped by the seek actions"));
    playback.attach(&editor.seek_seconds, 1, 0, 1, 1);
    playback.attach(
        &switch_row(&editor.resume_session, "Resume the previous session"),
        1,
        1,
        1,
        1,
    );
    playback.attach(
        &switch_row(
            &editor.show_track_artwork,
            "Show track artwork in the player",
        ),
        1,
        2,
        1,
        1,
    );
    content.append(&playback);

    content.append(&widgets::section_label("Keyboard Shortcuts"));
    editor.hint.set_text(
        "Select a shortcut, then press a key combination to set it. Press Delete to clear it.",
    );
    editor.hint.set_xalign(0.0);
    editor.hint.set_wrap(true);
    editor.hint.set_margin_bottom(6);
    content.append(&editor.hint);

    let shortcuts = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    editor.list.set_selection_mode(gtk::SelectionMode::Single);
    shortcuts.set_child(Some(&editor.list));
    content.append(&shortcuts);
    window.set_child(Some(&content));

    rebuild_rows(editor.as_ref());
}

fn wire_shortcuts(editor: Rc<Editor>, window: &Window) {
    let list = editor.list.clone();
    let selected = Rc::clone(&editor);
    list.connect_row_selected(move |_, row| {
        *selected.selected.borrow_mut() = row
            .map(|row| row.index())
            .filter(|index| *index >= 0)
            .and_then(|index| usize::try_from(index).ok());
    });

    let activated = Rc::clone(&editor);
    list.connect_row_activated(move |_, _row| {
        if let Some(action) = current_action(activated.as_ref()) {
            activated
                .hint
                .set_text(&format!("Press a combination for {}.", action.label()));
        }
    });

    let keys = EventControllerKey::new();
    let key_editor = Rc::clone(&editor);
    keys.connect_key_pressed(move |_controller, key, _code, state| {
        let Some(action) = current_action(key_editor.as_ref()) else {
            return Propagation::Proceed;
        };
        if matches!(key, gdk::Key::Delete | gdk::Key::BackSpace) {
            key_editor.keyboard.borrow_mut().set_binding(action, None);
            rebuild_rows(key_editor.as_ref());
            key_editor
                .hint
                .set_text(&format!("{}: unset.", action.label()));
            return Propagation::Stop;
        }
        let Some(shortcut) = shortcut_for(key, state) else {
            return Propagation::Proceed;
        };
        let existing = key_editor.keyboard.borrow().action_for(&shortcut);
        key_editor
            .keyboard
            .borrow_mut()
            .set_binding(action, Some(shortcut.clone()));
        rebuild_rows(key_editor.as_ref());
        key_editor.hint.set_text(&match existing {
            Some(other) if other != action => format!(
                "{action:?} is now {}. That combination already belonged to {}.",
                shortcut.to_label(),
                other.label()
            ),
            _ => format!("{}: {}.", action.label(), shortcut.to_label()),
        });
        Propagation::Stop
    });
    window.add_controller(keys);
}

fn current_action(editor: &Editor) -> Option<crate::app::actions::Action> {
    let index = (*editor.selected.borrow())?;
    editor.actions.borrow().get(index).copied()
}

fn rebuild_rows(editor: &Editor) {
    let list = &editor.list;
    while let Some(row) = list.first_child() {
        list.remove(&row);
    }
    *editor.selected.borrow_mut() = None;
    let mut actions = Vec::new();
    let keyboard = editor.keyboard.borrow().clone();
    for (action, shortcut) in keyboard.display_rows() {
        let text = shortcut.unwrap_or_else(|| "unset".to_string());
        let row = gtk::ListBoxRow::builder()
            .activatable(true)
            .child(&row_child(action.label(), &text))
            .build();
        list.append(&row);
        actions.push(action);
    }
    *editor.actions.borrow_mut() = actions;
}

fn row_child(action_label: &str, shortcut: &str) -> gtk::Box {
    let row = widgets::hstack(12);
    row.set_margin_top(4);
    row.set_margin_bottom(4);
    let name = widgets::text_label(action_label, Align::Start);
    name.set_hexpand(true);
    let key = widgets::text_label(shortcut, Align::End);
    key.set_width_chars(20);
    row.append(&name);
    row.append(&key);
    row
}

fn grid() -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(6);
    grid.set_margin_bottom(6);
    grid
}

fn switch_row(switch: &gtk::Switch, text: &str) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    switch.set_halign(Align::Start);
    switch.set_valign(Align::Center);
    row.append(switch);
    let caption = gtk::Label::new(Some(text));
    caption.set_xalign(0.0);
    row.append(&caption);
    row.upcast()
}

/// The shortcut a key press describes, or `None` for keys that cannot be named.
fn shortcut_for(key: gdk::Key, state: gdk::ModifierType) -> Option<Shortcut> {
    use crate::settings::shortcut::Modifiers;

    let character = key.to_unicode()?;
    if character.is_control() {
        return None;
    }
    let mut modifiers = Modifiers::default();
    // A bare letter would swallow typing, so shortcuts always carry a modifier.
    modifiers.insert(Modifiers::CONTROL);
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        modifiers.insert(Modifiers::SHIFT);
    }
    if state.contains(gdk::ModifierType::ALT_MASK) {
        modifiers.insert(Modifiers::ALT);
    }
    if state.contains(gdk::ModifierType::SUPER_MASK) {
        modifiers.insert(Modifiers::SUPER);
    }
    let name = if state.contains(gdk::ModifierType::SHIFT_MASK) {
        character.to_lowercase().to_string()
    } else {
        character.to_uppercase().to_string()
    };
    Shortcut::new(modifiers, name).ok()
}

fn label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_halign(Align::Start);
    label
}
