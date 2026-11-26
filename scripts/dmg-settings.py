# dmgbuild settings for Nimbra's disk image (used by scripts/dmg-macos.sh).
# The window is 660×400 points over assets/dmg/background (the Inbox zero sky, with a
# hint on top); the two icons sit either side of the arrow drawn between them.

app = defines["app"]  # noqa: F821 (dmgbuild passes -D values in `defines`)
background = defines["background"]  # noqa: F821
volume_icon = defines.get("volume_icon")  # noqa: F821

format = "UDZO"
filesystem = "HFS+"
files = [app]
symlinks = {"Applications": "/Applications"}
if volume_icon:
    icon = volume_icon

window_rect = ((200, 160), (660, 400))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
include_icon_view_settings = True
icon_size = 128
text_size = 13
arrange_by = None
icon_locations = {
    "Nimbra.app": (165, 190),
    "Applications": (495, 190),
}
# "Nimbra", not "Nimbra.app" (unless Finder is set to show every extension).
hide_extension = ["Nimbra.app"]
