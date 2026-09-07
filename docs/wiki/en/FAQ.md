# FAQ & Troubleshooting

Quick answers to common questions. If you're stuck, you can [report an issue](https://github.com/futuregene/future-os/issues).

---

### macOS won't open the app ("unidentified developer" / "damaged")

Official release builds are signed and notarized; unsigned test builds are separate. First verify the source and artifact channel. For an unexpected warning on an official signed release, redownload and report it rather than removing quarantine. Only for a trusted unsigned test build:

- **Right-click** (or Control-click) **FutureOS** in Applications → **Open** → **Open** again. After the first time it launches normally.
- If it says **"damaged"**, run this once in the **Terminal** app, then open it again:

  ```bash
  xattr -dr com.apple.quarantine /Applications/FutureOS.app
  ```

### Windows says "Windows protected your PC"

That's **SmartScreen**; a reputation warning can occur even for signed software. Verify the official source and publisher first. Only proceed via **More info → Run anyway** if you trust the artifact; do not bypass an unexpected publisher/signature mismatch.

### Windows: nothing happens when I launch it

- Install the **Microsoft Edge WebView2 Runtime** (Evergreen version) from Microsoft's website — the app needs it. Recent Windows 10 and Windows 11 usually already have it.
- On the **portable** version, make sure `FutureOS.exe` and `future.exe` are in the **same folder**.
- If the window opens but says the background service isn't connected, the `.zip` was tagged "came from the Internet". Right-click the `.zip` → **Properties** → tick **Unblock** → unzip again.

### I can't use any model / I'm not signed in

Open **Settings → Providers → FutureGene → Sign in** to sign in, or add your own provider. See [[Settings]].

### How do I switch models?

Use the **model selector** inside the input box, or manage which models appear in **Settings → Models**.

### The agent stopped and is asking me something

The selected approval mode/rules require a decision. Choose **Allow once**, **Deny**, or a saved project rule when offered; there is no timeout. The default Unrestricted mode does not ask, and even protected modes do not prompt for every call. See [[Sandbox]] and [[Using FutureOS|Using-FutureOS]].

### Where are my conversations and settings stored?

In a `.future` folder in your home directory:

- **macOS/Linux:** `~/.future`
- **Windows:** `C:\Users\<you>\.future`

### How do I update?

Download the latest version and install it over the old one (replace the folder for the portable version). Your `.future` data is kept. You can also check from **Settings → Check for updates**.

### How do I uninstall or clear my data?

Delete the app (macOS: remove `FutureOS.app`; Windows: uninstall or delete the portable folder; Linux: `sudo apt remove futureos` for deb installs or remove the portable files). To also remove your data, delete the `.future` folder. Inside the app, **Settings → Reset** can clear local data too.

### Which platforms are supported?

**macOS, Windows and Linux desktop; Android/iOS via [[Remote]].** Linux releases include x86_64/aarch64 desktop and CLI-only packages; see [[Installation]].

### Linux sandbox is unavailable

Install trusted system Bubblewrap ≥ 0.9.0, fully restart FutureOS, then run `future agent --probe-sandbox` and `future doctor`. User namespaces or fresh `/proc` mounts may be restricted by host policy; ask the administrator rather than bypassing it. See [[Sandbox]] for diagnostic codes and Manual fallback.

### Linux GUI won't start on an older server

The published GUI needs glibc ≥ 2.39 and WebKitGTK 4.1. Use the matching official static CLI-only tarball on a headless host instead; see [[Installation]].

---

## See also

- [[Install FutureOS|Installation]]
- [[Quick Start|Quick-Start]]
- [[Using FutureOS|Using-FutureOS]]
