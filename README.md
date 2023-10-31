# `childe`

A command-line utility for Sway workspace creation.

`childe` creates Sway workspaces in three variation:

1. Find the next (minimum index) unused workspace, create it, and navigate to it.
2. Find the next unused workspace, and move the currently focused window to it.
3. Find the next unused workspace, and move the currently focused window and then navigate to it.

## Requirements

`childe` was created for the `Sway` version of the Regolith desktop environment. It communicates with `Sway` over IPC and reads workspace configuration from `trawl`.

## Usage

```
Simple command line utility to find and move to the next unallocated workspace

Usage: childe [OPTIONS]

Options:
  -m, --move-window  
  -f, --follow       
  -h, --help         Print help
  -V, --version      Print version
```

## License

GNU General Public License version 3