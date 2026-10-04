# Atlas Monitor for AtlasOS.

# No debuginfo subpackage: the Rust flags below keep symbols (debuginfo=2,
# strip=none) and the binary is shipped as built.
%global debug_package %{nil}

Name:           atlas-monitor
Version:        0.1.0
Release:        1%{?dist}
Summary:        Atlas Monitor, the system monitor of AtlasOS
License:        MIT
URL:            https://github.com/EternalCoder454/atlasos-monitor
Source0:        atlas-monitor-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
# %%build_rustflags
BuildRequires:  rust-srpm-macros
BuildRequires:  gcc
BuildRequires:  gcc-c++
BuildRequires:  cmake
BuildRequires:  ninja-build
BuildRequires:  corrosion
# Cargo fetches the atlas-framework crates from GitHub.
BuildRequires:  git-core
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib
BuildRequires:  cmake(Qt6Core)
BuildRequires:  cmake(Qt6Gui)
BuildRequires:  cmake(Qt6Qml)
BuildRequires:  cmake(Qt6Quick)
BuildRequires:  cmake(Qt6QuickControls2)
BuildRequires:  cmake(Qt6Widgets)
BuildRequires:  cmake(Qt6QmlTools)
BuildRequires:  qt6-qtbase-devel
BuildRequires:  cmake(KF6CoreAddons)
BuildRequires:  cmake(KF6DBusAddons)
BuildRequires:  cmake(KF6WindowSystem)
# QML modules qmlcachegen resolves at build time (not linked). atlas-ui comes
# from atlas-framework, which is in no repository: install its RPMs first
# (build-rpm.sh does, given ATLAS_LOCAL_RPMS).
BuildRequires:  kf6-kirigami-devel
BuildRequires:  atlas-ui

Requires:       kf6-kirigami
# Atlas.Ui, the shared look (atlas-framework)
Requires:       atlas-ui
Requires:       kf6-qqc2-desktop-style
Requires:       qt6-qtdeclarative
# the app icon and Breeze's icons are SVG
Requires:       qt6-qtsvg

%description
Atlas Monitor shows your apps and what they use, the processor, memory, disks,
network, graphics card, battery and sensors, with live charts. It can end apps,
start and stop services, and choose what starts when you log in. System Info
and Devices show what the computer is made of and what is connected to it.

%prep
%autosetup -n atlas-monitor-%{version}

%build
# NETWORK: cargo (Corrosion runs it with --locked) fetches crates.io and the
# pinned atlas-framework crates during %%build. That works in podman and with `rpmbuild`
# on a networked machine, not in an offline mock/Koji build.
# CARGO_HOME from the environment keeps a crate cache between builds
# (CLAUDE.md mounts one); otherwise a fresh one in the build dir.
export CARGO_HOME=${CARGO_HOME:-%{_builddir}/cargo-home}
# Fedora's Rust flags (hardening, build-id, ...), also used by Corrosion's cargo.
# The remaps keep build paths (panic locations, assert file names) out of the
# package, as atlas-framework's DESIGN.md asks of apps using its crates.
# HOST_CXXFLAGS reaches only the C++ that cargo's build scripts compile
# (cc-rs reads HOST_ when not cross-compiling; CMake ignores it), which
# otherwise gets no flags from here. (cc-rs then ignores a plain CXXFLAGS,
# which is only for CMake.)
# CFLAGS and CXXFLAGS are Fedora's plus the same remap for the C++ CMake
# builds (%%cmake keeps them when set). Without it the build dir, a random
# mktemp one, goes into the debug info and so into the linker's build ID,
# and two builds of one commit differ. These flags split on spaces, so
# _topdir must have none (build-rpm.sh's hasn't).
export RUSTFLAGS="%{build_rustflags} --remap-path-prefix=$PWD=. --remap-path-prefix=$CARGO_HOME=cargo"
export HOST_CXXFLAGS="-ffile-prefix-map=$PWD=. -ffile-prefix-map=$CARGO_HOME=cargo"
export CFLAGS="%{build_cflags} -ffile-prefix-map=$PWD=."
export CXXFLAGS="%{build_cxxflags} -ffile-prefix-map=$PWD=."
export CARGO_PROFILE_RELEASE_STRIP=none
# (checked with rpmspec --eval: %%cmake honours _vpath_srcdir, not __cmake_source_dir)
%global _vpath_srcdir apps/atlas-monitor
%cmake -G Ninja -DCMAKE_BUILD_TYPE=Release
%cmake_build

%install
%cmake_install
# AtlasOS needs its system monitor: dnf refuses to remove it.
install -Dpm0644 apps/atlas-monitor/data/dnf/protected.d/atlas-monitor.conf \
    %{buildroot}%{_sysconfdir}/dnf/protected.d/atlas-monitor.conf

%check
# No path into the build tree (checked as well as set: see %%build).
# grep: 0 = found, 1 = not found, anything else (no binary) fails too.
rc=0
grep -qF "%{_builddir}" %{buildroot}%{_bindir}/atlas-monitor || rc=$?
if [ "$rc" != 1 ]; then
    echo "atlas-monitor holds the build path %{_builddir} (grep status $rc)" >&2
    exit 1
fi
desktop-file-validate %{buildroot}%{_datadir}/applications/net.eterneon.atlas.monitor.desktop
appstream-util validate-relax --nonet \
    %{buildroot}%{_datadir}/metainfo/net.eterneon.atlas.monitor.metainfo.xml

%files
%license LICENSE
%{_bindir}/atlas-monitor
%{_datadir}/applications/net.eterneon.atlas.monitor.desktop
%{_datadir}/kglobalaccel/net.eterneon.atlas.monitor.desktop
%{_datadir}/metainfo/net.eterneon.atlas.monitor.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/net.eterneon.atlas.monitor.svg
%{_datadir}/icons/hicolor/16x16/apps/net.eterneon.atlas.monitor.svg
%config(noreplace) %{_sysconfdir}/dnf/protected.d/atlas-monitor.conf

%changelog
* Fri Oct 02 2026 Atlas <atlas@eterneon.net> - 0.1.0-1
- First package
