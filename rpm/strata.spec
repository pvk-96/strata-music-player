Name:           strata
Version:        0.1.0
Release:        1%{?dist}
Summary:        Local-first music player for your own library

License:        GPL-3.0-or-later
URL:            https://github.com/strata-player/strata
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  gtk4-devel >= 4.14
BuildRequires:  gstreamer1-devel
BuildRequires:  gstreamer1-plugins-base-devel
BuildRequires:  sqlite-devel
BuildRequires:  make

Requires:       gtk4 >= 4.14
Requires:       gstreamer1
Requires:       gstreamer1-plugins-good
Requires:       gstreamer1-plugins-base
Requires:       sqlite-libs

%description
Strata plays the music on your own machine. It reads the tags of the files you
already have, keeps its library in a local SQLite database, and never modifies,
renames or moves your music. No accounts, no telemetry, no network access.

%prep
%autosetup

%build
cargo build --release

%install
rm -rf %{buildroot}
install -Dm755 target/release/strata %{buildroot}%{_bindir}/strata
install -Dm644 data/dev.strata.Strata.desktop %{buildroot}%{_datadir}/applications/dev.strata.Strata.desktop
install -Dm644 data/dev.strata.Strata.metainfo.xml %{buildroot}%{_datadir}/metainfo/dev.strata.Strata.metainfo.xml
install -d %{buildroot}%{_datadir}/icons/hicolor
cp -r data/icons/hicolor/. %{buildroot}%{_datadir}/icons/hicolor/
install -Dm644 LICENSE %{buildroot}%{_datadir}/licenses/%{name}/LICENSE

%check
cargo test --release

%post
for cache in gtk4-update-icon-cache gtk-update-icon-cache; do
    if command -v $cache >/dev/null 2>&1; then
        $cache -qtf %{_datadir}/icons/hicolor || :
        break
    fi
done

%postun
for cache in gtk4-update-icon-cache gtk-update-icon-cache; do
    if command -v $cache >/dev/null 2>&1; then
        $cache -qtf %{_datadir}/icons/hicolor || :
        break
    fi
done

%files
%license %{_datadir}/licenses/%{name}/LICENSE
%doc README.md
%doc docs/architecture.md
%doc docs/database.md
%doc docs/scanner.md
%doc docs/keyboard.md
%doc docs/branding.md
%doc docs/packaging.md
%{_bindir}/strata
%{_datadir}/applications/dev.strata.Strata.desktop
%{_datadir}/metainfo/dev.strata.Strata.metainfo.xml
%{_datadir}/icons/hicolor/

%changelog
* Thu Oct 02 2026 Strata contributors <strata@example.invalid> - 0.1.0-1
- First release.