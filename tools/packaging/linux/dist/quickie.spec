%global debug_package %{nil}
%global _build_id_links none
%global __os_install_post %{nil}
%global __requires_exclude ^lib(webkitgtk-6[.]0|javascriptcoregtk-6[.]0)[.]so
Name: quickie
Version: %{quickie_version}
Release: 1%{?dist}
Summary: A native, content-first browser
License: Apache-2.0 AND BSD-2-Clause AND BSD-3-Clause AND LGPL-2.0-or-later AND LGPL-2.1-or-later
AutoProv: no
Requires: bubblewrap
Requires: xdg-dbus-proxy
Requires: polkit
Requires: gstreamer1-plugins-base
Requires: gstreamer1-plugins-good
Requires: pipewire-gstreamer
Requires: xdg-desktop-portal
# WebKit loads GLES dynamically, so automatic ELF requirements omit it.
Requires: libglvnd-gles%{?_isa}
Requires: shared-mime-info

%description
The first Quickie preview for Linux, with a native GTK4 window and Quickie's WebKitGTK fork.

%install
mkdir -p "%{buildroot}"
cp -a "%{quickie_stage}"/. "%{buildroot}"/

%files
/opt/quickie
%{_bindir}/quickie
%{_datadir}/applications/quickie.desktop
%{_datadir}/icons/hicolor/512x512/apps/quickie.png
%license /opt/quickie/share/doc/quickie/LICENSE
