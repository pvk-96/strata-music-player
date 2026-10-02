//! The main window: sidebar, views, context menu, player bar, and the glue between GTK and
//! [`AppCore`].
//!
//! Every GTK callback reaches the core through one [`Shared`] handle, so the core state and
//! the widgets that mirror it have a single owner.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::Orientation;

use crate::app::actions::{Action, ActionTarget, ALL_ACTIONS};
use crate::app::core::{AppCore, UiRequest};
use crate::app::navigation::{Section, View};
use crate::app::state::format_total;
use crate::app::ui::accels::accelerator;
use crate::app::ui::collection::CollectionView;
use crate::app::ui::player_bar::PlayerBar;
use crate::app::ui::preferences::Preferences;
use crate::app::ui::sidebar::Sidebar;
use crate::app::ui::track_list::TrackList;
use crate::app::ui::widgets;
use crate::library::scanner::ScanKind;
use crate::playback::state::PlaybackState;
use crate::settings::model::Settings;

/// How often the core is pumped and the interface redrawn.
const TICK_MS: u32 = 200;

/// State shared by every GTK callback.
pub struct Shared {
    window: gtk::ApplicationWindow,
    core: RefCell<AppCore>,
    sidebar: Rc<Sidebar>,
    sidebar_revealer: gtk::Revealer,
    tracks: Rc<RefCell<TrackList>>,
    collection: Rc<RefCell<CollectionView>>,
    player_bar: Rc<PlayerBar>,
    search: gtk::SearchEntry,
    title: gtk::Label,
    subtitle: gtk::Label,
    back: gtk::Button,
    forward: gtk::Button,
    sidebar_toggle: gtk::ToggleButton,
    stack: gtk::Stack,
    status: gtk::Label,
    preferences: RefCell<Option<Preferences>>,
    scan_was_running: RefCell<bool>,
    tick: RefCell<Option<glib::SourceId>>,
    /// The view to return to when the search entry is cleared.
    view_before_search: RefCell<View>,
}

/// The main window.
pub struct Window {
    shared: Rc<Shared>,
}

impl Window {
    /// Build the window for an application.
    pub fn new(app: &gtk::Application, core: AppCore) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Strata")
            .default_width(1100)
            .default_height(720)
            .build();
        let start_view = core.navigation().current().clone();

        let shared = Rc::new(Shared {
            window,
            core: RefCell::new(core),
            sidebar: Rc::new(Sidebar::new()),
            sidebar_revealer: gtk::Revealer::new(),
            tracks: Rc::new(RefCell::new(TrackList::new())),
            collection: Rc::new(RefCell::new(CollectionView::new())),
            player_bar: Rc::new(PlayerBar::new()),
            search: gtk::SearchEntry::new(),
            title: gtk::Label::new(None),
            subtitle: gtk::Label::new(None),
            back: widgets::icon_button("go-previous-symbolic", "Back"),
            forward: widgets::icon_button("go-next-symbolic", "Forward"),
            sidebar_toggle: widgets::toggle_button("sidebar-show-symbolic", "Toggle sidebar"),
            stack: gtk::Stack::new(),
            status: widgets::hint_label(""),
            preferences: RefCell::new(None),
            scan_was_running: RefCell::new(false),
            tick: RefCell::new(None),
            view_before_search: RefCell::new(start_view),
        });

        build_layout(shared.clone());
        install_actions(shared.clone(), app);
        shared.install_accelerators(app);
        wire_sidebar(shared.clone());
        wire_collection(shared.clone());
        wire_tracks(shared.clone());
        wire_player_bar(shared.clone());
        wire_search(shared.clone());
        wire_navigation_buttons(shared.clone());
        wire_sidebar_toggle(shared.clone());
        wire_keys(shared.clone());
        wire_context_menu(shared.clone());

        let closing = Rc::clone(&shared);
        shared.window.connect_close_request(move |_| {
            closing.core.borrow_mut().save_session();
            gtk::glib::Propagation::Proceed
        });
        let mapped = Rc::clone(&shared);
        shared.window.connect_map(move |_| {
            start_timer(mapped.clone());
            refresh(&mapped);
        });
        let unmapped = Rc::clone(&shared);
        shared
            .window
            .connect_unmap(move |_| stop_timer(unmapped.as_ref()));

        Rc::new(Self { shared })
    }

    pub fn window(&self) -> &gtk::ApplicationWindow {
        &self.shared.window
    }

    /// Show the window, restoring the previous session if that is enabled.
    pub fn present(&self) {
        {
            let mut core = self.shared.core.borrow_mut();
            if core.settings().playback.resume_session {
                core.restore_session();
            }
            let sidebar_visible = core.settings().appearance.sidebar_visible;
            core.set_sidebar_visible(sidebar_visible);
            core.revalidate_playlists();
        }
        self.shared.window.present();
        refresh(&self.shared);
    }
}

fn build_layout(shared: Rc<Shared>) {
    let window = &shared.window;
    let header = gtk::HeaderBar::new();
    header.set_title_widget(Some(&header_titles(shared.as_ref())));
    header.pack_start(&shared.back);
    header.pack_start(&shared.forward);
    shared
        .sidebar_toggle
        .set_active(shared.core.borrow().sidebar_visible());
    header.pack_start(&shared.sidebar_toggle);

    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Main menu")
        .menu_model(&main_menu())
        .primary(true)
        .build();
    header.pack_end(&menu_button);

    shared.search.set_hexpand(true);
    shared.search.set_placeholder_text(Some("Search tracks"));
    shared.search.set_width_chars(22);
    header.pack_end(&shared.search);
    window.set_titlebar(Some(&header));

    let paned = gtk::Paned::builder()
        .orientation(Orientation::Horizontal)
        .shrink_start_child(false)
        .resize_start_child(false)
        .build();
    shared
        .sidebar_revealer
        .set_child(Some(shared.sidebar.widget()));
    shared
        .sidebar_revealer
        .set_transition_type(gtk::RevealerTransitionType::SlideLeft);
    shared
        .sidebar_revealer
        .set_reveal_child(shared.core.borrow().sidebar_visible());
    paned.set_start_child(Some(&shared.sidebar_revealer));
    paned.set_position(260);

    shared
        .stack
        .add_named(shared.collection.borrow().widget(), Some("collection"));
    let tracks_box = gtk::Box::new(Orientation::Vertical, 0);
    tracks_box.append(shared.tracks.borrow().header());
    tracks_box.append(shared.tracks.borrow().widget());
    shared.stack.add_named(&tracks_box, Some("tracks"));
    shared.stack.set_vexpand(true);
    let content = gtk::Box::new(Orientation::Vertical, 0);
    content.append(&shared.stack);
    paned.set_end_child(Some(&content));
    paned.set_vexpand(true);

    let footer = gtk::Box::new(Orientation::Vertical, 0);
    shared.status.set_xalign(0.0);
    shared.status.set_margin_top(2);
    shared.status.set_margin_bottom(2);
    shared.status.set_margin_start(8);
    shared.status.set_margin_end(8);
    footer.append(&shared.status);
    footer.append(shared.player_bar.widget());

    let root = gtk::Box::new(Orientation::Vertical, 0);
    root.append(&paned);
    root.append(&footer);
    window.set_child(Some(&root));
}

/// Menu layout: `(section index, section label, actions)`. Section `0` is the untitled
/// group of playback commands.
const MENU_LAYOUT: &[(u8, &str, &[&str])] = &[
    (
        0,
        "",
        &[
            "play_pause",
            "previous_track",
            "next_track",
            "toggle_shuffle",
            "cycle_repeat",
        ],
    ),
    (
        1,
        "Library",
        &[
            "go_to_tracks",
            "go_to_artists",
            "go_to_albums",
            "go_to_folders",
            "go_to_favorites",
            "go_to_playlists",
        ],
    ),
    (
        2,
        "Selection",
        &[
            "select_all",
            "add_to_queue",
            "play_next",
            "add_to_playlist",
            "toggle_favorite",
        ],
    ),
    (
        3,
        "Library Management",
        &[
            "add_library_folder",
            "rescan_library",
            "rescan_folder",
            "new_playlist",
            "import_playlist",
        ],
    ),
    (
        4,
        "Track",
        &["show_track_info", "copy_path", "show_in_file_manager"],
    ),
    (
        5,
        "Playback",
        &[
            "seek_backward",
            "seek_forward",
            "volume_up",
            "volume_down",
            "mute",
            "toggle_sidebar",
        ],
    ),
    (6, "Strata", &["open_settings", "save_playlist", "quit"]),
];

fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    for (section, label, actions) in MENU_LAYOUT {
        let group = gio::Menu::new();
        for name in *actions {
            group.append(Some(label_for(name)), Some(&format!("win.{name}")));
        }
        if *section == 0 {
            menu.append_section(None, &group);
        } else {
            menu.append_section(Some(label), &group);
        }
    }
    menu
}

fn label_for(name: &str) -> &'static str {
    ALL_ACTIONS
        .iter()
        .find(|action| action.name() == name)
        .map(|action| action.label())
        .unwrap_or("")
}

fn header_titles(shared: &Shared) -> gtk::Box {
    let titles = gtk::Box::new(Orientation::Vertical, 0);
    shared.title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    shared.title.add_css_class("title");
    shared
        .subtitle
        .set_ellipsize(gtk::pango::EllipsizeMode::End);
    shared.subtitle.add_css_class("subtitle");
    shared.subtitle.add_css_class("dim-label");
    titles.append(&shared.title);
    titles.append(&shared.subtitle);
    titles
}

/// Install one `GAction` per Strata action, so menus, buttons and shortcuts share one path.
fn install_actions(shared: Rc<Shared>, app: &gtk::Application) {
    let window = shared.window.clone();
    for action in ALL_ACTIONS {
        let simple = gio::SimpleAction::new(action.name(), None);
        let action_shared = Rc::clone(&shared);
        let dispatched = *action;
        simple.connect_activate(move |_, _| {
            perform(&action_shared, dispatched);
        });
        window.add_action(&simple);
    }
    for name in ["rename_playlist", "delete_playlist"] {
        let simple = gio::SimpleAction::new(name, None);
        let action_shared = Rc::clone(&shared);
        simple.connect_activate(move |_, _| {
            let Some(playlist_id) = action_shared.sidebar.selected_playlist() else {
                return;
            };
            if name == "rename_playlist" {
                rename_playlist(Rc::clone(&action_shared), playlist_id);
            } else {
                delete_playlist(Rc::clone(&action_shared), playlist_id);
            }
        });
        window.add_action(&simple);
    }
    let quit = gio::SimpleAction::new("quit", None);
    let quit_window = shared.window.clone();
    quit.connect_activate(move |_, _| quit_window.close());
    app.add_action(&quit);
}

impl Shared {
    /// Window accelerators belong to the application, which owns the `win.*` actions. They
    /// are re-installed whenever the shortcuts change.
    fn install_accelerators(&self, app: &gtk::Application) {
        let keyboard = self.core.borrow().keyboard().clone();
        for action in ALL_ACTIONS {
            let accels: Vec<String> = keyboard
                .binding(*action)
                .map(|shortcut| vec![accelerator(shortcut)])
                .unwrap_or_default();
            let refs: Vec<&str> = accels.iter().map(String::as_str).collect();
            // `quit` lives on the application, every other action on the window.
            let action = match *action {
                Action::Quit => "app.quit",
                action => &*format!("win.{}", action.name()),
            };
            app.set_accels_for_action(action, &refs);
        }
    }
}

// ----- wiring -----------------------------------------------------------------

fn wire_sidebar(shared: Rc<Shared>) {
    let sidebar = Rc::clone(&shared.sidebar);
    let state = Rc::clone(&shared);
    sidebar.connect_section_activate(move |section| {
        state.core.borrow_mut().go_to_section(section);
        refresh(&state);
    });

    let playlists = Rc::clone(&shared.sidebar);
    let state = Rc::clone(&shared);
    playlists.connect_playlist_activate(move |id| {
        state.core.borrow_mut().go_to(View::Playlist(id));
        refresh(&state);
    });

    let menu = Rc::clone(&shared.sidebar);
    menu.connect_playlist_menu(move |id| show_playlist_menu(shared.clone(), id));
}

fn wire_collection(shared: Rc<Shared>) {
    let collection = Rc::clone(&shared.collection);
    let state = Rc::clone(&shared);
    collection.borrow().connect_activate(move |view| {
        state.core.borrow_mut().go_to(view);
        refresh(&state);
    });
}

fn wire_tracks(shared: Rc<Shared>) {
    let tracks = Rc::clone(&shared.tracks);
    let state = Rc::clone(&shared);
    tracks.borrow().connect_activate(move |track_id| {
        let mut core = state.core.borrow_mut();
        if let Err(error) = core.play_track(track_id) {
            core.report(&error);
        }
        drop(core);
        refresh(&state);
    });

    let list = Rc::clone(&shared.tracks);
    let state = Rc::clone(&shared);
    list.borrow().connect_activate_selection(move |track_ids| {
        let mut core = state.core.borrow_mut();
        match track_ids.len() {
            // One row plays; a multi-row selection is a queue request.
            0 => {}
            1 => {
                if let Err(error) = core.play_track(track_ids[0]) {
                    core.report(&error);
                }
            }
            _ => {
                core.add_to_queue(&track_ids);
                core.notify(format!("Queued {} tracks.", track_ids.len()));
            }
        }
        drop(core);
        refresh(&state);
    });

    let selected = Rc::clone(&shared.tracks);
    let state = Rc::clone(&shared);
    selected.borrow().connect_selection_changed(move |indexes| {
        let mut core = state.core.borrow_mut();
        core.tracks_mut().selected = indexes.clone();
        core.tracks_mut().anchor = indexes.first().copied();
    });

    let reordered = Rc::clone(&shared.tracks);
    let state = Rc::clone(&shared);
    reordered.borrow().connect_reordered(move |from, to| {
        let mut core = state.core.borrow_mut();
        if matches!(core.navigation().current(), View::Queue) {
            core.reorder_queue(from, to);
            core.reload_view();
        } else {
            core.warn("Tracks can only be reordered in the queue view.");
        }
        drop(core);
        refresh(&state);
    });

    wire_sort_header(&shared);
}

/// Clicking a column heading sorts by it.
fn wire_sort_header(shared: &Rc<Shared>) {
    let header = shared.tracks.borrow().header().clone();
    let mut child = header.first_child();
    let mut column = 0usize;
    while let Some(widget) = child {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
            let state = Rc::clone(shared);
            let index = column;
            button.connect_clicked(move |_| {
                if let Some(column) = column_for(index) {
                    let mut core = state.core.borrow_mut();
                    core.tracks_mut().sort_by(column);
                    core.reload_view();
                    drop(core);
                    refresh(&state);
                }
            });
            column += 1;
        }
        child = widget.next_sibling();
    }
}

/// Header column to sort by, if that column can sort.
fn column_for(index: usize) -> Option<crate::library::model::TrackSort> {
    use crate::library::model::TrackSort;
    match index {
        0 => Some(TrackSort::TrackNumber),
        1 => Some(TrackSort::Title),
        2 => Some(TrackSort::Artist),
        3 => Some(TrackSort::Album),
        4 => Some(TrackSort::Duration),
        _ => None,
    }
}

fn wire_player_bar(shared: Rc<Shared>) {
    let bar = Rc::clone(&shared.player_bar);
    let state = Rc::clone(&shared);
    bar.connect_play(move || run(&state, Action::PlayPause));

    let previous = Rc::clone(&shared.player_bar);
    let state = Rc::clone(&shared);
    previous.connect_previous(move || run(&state, Action::PreviousTrack));

    let next = Rc::clone(&shared.player_bar);
    let state = Rc::clone(&shared);
    next.connect_next(move || run(&state, Action::NextTrack));

    let shuffle = Rc::clone(&shared.player_bar);
    let state = Rc::clone(&shared);
    shuffle.connect_toggle_shuffle(move || run(&state, Action::ToggleShuffle));

    let repeat = Rc::clone(&shared.player_bar);
    let state = Rc::clone(&shared);
    repeat.connect_cycle_repeat(move || run(&state, Action::CycleRepeat));

    let mute = Rc::clone(&shared.player_bar);
    let state = Rc::clone(&shared);
    mute.connect_toggle_mute(move || run(&state, Action::Mute));

    let queue = Rc::clone(&shared.player_bar);
    let state = Rc::clone(&shared);
    queue.connect_show_queue(move || {
        state.core.borrow_mut().go_to_section(Section::Queue);
        refresh(&state);
    });

    let seeking = Rc::clone(&shared.player_bar);
    let state = Rc::clone(&shared);
    seeking.connect_seek(move |seconds| {
        state.core.borrow_mut().seek_to(seconds_to_ms(seconds));
    });

    let volume = Rc::clone(&shared.player_bar);
    let state = Rc::clone(&shared);
    volume.connect_volume(move |percent| {
        state.core.borrow_mut().set_volume(percent / 100.0);
    });
}

fn wire_search(shared: Rc<Shared>) {
    let state = Rc::clone(&shared);
    shared.search.connect_search_changed(move |entry| {
        let term = entry.text().trim().to_string();
        let mut core = state.core.borrow_mut();
        let searching = matches!(core.navigation().current(), View::Search(_));
        if searching && term.is_empty() {
            core.clear_search();
            let previous = state.view_before_search.borrow().clone();
            core.go_to(previous);
        } else {
            if !searching {
                *state.view_before_search.borrow_mut() = core.navigation().current().clone();
            }
            core.set_search_term(&term);
            if term.is_empty() {
                core.clear_search();
            } else {
                core.apply_search();
                core.go_to(View::Search(term));
            }
        }
        drop(core);
        refresh(&state);
    });
}

fn wire_navigation_buttons(shared: Rc<Shared>) {
    let back = shared.back.clone();
    let state = Rc::clone(&shared);
    back.connect_clicked(move |_| {
        state.core.borrow_mut().back();
        refresh(&state);
    });

    let forward = shared.forward.clone();
    let state = Rc::clone(&shared);
    forward.connect_clicked(move |_| {
        state.core.borrow_mut().forward();
        refresh(&state);
    });
}

fn wire_sidebar_toggle(shared: Rc<Shared>) {
    let state = Rc::clone(&shared);
    shared.sidebar_toggle.connect_toggled(move |button| {
        let visible = button.is_active();
        state.core.borrow_mut().set_sidebar_visible(visible);
        state.sidebar_revealer.set_reveal_child(visible);
    });
}

/// Arrow keys move the selection, Return plays it.
fn wire_keys(shared: Rc<Shared>) {
    let keys = gtk::EventControllerKey::new();
    let state = Rc::clone(&shared);
    keys.connect_key_pressed(move |_, key, _, _| {
        let mut core = state.core.borrow_mut();
        match key {
            gdk::Key::Down => core.tracks_mut().select_next(),
            gdk::Key::Up => core.tracks_mut().select_previous(),
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => {
                drop(core);
                state.tracks.borrow().activate_selected();
                refresh(&state);
                return gtk::glib::Propagation::Stop;
            }
            _ => return gtk::glib::Propagation::Proceed,
        }
        let selected = core.tracks().selected.clone();
        drop(core);
        state.tracks.borrow().set_selected(&selected);
        if let Some(index) = selected.first() {
            state.tracks.borrow().scroll_to(*index);
        }
        gtk::glib::Propagation::Stop
    });
    shared.window.add_controller(keys);
}

/// Right-clicking a track offers the selection commands.
fn wire_context_menu(shared: Rc<Shared>) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    let list = shared.tracks.borrow().list().clone();
    let menu_list = list.clone();
    let state = Rc::clone(&shared);
    gesture.connect_pressed(move |_gesture, _button, x, y| {
        let Some(index) = row_under(TRACK_ROW_HEIGHT, y) else {
            return;
        };
        let track_id = menu_list
            .model()
            .and_then(|model| model.item(index))
            .and_then(|item| {
                item.downcast::<crate::app::ui::track_object::TrackObject>()
                    .ok()
            })
            .map(|object| object.track_id());
        if track_id.is_none() {
            return;
        }
        {
            let mut core = state.core.borrow_mut();
            core.tracks_mut().select_only(index as usize);
        }
        state.tracks.borrow().set_selected(&[index as usize]);

        let popover = gtk::PopoverMenu::from_model(Some(&track_menu()));
        popover.set_parent(&menu_list);
        popover.set_has_arrow(false);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    });
    list.add_controller(gesture);
}

/// Row under a pointer position. The track list uses a fixed row height, so the position
/// maps straight onto a row index.
fn row_under(height: f64, y: f64) -> Option<u32> {
    if y < 0.0 {
        return None;
    }
    Some((y / height.max(1.0)).floor() as u32)
}

/// Height of one row in the track list.
pub const TRACK_ROW_HEIGHT: f64 = 44.0;

fn track_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    for name in [
        "add_to_queue",
        "play_next",
        "add_to_playlist",
        "toggle_favorite",
        "select_all",
    ] {
        menu.append(Some(label_for(name)), Some(&format!("win.{name}")));
    }
    let second = gio::Menu::new();
    second.append(Some("Copy Path"), Some("win.copy_path"));
    second.append(Some("Track Information"), Some("win.show_track_info"));
    second.append(
        Some("Show in File Manager"),
        Some("win.show_in_file_manager"),
    );
    menu.append_section(None, &second);
    menu
}

/// Pop up the playlist menu on the row the user right-clicked.
fn show_playlist_menu(shared: Rc<Shared>, _playlist_id: i64) {
    let Some(row) = shared.sidebar.selected_playlist_row() else {
        return;
    };
    let menu = gio::Menu::new();
    menu.append(Some("_Rename Playlist"), Some("win.rename_playlist"));
    menu.append(Some("_Delete Playlist"), Some("win.delete_playlist"));
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(&row);
    popover.set_has_arrow(false);
    popover.popup();
}

fn rename_playlist(shared: Rc<Shared>, playlist_id: i64) {
    let current = shared.core.borrow().playlist_name(playlist_id);
    let state = Rc::clone(&shared);
    widgets::text_prompt(
        Some(shared.window.upcast_ref()),
        "Rename playlist",
        "Playlist name",
        &current,
        move |name| {
            if name.is_empty() {
                return;
            }
            {
                let mut core = state.core.borrow_mut();
                match core.rename_playlist(playlist_id, &name) {
                    Ok(()) => core.notify(format!("Renamed to {name}.")),
                    Err(error) => core.report(&error),
                }
            }
            refresh(&state);
        },
    );
}

fn delete_playlist(shared: Rc<Shared>, playlist_id: i64) {
    let name = shared.core.borrow().playlist_name(playlist_id);
    let state = Rc::clone(&shared);
    widgets::confirm_dialog(
        Some(shared.window.upcast_ref()),
        "Delete playlist",
        &format!("Delete the playlist \"{name}\"? Tracks are not removed from the library."),
        move |confirmed| {
            if !confirmed {
                return;
            }
            {
                let mut core = state.core.borrow_mut();
                match core.delete_playlist(playlist_id) {
                    Ok(()) => {
                        core.go_to_section(Section::Playlists);
                        core.notify(format!("Deleted {name}."));
                    }
                    Err(error) => core.report(&error),
                }
            }
            refresh(&state);
        },
    );
}

// ----- running ----------------------------------------------------------------

/// Run an action through the core, then redraw.
pub fn run(shared: &Rc<Shared>, action: Action) {
    shared.core.borrow_mut().perform(action);
    refresh(shared);
}

fn perform(shared: &Rc<Shared>, action: Action) {
    match action {
        Action::FocusSearch => {
            shared.search.grab_focus();
        }
        Action::CopyPath => {
            let text = {
                let core = shared.core.borrow();
                core.copy_paths_text(&core.selected_track_ids())
            };
            if text.is_empty() {
                shared.core.borrow_mut().warn("Select a track first.");
            } else {
                widgets::copy_to_clipboard(&text);
                shared
                    .core
                    .borrow_mut()
                    .notify(format!("{} path(s) copied.", text.lines().count()));
            }
        }
        Action::ToggleSidebar => {
            let visible = !shared.core.borrow().sidebar_visible();
            shared.core.borrow_mut().set_sidebar_visible(visible);
            shared.sidebar_revealer.set_reveal_child(visible);
            shared.sidebar_toggle.set_active(visible);
        }
        Action::Quit => shared.window.close(),
        _ => shared.core.borrow_mut().perform(action),
    }
    refresh(shared);
}

fn start_timer(shared: Rc<Shared>) {
    if shared.tick.borrow().is_some() {
        return;
    }
    let state = Rc::clone(&shared);
    let source = glib::timeout_add_local(
        std::time::Duration::from_millis(u64::from(TICK_MS)),
        move || {
            tick(&state);
            glib::ControlFlow::Continue
        },
    );
    *shared.tick.borrow_mut() = Some(source);
}

fn stop_timer(shared: &Shared) {
    if let Some(source) = shared.tick.borrow_mut().take() {
        source.remove();
    }
}

/// Periodic work: pump playback and the scanner, then redraw.
fn tick(shared: &Rc<Shared>) {
    let scan_finished = {
        let mut core = shared.core.borrow_mut();
        core.pump_playback();
        core.pump_scan()
    };
    let was_running = *shared.scan_was_running.borrow();
    let running = shared.core.borrow().scan_running();
    if scan_finished || (was_running && !running) {
        shared.core.borrow_mut().reload_view();
        *shared.scan_was_running.borrow_mut() = false;
    } else {
        *shared.scan_was_running.borrow_mut() = running;
    }
    refresh(shared);
}

/// Redraw everything from the core.
fn refresh(shared: &Rc<Shared>) {
    let view = shared.core.borrow().navigation().current().clone();
    let stats = shared.core.borrow().library_stats();
    let count = shared.core.borrow().tracks().len();
    let total_ms = shared.core.borrow().tracks().total_duration_ms();
    shared.title.set_text(&view.title());
    shared.subtitle.set_text(&match (&stats, count) {
        (Some(stats), 0) => library_totals(stats),
        (_, 0) => String::new(),
        _ => format!("{count} tracks · {}", format_total(total_ms)),
    });

    let (can_back, can_forward) = {
        let core = shared.core.borrow();
        (
            core.navigation().can_go_back(),
            core.navigation().can_go_forward(),
        )
    };
    shared.back.set_sensitive(can_back);
    shared.forward.set_sensitive(can_forward);

    // ----- views -----------------------------------------------------------
    if view.shows_tracks() {
        shared.stack.set_visible_child_name("tracks");
        let rows = shared.core.borrow().tracks().rows.clone();
        let selected = shared.core.borrow().tracks().selected.clone();
        let core = shared.core.borrow();
        shared.tracks.borrow_mut().set_rows(&rows);
        shared.tracks.borrow().set_selected(&selected);
        shared.tracks.borrow().request_artwork(&core);
    } else {
        shared.stack.set_visible_child_name("collection");
        let core = shared.core.borrow();
        let artists = core.artists();
        let albums = core.albums();
        let folders = core.folders();
        let playlists = core.playlists();
        shared
            .collection
            .borrow_mut()
            .set_view(&view, &artists, &albums, &folders, &playlists);
    }

    let playlists = shared.core.borrow().playlists();
    shared.sidebar.set_playlists(&playlists);
    shared.sidebar.select(&view);
    if let Some(stats) = stats {
        shared.sidebar.set_summary(library_totals(&stats));
    }

    // ----- player ----------------------------------------------------------
    {
        let core = shared.core.borrow();
        let current_id = core.sequencer().current_track_id();
        let track = current_id.and_then(|id| core.track(id));
        match &track {
            Some(track) => shared
                .player_bar
                .set_now_playing(&track.title, &track.artist),
            None => shared.player_bar.set_now_playing("", ""),
        }
        shared
            .player_bar
            .set_playing(matches!(core.playback_state(), PlaybackState::Playing));
        shared.player_bar.set_shuffle(core.shuffle_enabled());
        shared.player_bar.set_repeat(core.repeat_mode());
        shared.player_bar.set_volume(core.volume(), core.is_muted());
        let position = core.position_hint();
        let duration = track.as_ref().and_then(|track| track.duration_ms);
        shared
            .player_bar
            .set_position(position, duration.filter(|value| *value > 0));
    }

    // ----- status ----------------------------------------------------------
    let status = {
        let core = shared.core.borrow();
        status_text(&core, count)
    };
    shared.status.set_text(&status);
    let sidebar_visible = shared.core.borrow().sidebar_visible();
    shared.sidebar_revealer.set_reveal_child(sidebar_visible);
    shared.sidebar_toggle.set_active(sidebar_visible);

    // ----- requests -------------------------------------------------------
    let requests = shared.core.borrow_mut().take_requests();
    for request in requests {
        handle_request(shared, request);
    }
}

fn library_totals(stats: &crate::library::queries::LibraryStats) -> String {
    format!(
        "{} tracks\n{} albums\n{} artists",
        stats.tracks, stats.albums, stats.artists
    )
}

fn status_text(core: &AppCore, count: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    let progress = core.scan_progress();
    if progress.running {
        let name = progress
            .current
            .as_deref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        parts.push(if progress.total > 0 {
            format!("Scanning {}/{} {name}", progress.processed, progress.total)
        } else {
            format!("Scanning {} {name}", progress.processed)
        });
    }
    if core.player().is_none() {
        parts.push("No audio output available".to_string());
    }
    if parts.is_empty() && count > 0 {
        parts.push(core.tracks().summary());
    }
    parts.join("   ")
}

fn handle_request(shared: &Rc<Shared>, request: UiRequest) {
    let window: &gtk::Window = shared.window.upcast_ref();
    match request {
        UiRequest::RefreshView | UiRequest::RefreshSidebar | UiRequest::RefreshPlayerBar => {
            refresh(shared);
        }
        UiRequest::Notify { message, warning } => {
            shared.status.set_text(&message);
            if warning {
                shared.status.add_css_class("warning");
            } else {
                shared.status.remove_css_class("warning");
            }
        }
        UiRequest::OpenTrackInfo(track_id) => show_track_info(shared, track_id),
        UiRequest::AskToChooseFolder => {
            let chooser_shared = Rc::clone(shared);
            widgets::choose_folder(Some(window), None, move |path| {
                if let Some(path) = path {
                    let mut core = chooser_shared.core.borrow_mut();
                    match core.add_library_folder(&path) {
                        Ok(()) => core.start_scan(ScanKind::AllFolders),
                        Err(error) => core.report(&error),
                    }
                    drop(core);
                    refresh(&chooser_shared);
                }
            });
        }
        UiRequest::AskToOpenSettings => open_settings(shared),
        UiRequest::AskToImportPlaylist => {
            let import_shared = Rc::clone(shared);
            widgets::choose_file(
                Some(window),
                None,
                vec!["m3u".to_string(), "m3u8".to_string(), "xspf".to_string()],
                move |path| {
                    if let Some(path) = path {
                        let name = path
                            .file_stem()
                            .map(|stem| stem.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "Imported playlist".to_string());
                        let mut core = import_shared.core.borrow_mut();
                        match core.import_playlist(&path, &name) {
                            Ok(()) => core.notify(format!("Imported {name}.")),
                            Err(error) => core.report(&error),
                        }
                        drop(core);
                        refresh(&import_shared);
                    }
                },
            );
        }
        UiRequest::AskToSavePlaylist => {
            let View::Playlist(playlist_id) = shared.core.borrow().navigation().current().clone()
            else {
                return;
            };
            let name = shared.core.borrow().playlist_name(playlist_id);
            let save_shared = Rc::clone(shared);
            let suggested = format!("{name}.m3u");
            widgets::choose_save_file(
                Some(window),
                &suggested,
                None,
                move |path: Option<PathBuf>| {
                    let Some(path) = path else {
                        return;
                    };
                    let result = save_shared
                        .core
                        .borrow()
                        .export_playlist(playlist_id, &path);
                    let mut core = save_shared.core.borrow_mut();
                    match result {
                        Ok(()) => core.notify(format!("Exported to {}.", path.display())),
                        Err(error) => core.report(&error),
                    }
                    drop(core);
                    refresh(&save_shared);
                },
            );
        }
        UiRequest::AskToCreatePlaylist => create_playlist(shared),
        UiRequest::ShowInFileManager(path) => {
            if let Err(error) = widgets::show_in_file_manager(&path) {
                shared.core.borrow_mut().warn(error);
            }
        }
        UiRequest::SaveSession => shared.core.borrow_mut().save_session(),
    }
}

fn create_playlist(shared: &Rc<Shared>) {
    let create_shared = Rc::clone(shared);
    widgets::text_prompt(
        Some(shared.window.upcast_ref()),
        "New playlist",
        "Playlist name",
        "",
        move |name| {
            if name.is_empty() {
                return;
            }
            let mut core = create_shared.core.borrow_mut();
            match core.create_playlist(&name) {
                Ok(id) => {
                    core.go_to(View::Playlist(id));
                    core.notify(format!("Created {name}."));
                }
                Err(error) => core.report(&error),
            }
            drop(core);
            refresh(&create_shared);
        },
    );
}

fn open_settings(shared: &Rc<Shared>) {
    if let Some(preferences) = shared.preferences.borrow().as_ref() {
        preferences.show();
        return;
    }
    let settings = shared.core.borrow().settings().clone();
    let apply_shared = Rc::clone(shared);
    let on_apply: preferences_apply::Callback = Rc::new(move |settings: Settings| {
        let path = apply_shared.core.borrow().paths().settings_file().clone();
        let saved = crate::settings::save(&path, &settings);
        apply_shared.core.borrow_mut().apply_settings(settings);
        if let Err(error) = saved {
            apply_shared.core.borrow_mut().report(&error);
        }
        if let Some(app) = apply_shared.window.application() {
            apply_shared.install_accelerators(&app);
        }
        refresh(&apply_shared);
    });
    *shared.preferences.borrow_mut() = Some(Preferences::new(
        Some(shared.window.upcast_ref()),
        &settings,
        on_apply,
    ));
}

mod preferences_apply {
    pub use crate::app::ui::preferences::ApplyCallback as Callback;
}

fn show_track_info(shared: &Rc<Shared>, track_id: i64) {
    let Some(track) = shared.core.borrow().track(track_id) else {
        return;
    };
    widgets::message_dialog(
        Some(shared.window.upcast_ref()),
        "Track information",
        &crate::app::state::track_information(&track),
        |_| {},
    );
}

/// Seconds on the seek bar to milliseconds of playback.
fn seconds_to_ms(seconds: f64) -> i64 {
    (seconds * 1000.0).round() as i64
}
