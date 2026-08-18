FutureOS Portable Setup Guide (Linux)
=====================================

[Launch]
Extract the archive to any directory; all three files must stay together:
    tar -xzf FutureOS-portable-linux.tar.gz
    ./futureos
Note: futureos and future must be in the same folder — do not move them
separately. The backend agent is launched automatically via future
(`future agent`) on startup.

[Runtime]
The GUI requires the system WebKitGTK runtime. If you see a missing-library
error on launch, install it via your package manager:
    Debian/Ubuntu:  sudo apt install libwebkit2gtk-4.1-0
    Fedora:         sudo dnf install webkit2gtk4.1
Note: the GUI requires a recent system (glibc >= 2.39, roughly Ubuntu 24.04+).
However, the bundled command-line tool future is statically linked and has no
system library requirements — even if the GUI cannot start on an older system,
you can still use ./future directly (CLI / `future tui`).

[Notes]
· An internet connection is required for the first-time login (in the app).
  Personal data is stored in ~/.future.
· The background agent stops automatically when you quit the app.
· The command-line tool future is included in the same directory.

[License]
FutureOS is distributed under the MIT License; the bundled future loop
component is derived from LoopX and distributed under Apache-2.0.
Full license texts and attribution notices: see the licenses/ directory.

If you encounter any issues, please send us a screenshot of the error message.
