FutureOS Portable Setup Guide (Linux)
=====================================

[Launch]
Extract the archive to any directory; all three files must stay together:
    tar -xzf FutureOS-portable-linux.tar.gz
    ./futureos
Note: futureos, future-agent, and future must be in the same folder — do not
move them separately. future-agent is the backend service and is launched
automatically on startup.

[Runtime]
Requires the system WebKitGTK runtime. If you see a missing-library error on
launch, install it via your package manager:
    Debian/Ubuntu:  sudo apt install libwebkit2gtk-4.1-0
    Fedora:         sudo dnf install webkit2gtk4.1

[Notes]
· An internet connection is required for the first-time login (in the app).
  Personal data is stored in ~/.future.
· The background future-agent service stops automatically when you quit the app.
· The command-line tool future is included in the same directory.

If you encounter any issues, please send us a screenshot of the error message.
