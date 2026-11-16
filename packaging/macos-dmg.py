from pathlib import Path

app = Path(defines["app"])
files = [str(app)]
symlinks = {"Applications": "/Applications"}
icon = str(app / "Contents/Resources/Duocle.icns")
background = defines["background"]

format = "UDZO"
window_rect = ((180, 130), (600, 430))
icon_size = 104
text_size = 13
icon_locations = {"Duocle.app": (130, 215), "Applications": (470, 215)}
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
