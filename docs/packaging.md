# Packaging

## Desktop integration

`data/` holds everything a desktop needs, already validated:

| File | Installs to |
| --- | --- |
| `dev.strata.Strata.desktop` | `/usr/share/applications/` |
| `dev.strata.Strata.metainfo.xml` | `/usr/share/metainfo/` |
| `icons/hicolor/` | `/usr/share/icons/hicolor/` |

```sh
install -Dm644 data/dev.strata.Strata.desktop /usr/share/applications/dev.strata.Strata.desktop
install -Dm644 data/dev.strata.Strata.metainfo.xml /usr/share/metainfo/dev.strata.Strata.metainfo.xml
install -d /usr/share/icons/hicolor
cp -r data/icons/hicolor/. /usr/share/icons/hicolor/
install -Dm755 target/release/strata /usr/bin/strata
gtk4-update-icon-cache -qtf /usr/share/icons/hicolor
```

`desktop-file-validate data/dev.strata.Strata.desktop` is part of the checks.

## Icons

Everything is named `dev.strata.Strata`, which is also the application id, so the desktop entry,
the AppStream metadata, the hicolor theme and the window icon cannot drift apart. The PNG family
(16 to 1024 px, transparent outside the rounded square), the scalable SVG and
`assets/branding/windows/strata.ico` are all generated from `assets/branding/strata-icon.svg`; see
[docs/branding.md](branding.md) for the geometry, the rules and the regeneration command.

`src/app/run.rs` asks GTK for `dev.strata.Strata` with `gtk_window_set_default_icon_name()`. An
installed build resolves it through the icon theme. A `cargo run` binary additionally adds the
repository's `data/icons` to the theme search path when it finds it above the executable, so the
icon shows up during development without a path baked in at compile time.

Regenerate or verify the family with:

```sh
python3 scripts/generate-icons.py --check
```

## Flatpak

`flatpak/dev.strata.Strata.json` is a manifest for `flatpak-builder`. It builds from the
offline SDK, so the GStreamer plugins and codecs come from the runtime rather than the host.

```sh
flatpak-builder --user --install build flatpak/dev.strata.Strata.json
flatpak run dev.strata.Strata
```

## Debian and Ubuntu

`debian/` is a minimal source package: `control`, `changelog`, `rules`, `source/format`, the
`postinst`/`postrm` that refresh the icon cache, and `copyright`. It needs `dpkg-deb` and the
debhelper tooling.

```sh
cargo build --release
dpkg-buildpackage -us -uc -b     # ../strata_0.1.0_amd64.deb
```

## Fedora

`rpm/strata.spec` builds the same binary and installs the same data files. It needs `rpmbuild`.

```sh
cargo build --release
rpmbuild -bb rpm/strata.spec
```

## Windows

`assets/branding/windows/strata.ico` carries the 16, 24, 32, 48, 64, 128 and 256 px frames.
Windows reads it from the executable resources, and `windows/strata.rc` is the script that
compiles it in. `build.rs` deliberately does not do this: a Windows-only build dependency would
break the `cargo fetch --offline` step of the offline Flatpak build, so the resource is embedded
with the toolchain the Windows build already uses.

```sh
# MSVC, from a Visual Studio command prompt
rc.exe /fo strata-rc.res windows\strata.rc
set RUSTFLAGS=-C link-arg=strata-rc.res
cargo build --release

# MinGW, from MSYS2 or a UCRT shell
windres windows/strata.rc -O coff -o strata-rc.res
export RUSTFLAGS=-C link-arg=strata-rc.res
cargo build --release
```

The binary itself has no other platform-specific code, so it builds with the MSVC and GNU
toolchains as they are.

## Nix

The repository does not use flakes, so there is nothing to hook into. A Nix package would take the
same shape as the RPM spec, and the icon needs no special treatment because it is a plain
`data/icons/hicolor` tree:

```nix
postInstall = ''
  install -Dm644 data/dev.strata.Strata.desktop \
    $out/share/applications/dev.strata.Strata.desktop
  install -Dm644 data/dev.strata.Strata.metainfo.xml \
    $out/share/metainfo/dev.strata.Strata.metainfo.xml
  install -d $out/share/icons/hicolor
  cp -r data/icons/hicolor/. $out/share/icons/hicolor/
'';
```

Add that `postInstall` to a `rustPlatform.buildRustPackage`, and wrap the binary in a
`wrapGAppsHook4` so GTK finds the theme. Set `XDG_DATA_DIRS` to include `$out/share` when
installing into a test shell, and run `gtk4-update-icon-cache -qtf $out/share/icons/hicolor` once
after the build to see the launcher icon straight away.

## Dependencies

The runtime dependencies are GTK 4.14 or newer, GStreamer with the standard plugin set, and the
icon theme in use. `python3 scripts/generate-icons.py` additionally needs one of `rsvg-convert`,
`resvg` or `cairosvg`. `deny.toml` records the licence and advisory policy used to review
dependencies; run `cargo deny check` to apply it.