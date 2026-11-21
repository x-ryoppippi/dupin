# dmgbuild settings for Dupin.dmg. scripts/package-app.nu passes the app and the background
# with `-D app=… -D background=…`; the window matches packaging/dmg/background.svg.
import os.path

app = defines["app"]  # noqa: F821
background = defines["background"]  # noqa: F821

format = "UDZO"
files = [app]
symlinks = {"Applications": "/Applications"}
hide_extensions = [os.path.basename(app)]

window_rect = ((200, 200), (660, 400))
icon_size = 128
text_size = 13
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
default_view = "icon-view"
icon_locations = {
    os.path.basename(app): (165, 175),
    "Applications": (495, 175),
}
