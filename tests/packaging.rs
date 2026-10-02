//! Checks that the icon is wired the same way everywhere it is used.
//!
//! The desktop entry, the AppStream metadata, the hicolor theme, the Flatpak manifest and the
//! Debian and RPM packaging all have to agree on one icon name, and the shipped size family has
//! to match `scripts/generate-icons.py`. These tests need no display, no renderer and no
//! network, so they run anywhere the crate builds.

use std::path::{Path, PathBuf};

const ICON_NAME: &str = "dev.strata.Strata";
const SIZES: [u32; 9] = [16, 22, 24, 32, 48, 64, 128, 256, 512];
const ICO_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    std::fs::read_to_string(repository().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

/// The width and height stored in a PNG header, without decoding the image.
fn png_dimensions(path: &Path) -> (u32, u32) {
    let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("read {path:?}: {error}"));
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{path:?} is not a PNG");
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    (width, height)
}

#[test]
fn master_artwork_is_a_flat_vector_document() {
    let svg = read("assets/branding/strata-icon.svg");
    assert!(svg.contains("viewBox=\"0 0 1024 1024\""));
    for banned in [
        "<image",
        "<filter",
        "<linearGradient",
        "<radialGradient",
        "<text",
    ] {
        assert!(
            !svg.contains(banned),
            "master artwork must not contain {banned}"
        );
    }
    for colour in ["#1056E5", "#F3F0EE"] {
        assert!(svg.contains(colour), "master artwork must use {colour}");
    }
}

#[test]
fn the_installed_theme_ships_every_hicolor_size() {
    for size in SIZES {
        let path = repository()
            .join("data/icons/hicolor")
            .join(format!("{size}x{size}/apps/{ICON_NAME}.png"));
        assert_eq!(
            png_dimensions(&path),
            (size, size),
            "{path:?} has the wrong size"
        );
    }
    assert!(repository()
        .join(format!("data/icons/hicolor/scalable/apps/{ICON_NAME}.svg"))
        .is_file());
}

#[test]
fn the_windows_icon_carries_every_shell_size() {
    let bytes = std::fs::read(repository().join("assets/branding/windows/strata.ico"))
        .expect("assets/branding/windows/strata.ico");
    assert_eq!(&bytes[..4], &[0, 0, 1, 0], "not an icon container");
    let count = u16::from_le_bytes(bytes[4..6].try_into().unwrap()) as usize;
    assert_eq!(count, ICO_SIZES.len(), "unexpected frame count");
    let mut found = Vec::new();
    for index in 0..count {
        let entry = 6 + 16 * index;
        let width = bytes[entry] as u32;
        assert_eq!(bytes[entry], bytes[entry + 1], "every frame must be square");
        found.push(if width == 0 { 256 } else { width });
    }
    assert_eq!(found, ICO_SIZES);
    let resource = read("windows/strata.rc");
    assert!(resource.contains("assets/branding/windows/strata.ico"));
}

#[test]
fn the_desktop_entry_and_appstream_metadata_agree() {
    let desktop = read("data/dev.strata.Strata.desktop");
    assert!(
        desktop.contains(&format!("Icon={ICON_NAME}\n")),
        "the desktop entry must request {ICON_NAME}"
    );
    let metainfo = read("data/dev.strata.Strata.metainfo.xml");
    assert!(
        metainfo.contains(&format!("<icon type=\"stock\">{ICON_NAME}</icon>")),
        "the AppStream metadata must advertise {ICON_NAME}"
    );
    assert!(metainfo.contains("<launchable type=\"desktop-id\">dev.strata.Strata.desktop"));
}

#[test]
fn packages_install_the_whole_icon_theme() {
    for (package, file) in [
        ("debian/rules", "debian/rules"),
        ("rpm/strata.spec", "rpm/strata.spec"),
    ] {
        let script = read(file);
        assert!(
            script.contains("data/icons/hicolor"),
            "{package} must install the hicolor theme"
        );
        assert!(
            !script.contains("data/icons/hicolor/scalable/apps/dev.strata.Strata.svg"),
            "{package} must not install a single hand-picked size"
        );
    }
    let flatpak = read("flatpak/dev.strata.Strata.json");
    assert!(flatpak.contains("cp -r data/icons/hicolor/. /app/share/icons/hicolor/"));
    let maintainer = read("debian/strata.postinst");
    assert!(maintainer.contains("update-icon-cache"));
}

#[test]
fn the_binary_asks_for_the_installed_icon() {
    let source = read("src/app/run.rs");
    assert!(source.contains(&format!("pub const ICON_NAME: &str = \"{ICON_NAME}\"")));
    assert!(source.contains("set_default_icon_name(ICON_NAME)"));
    let lookup = source
        .split("fn use_icon_theme()")
        .nth(1)
        .and_then(|rest| rest.split("pub fn main_with_args").next())
        .expect("use_icon_theme is defined in src/app/run.rs");
    assert!(lookup.contains("std::env::current_exe()"));
    assert!(
        !lookup.contains("CARGO_MANIFEST_DIR"),
        "icon lookup must not use a path frozen at compile time"
    );
}
