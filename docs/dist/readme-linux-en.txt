FutureOS Portable Setup Guide (Linux)
=====================================

[Launch]
Extract the downloaded archive to any directory, then run ./futureos.
Use its actual filename: official releases use
FutureOS_<version>_linux_<arch>-portable.tar.gz (x86_64 or aarch64);
local developer builds may use FutureOS-portable-linux.tar.gz.
Keep futureos, futureos-headless, and future in the same folder. The graphical
or headless entrypoint starts its agent via future agent when needed.

[Runtime]
Published GUI packages require glibc >= 2.39 (roughly Ubuntu 24.04+) and WebKitGTK:
    Debian/Ubuntu: sudo apt install libwebkit2gtk-4.1-0
    Fedora:        sudo dnf install webkit2gtk4.1
Official Linux CI builds future and futureos-headless as fully static musl
binaries — no GUI libraries and no glibc requirement (they run on CentOS 7 /
Rocky 8-era hosts). Local source builds use their selected host target and are
not necessarily static.
On a headless host, download the matching official CLI-only tarball and use
./future config and ./future tui, or ./future agent for other CLI clients.
For server phone access, run ./futureos-headless. It is an independent foreground
program with no graphical UI; Ctrl+C closes it. It is not a futureos sidecar.
futureos starts only the GUI; its former --headless option has been removed.

[Optional Sandbox]
The desktop defaults to Unrestricted. Select Manual or Sandboxed in Settings.
Linux Sandboxed mode needs trusted system Bubblewrap >= 0.9.0 (not bundled):
    Debian/Ubuntu: sudo apt install bubblewrap
    Fedora:        sudo dnf install bubblewrap
Old distribution packages may need a trusted upgrade. Fully restart FutureOS,
then run future agent --probe-sandbox and future doctor. Host namespace policy
may prevent use; a definite unavailable result falls back to Manual.
Network remains open; missing protected paths/new matches have detection-only
limits. See the repository wiki Sandbox guide for full boundaries.

[Notes]
· Configure a provider/key, or sign in online for hosted models.
· Personal data is stored in ~/.future. Online model/tools and Remote send requests
  to their services; local-first does not mean no data leaves the machine.
· The app stops only the agent it started; an externally managed agent stays running.
· The standalone futureos-headless entrypoint and unified future CLI are included.

[License]
FutureOS is distributed under the MIT License; the bundled future loop
component is derived from LoopX and distributed under Apache-2.0.
Full license texts and attribution notices: see the licenses/ directory.

If you encounter issues, report the version, architecture and error, without secrets.
