//! Starting Strata: paths, database, settings, and the GTK application.
//!
//! Everything that can fail without a display is handled before GTK is initialised, so a
//! bad configuration file produces a clear message instead of an empty window.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::channel;

use gtk::glib;
use gtk::prelude::*;

use crate::app::core::{AppCore, Paths};
use crate::app::ui::window::Window;
use crate::database::Database;
use crate::library::scanner::ScanKind;
use crate::playback::player::Player;

/// The application id, used for the desktop entry and the session bus name.
pub const APPLICATION_ID: &str = "dev.strata.Strata";

/// The icon name shared by the desktop entry, the AppStream metadata and the hicolor theme.
pub const ICON_NAME: &str = "dev.strata.Strata";

/// Command line options.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Options {
    /// Add this folder to the library on start.
    pub add_folder: Option<PathBuf>,
    /// Do not restore the previous session.
    pub no_session: bool,
    /// Show the version and exit.
    pub version: bool,
    /// Show the usage summary and exit.
    pub help: bool,
}

/// The usage summary, also shown on an unknown option.
pub const USAGE: &str = "\
Usage: strata [--add-folder PATH] [--no-session] [--version] [--help]

A local-first music player for your own library.

Options:
  --add-folder PATH  Add a folder to the library on start
  --no-session       Start empty instead of restoring the last session
  -v, --version      Print the version and exit
  -h, --help         Print this message and exit";
pub fn parse_options<I: IntoIterator<Item = String>>(args: I) -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = args.into_iter().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" | "-v" => options.version = true,
            "--help" | "-h" => options.help = true,
            "--no-session" => options.no_session = true,
            "--add-folder" => {
                options.add_folder = Some(
                    args.next()
                        .ok_or_else(|| "--add-folder needs a path".to_string())?
                        .into(),
                );
            }
            other => return Err(format!("Unknown option: {other}")),
        }
    }
    Ok(options)
}

/// What happened before GTK started.
pub struct Startup {
    pub core: AppCore,
    pub warnings: Vec<String>,
}

/// Build the core, reporting anything that needs fixing.
pub fn prepare(options: &Options) -> Result<Startup, String> {
    let paths = Paths::standard().map_err(|error| error.to_string())?;
    prepare_with_paths(&paths, options)
}

/// Same as [`prepare`], with explicit paths; used by tests and by `--add-folder`.
pub fn prepare_with_paths(paths: &Paths, options: &Options) -> Result<Startup, String> {
    let mut warnings = Vec::new();
    for directory in [paths.data.clone(), paths.cache.clone()] {
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("Could not create {}: {error}", directory.display()))?;
    }

    let loaded = crate::settings::load(&paths.settings_file())
        .map_err(|error| format!("Could not read the settings: {error}"))?;
    warnings.extend(loaded.warnings);
    let settings = loaded.settings;
    let settings_file = paths.settings_file();

    let database = Database::open(&paths.database_file())
        .map_err(|error| format!("Could not open the library database: {error}"))?;

    let player = match Player::new() {
        Ok(player) => Some(player),
        Err(error) => {
            // No audio output is not fatal: the library still browses and plays later.
            warnings.push(format!("Audio is unavailable: {error}"));
            None
        }
    };

    let (requests, _requests) = channel();
    let mut core = AppCore::new(paths.clone(), database, settings, player, requests);

    if let Some(folder) = &options.add_folder {
        if let Err(error) = core.add_library_folder(folder) {
            warnings.push(format!("Could not add {}: {error}", folder.display()));
        } else {
            // A newly added folder is indexed straight away, so `--add-folder` is enough to
            // get a usable library without opening the menu as well.
            core.start_scan(ScanKind::AllFolders);
        }
    }

    // Persist anything the validation had to correct.
    let _ = crate::settings::save(&settings_file, core.settings());

    Ok(Startup { core, warnings })
}

/// Run the application.
pub fn run(options: Options) -> i32 {
    if options.version {
        println!("strata {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    if options.help {
        println!("{USAGE}");
        return 0;
    }
    let startup = match prepare(&options) {
        Ok(startup) => startup,
        Err(message) => {
            eprintln!("strata: {message}");
            return 1;
        }
    };
    let warnings = startup.warnings;
    let core = startup.core;
    let application = gtk::Application::builder()
        .application_id(APPLICATION_ID)
        .flags(gtk::gio::ApplicationFlags::empty())
        .build();
    declare_options(&application);

    let holder: Rc<RefCell<Option<Rc<Window>>>> = Rc::new(RefCell::new(None));
    let open_core = RefCell::new(Some(core));
    let pending_warnings = RefCell::new(warnings);

    // `run` is the only place GTK gets initialised: `gtk::Application` initialises it from its
    // `startup`, which happens inside `run` before any handler below fires. Nothing above this
    // point may touch GTK, and everything GTK-shaped has to be built from here on.
    application.connect_activate(move |application| {
        if let Some(window) = holder.borrow().as_ref() {
            window.window().present();
            return;
        }
        use_icon_theme();
        let Some(core) = open_core.borrow_mut().take() else {
            return;
        };
        let window = Window::new(application, core);
        *holder.borrow_mut() = Some(window.clone());
        window.present();

        let messages = std::mem::take(&mut *pending_warnings.borrow_mut());
        if !messages.is_empty() {
            glib::idle_add_local_once(move || {
                crate::app::ui::widgets::message_dialog(
                    None,
                    "Strata started with warnings",
                    &messages.join("\n"),
                    |_| {},
                );
            });
        }
    });

    application.run().into()
}

/// Tell GApplication about our own options.
///
/// GLib parses the process arguments itself and refuses anything it does not know, so the
/// options `parse_options` handles have to be declared here as well. They are consumed by
/// Strata, not by GApplication.
fn declare_options(application: &gtk::Application) {
    let flags = glib::OptionFlags::NONE;
    let nothing: Option<&str> = None;
    application.add_main_option(
        "version",
        glib::Char::from(b'v'),
        flags,
        glib::OptionArg::None,
        "Print the version and exit",
        nothing,
    );
    application.add_main_option(
        "help",
        glib::Char::from(b'h'),
        flags,
        glib::OptionArg::None,
        "Print the usage summary and exit",
        nothing,
    );
    application.add_main_option(
        "no-session",
        glib::Char::from(0),
        flags,
        glib::OptionArg::None,
        "Start empty instead of restoring the last session",
        nothing,
    );
    application.add_main_option(
        "add-folder",
        glib::Char::from(0),
        flags,
        glib::OptionArg::String,
        "Add a folder to the library on start",
        nothing,
    );
}

/// Locate the repository icon theme, if the executable is a development build.
///
/// Returns the `data/icons` directory that sits above a `target/<profile>/` executable.
fn repository_icon_dir(executable: &std::path::Path) -> Option<PathBuf> {
    executable
        .ancestors()
        .skip(1)
        .map(|parent| parent.join("data").join("icons"))
        .find(|icons| icons.join("hicolor").is_dir())
}

/// Make `dev.strata.Strata` resolve for the window icon.
///
/// An installed build finds the icon through the icon theme, because every package installs
/// `data/icons/hicolor` next to the desktop entry. A binary run straight from `cargo run`
/// lives in `target/<profile>/`, so the repository's `data/icons` is added to the theme search
/// path when it sits above the executable. That keeps development honest without depending on
/// the working directory or on a path frozen at compile time.
///
/// GTK is initialised by then: this runs from `activate`, which the application only reaches
/// through [`run`]. Calling `set_default_icon_name` any earlier aborts with "GTK has not been
/// initialized", because both `Display::default` and the window icon are GTK calls.
fn use_icon_theme() {
    if let (Ok(executable), Some(display)) = (std::env::current_exe(), gtk::gdk::Display::default())
    {
        if let Some(icons) = repository_icon_dir(&executable) {
            gtk::IconTheme::for_display(&display).add_search_path(icons);
        }
    }
    gtk::Window::set_default_icon_name(ICON_NAME);
}

/// Entry point used by `main`.
pub fn main_with_args(args: Vec<String>) -> i32 {
    match parse_options(args) {
        Ok(options) => run(options),
        Err(message) => {
            eprintln!("strata: {message}");
            eprintln!("{USAGE}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(args: &[&str]) -> Result<Options, String> {
        let mut all = vec!["strata".to_string()];
        all.extend(args.iter().map(|arg| (*arg).to_string()));
        parse_options(all)
    }

    #[test]
    fn no_arguments_are_accepted() {
        assert_eq!(options(&[]).unwrap(), Options::default());
    }

    #[test]
    fn development_builds_find_the_repository_icon_theme() {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let development = repository.join("target").join("debug").join("strata");
        assert_eq!(
            repository_icon_dir(&development),
            Some(repository.join("data").join("icons"))
        );
    }

    #[test]
    fn installed_builds_rely_on_the_icon_theme() {
        assert_eq!(
            repository_icon_dir(std::path::Path::new("/usr/bin/strata")),
            None
        );
    }

    #[test]
    fn version_and_no_session_are_flags() {
        let parsed = options(&["--version", "--no-session"]).unwrap();
        assert!(!parsed.help);
        assert!(parsed.version);
        assert!(parsed.no_session);
        assert!(options(&["-v"]).unwrap().version);
        assert!(options(&["-h"]).unwrap().help);
        assert!(options(&["--help"]).unwrap().help);
        assert!(options(&["--add-folder", "/music"])
            .unwrap()
            .add_folder
            .is_some());
        assert!(options(&["--add-folder"]).is_err());
        assert!(options(&["--nope"]).is_err());
    }

    #[test]
    fn a_folder_can_be_added_on_the_command_line() {
        let parsed = options(&["--add-folder", "/music"]).unwrap();
        assert_eq!(parsed.add_folder, Some(PathBuf::from("/music")));
    }

    #[test]
    fn a_folder_without_a_path_is_rejected() {
        assert!(options(&["--add-folder"]).is_err());
    }

    #[test]
    fn unknown_options_are_rejected() {
        assert!(options(&["--teleport"]).is_err());
    }

    #[test]
    fn startup_reports_a_bad_configuration_path() {
        let paths = Paths {
            config: PathBuf::from("/proc/strata-nonexistent"),
            data: PathBuf::from("/proc/strata-nonexistent"),
            cache: PathBuf::from("/proc/strata-nonexistent"),
        };
        let error = match prepare_with_paths(&paths, &Options::default()) {
            Ok(_) => panic!("an unwritable path should not start Strata"),
            Err(error) => error,
        };
        assert!(error.contains("Could not create"));
    }
}
