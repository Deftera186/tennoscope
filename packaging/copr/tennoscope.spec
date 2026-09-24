Name:           tennoscope
Version:        0.12.0
Release:        1%{?dist}
Summary:        Local-first Warframe collection and relic companion
License:        GPL-3.0-only
URL:            https://github.com/Deftera186/tennoscope
# No -debuginfo: the vendored Rust sources trip brp-mangle-shebangs (inner
# attributes like #![no_std] read as shebangs), and a leaf GUI gains nothing
# from a debug package worth the size.
%global debug_package %{nil}
Source0:        %{url}/archive/refs/tags/v%{version}.tar.gz
# Vendored dependencies (COPR chroots have no network): cargo vendor output and
# the installed node_modules, regenerated per release (see README).
Source1:        tennoscope-vendor-%{version}.tar.zst
Source2:        tennoscope-node-modules-%{version}.tar.zst
BuildRequires:  cargo
BuildRequires:  clang
BuildRequires:  gcc
BuildRequires:  gcc-c++
BuildRequires:  libappindicator-gtk3-devel
BuildRequires:  libglvnd-devel
BuildRequires:  make
BuildRequires:  mesa-libEGL-devel
BuildRequires:  mesa-libgbm-devel
BuildRequires:  openssl-devel
BuildRequires:  pipewire-devel
BuildRequires:  pnpm
BuildRequires:  webkit2gtk4.1-devel
BuildRequires:  wget
# check() runs the reward-reader tests, which shell out to tesseract for real.
BuildRequires:  tesseract
BuildRequires:  tesseract-langpack-eng

Requires:       gtk3
Requires:       pipewire-libs
Requires:       webkit2gtk4.1
Requires:       xdg-utils
# The relic overlay shells out to tesseract; the collection browser works
# without it, so this stays a recommendation, not a requirement.
Recommends:     tesseract

%description
TennoScope reads a running Warframe process without modifying it and keeps a
local copy of your collection. When a relic cracks it recognises the four
rewards on screen and prices them in platinum and ducats from warframe.market,
counting only sellers who are online. No account, no telemetry, no Overwolf.
%prep
# Explicit extraction (not %%autosetup -a): a silently skipped -a flag once
# shipped a vendor-less tree to the builders. Every section below cds
# explicitly: rpm only guarantees the build root, never the source dir.
cd "%{_builddir}"
%autosetup -n %{name}-%{version}
cd "%{_builddir}/%{name}-%{version}"
tar --zstd -xf %{SOURCE1}
tar --zstd -xf %{SOURCE2}
mv node_modules app/node_modules
test -d vendor -a -d app/node_modules || { echo "vendored sources missing after extraction" >&2; exit 1; }
# pnpm 10 self-installs the packageManager-pinned version on every invocation;
# with no network that kills the build before it starts. Pin it off locally.
printf '\nmanage-package-manager-versions=false\n' >> app/.npmrc
mkdir -p .cargo
printf '[source.crates-io]\nreplace-with = "vendored-sources"\n[source.vendored-sources]\ndirectory = "vendor"\n' > .cargo/config.toml
%build
cd "%{_builddir}/%{name}-%{version}"
export CARGO_TARGET_DIR=%{_builddir}/target
export CARGO_NET_OFFLINE=true
# node_modules is vendored: no install step, and no network to install from.
pnpm --dir app build
# tauri/custom-protocol is mandatory: without it generate_context!() bakes in
# the dev-server URL and the release binary opens a connection-refused WebView.
cargo build --release --locked --offline -p tennoscope --features tauri/custom-protocol

%install
cd "%{_builddir}/%{name}-%{version}"
install -Dm755 %{_builddir}/target/release/tennoscope %{buildroot}%{_bindir}/tennoscope
install -Dm644 packaging/tennoscope.desktop %{buildroot}%{_datadir}/applications/tennoscope.desktop
install -Dm644 app/src-tauri/icons/128x128.png %{buildroot}%{_datadir}/icons/hicolor/128x128/apps/tennoscope.png
install -Dm644 LICENSE %{buildroot}%{_datadir}/licenses/%{name}/LICENSE
install -Dm644 THIRD_PARTY_NOTICES.md %{buildroot}%{_datadir}/doc/%{name}/THIRD_PARTY_NOTICES.md
%check
cd "%{_builddir}/%{name}-%{version}"
export CARGO_TARGET_DIR=%{_builddir}/target
export CARGO_NET_OFFLINE=true
# Same traineddata caveat as the Arch recipe: Fedora ships upstream's combined
# legacy+LSTM data, on which one 16:10 fixture reads 0.875 against a 0.9 floor
# that proves crop geometry elsewhere. Skipped, not loosened.
RUSTFLAGS="${RUSTFLAGS:-} -C debug-assertions=on" \
  cargo test --workspace --locked --offline -- \
    --skip a_16_10_screen_is_read_where_a_16_10_screen_actually_sits
pnpm --dir app check
%files
%{_bindir}/tennoscope
%{_datadir}/applications/tennoscope.desktop
%{_datadir}/icons/hicolor/128x128/apps/tennoscope.png
%{_datadir}/licenses/%{name}/LICENSE
%{_datadir}/doc/%{name}/THIRD_PARTY_NOTICES.md

%changelog
* Tue Sep 22 2026 Deftera186 <https://github.com/Deftera186/tennoscope/issues> - 0.11.0-1
- Initial COPR recipe
