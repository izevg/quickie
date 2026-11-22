# dmgbuild settings for the Quickie installer. Coordinates are in Finder points.

format = "UDZO"
files = [defines["app"]]
symlinks = {"Applications": "/Applications"}
background = defines["background"]

default_view = "icon-view"
window_rect = ((120, 120), (768, 512))
icon_size = 128
text_size = 16
icon_locations = {"Quickie.app": (174, 288), "Applications": (594, 288)}
