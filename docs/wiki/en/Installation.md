# Install FutureOS

FutureOS runs on **macOS and Windows**. This page covers downloading, first launch, where your data lives, updating, and uninstalling.

---

## Download

Get the latest version for your system from the Releases page:

> **Downloads:** https://github.com/futuregene/future-os/releases

| System | What to download |
|---|---|
| **macOS** | `.dmg` disk image |
| **Windows** | An installer (`.exe`), or the portable `.zip` |

The command-line tool `future` ships **inside every download** — it sits next to the app in both the installer and the portable package. You don't need to install it separately. See the [[CLI (future)|CLI]] page if you want to use it.

---

## First launch

Formal macOS and Windows installers are signed, and the macOS build is also notarized by Apple.

### macOS

1. Open the `.dmg` and drag **FutureOS** into your **Applications** folder.
2. Double-click FutureOS in Applications to launch it.

### Windows

- **Installer version:** run the `.exe` and follow the prompts.
- **Portable version:** unzip the **whole folder**, then double-click `FutureOS.exe`. Keep `FutureOS.exe` and `future-agent.exe` in the **same folder** — the background service is started automatically from there, so don't move `FutureOS.exe` on its own.
- Windows may temporarily show a SmartScreen reputation prompt. Confirm that the publisher and download source are the official FutureOS channel.
- FutureOS needs the **Microsoft Edge WebView2 Runtime**. Recent Windows 10 and Windows 11 usually already have it. If the window won't open or a component is reported missing, install the "Evergreen" WebView2 Runtime from Microsoft's website and try again.

> **Portable "zip" tip (Windows):** if the window opens but says the background service isn't connected, the downloaded `.zip` was tagged "came from the Internet". Before unzipping, right-click the `.zip` → **Properties** → tick **Unblock** → OK, then unzip. (Or, after unzipping, run `Get-ChildItem -Recurse | Unblock-File` inside the folder in PowerShell.)

---

## Sign in

The first time you use FutureOS you'll need an internet connection and a quick sign-in **inside the app**. See [[Quick Start|Quick-Start]] for the steps.

---

## Where your data lives

Your conversations and settings are stored in a `.future` folder in your home directory:

| System | Location |
|---|---|
| **macOS** | `~/.future` |
| **Windows** | `C:\Users\<you>\.future` |

---

## Updating

Installer builds can open **Settings → Check for updates** to download and install a signature-verified update, then restart when prompted. You can also manually install the latest version over the old one. For the portable version, replace the folder. Your `.future` data is kept.

---

## Uninstalling

- **macOS:** delete `FutureOS.app` from Applications.
- **Windows:** uninstall from Windows Settings (installer version), or delete the portable folder.

To also remove your data, delete the `.future` folder afterward.

---

## See also

- [[Quick Start|Quick-Start]] — sign in and send your first message.
- [[FAQ]] — first-launch warnings and other common issues.
