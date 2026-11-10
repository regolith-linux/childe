# `childe`

A command-line utility for workspace creation and naming in the Regolith desktop, for both Sway and i3.

`childe` creates workspaces in three variation:

1. Find the next (minimum index) unused workspace, create it, and navigate to it.
2. Find the next unused workspace, and move the currently focused window to it.
3. Find the next unused workspace, and move the currently focused window and then navigate to it.

The new workspace is named from the `wm.workspace.NN.name` resource, or by its number without one.

## Workspace names

`childe --workspace-names` runs for the whole session and keeps every workspace's name showing the apps on it.

- A window is labelled by the resource `wm.workspace.icon.<id>`, where `<id>` is its lowercased app_id (native Wayland) or `WM_CLASS` class (X11). Without that resource, the label is the lowercased identifier. Values are used as they are, so they can be icon glyphs.
- Labels follow the layout: tiled windows first, depth-first, then floating ones. Each label appears once.
- Workspace `2` with Firefox (`wm.workspace.icon.firefox: web`) and foot shows `2: web foot`; `music` with foot shows `music foot`. With Regolith's default `wm.bar.stripworkspacenumbers: yes`, the bar shows only `web foot` for the first one.
- A name given by the user (`rename workspace`) becomes the new base name and is decorated in turn.
- On SIGTERM or SIGINT the daemon gives every workspace its name back. It exits with the window manager, and survives an i3 `restart`. Changed resources take effect on `reload`.
- Only one daemon runs per window manager. A second `childe --workspace-names`, for example from `exec_always` on a reload, prints a note and exits with status 0.

To start it with a Regolith session, add a config partial such as `~/.config/regolith3/sway/config.d/90_workspace-names` (or `.../i3/config.d/...` for i3) containing:

```
exec --no-startup-id childe --workspace-names
```

Rules and bindings that send windows or focus to a workspace, such as `assign` or `move container to workspace`, should refer to it as `number N`: one that names `2` no longer finds the workspace once it is called `2: web foot`, and creates a new workspace `2` instead. Regolith's own bindings already use `number`.

## Requirements

`childe` communicates with Sway or i3 over IPC. In a Sway session it reads its resources from `trawl`; in an i3 session, from the X resource database loaded by `xrdb`.

## Usage

```
Simple command line utility to find and move to the next unallocated workspace

Usage: childe [OPTIONS]

Options:
  -m, --move-window      
  -f, --follow           
  -w, --workspace-names  Keep running, and keep every workspace's name showing the apps on it
  -h, --help             Print help
  -V, --version          Print version
```

## License

GNU General Public License version 3