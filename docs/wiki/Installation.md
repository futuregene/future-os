# Install FutureOS

FutureOS is a desktop app for **macOS, Windows, and Linux**. Download it, open it once following the first-launch steps for your system, and you're ready.

## 1. Download

Go to the **[Releases page](https://github.com/futuregene/future-os/releases)** and download the **latest version** for your operating system:

| System | What to download |
|---|---|
| **macOS** | the `.dmg` disk image |
| **Windows** | the installer (`.exe` / `.msi`), or the portable `.zip` |
| **Linux** | the `.AppImage` or `.deb` package |

Always grab the newest release — that's how you get updates.

> **Want the command-line tools too?** Optional `future-cli` and `future-tui` binaries are included in the **macOS app** and in the **Windows/Linux portable** packages (not the plain Windows installer or Linux `.deb`). See [[CLI|CLI]] and [[Terminal UI|TUI]].

## 2. First launch

The current builds are not yet signed/notarized, so your system may warn you the first time. This is expected — here's how to open it.

### macOS

1. Open the `.dmg` and drag the **FutureOS** icon into your **Applications** folder.
2. The first time, double-clicking may be blocked ("unidentified developer" or "damaged"). To open it:
   - **Right-click** (or Control-click) **FutureOS** in Applications → **Open** → click **Open** again in the dialog. After this first time, it opens normally.
   - If it says *"damaged and can't be opened"*, open **Terminal** and run:
     ```
     xattr -dr com.apple.quarantine /Applications/FutureOS.app
     ```
     then open it normally.

### Windows

1. **Installer:** run the `.exe`/`.msi` and follow the prompts.
   **Portable:** unzip the whole folder anywhere (e.g. `D:\FutureOS`) and double-click `FutureOS.exe` — no install needed.
   > For the portable version, keep `FutureOS.exe` and `future-agent.exe` together in the same folder. The agent is the background service the app starts automatically.
2. On first run, Windows SmartScreen may show *"Windows protected your PC."* Click **More info → Run anyway**.
3. FutureOS needs the **Microsoft Edge WebView2 Runtime**. Windows 10 (recent) and Windows 11 usually have it already. If the window doesn't appear or you're told a component is missing, install **"Microsoft Edge WebView2 Runtime"** (Evergreen) from Microsoft's site, then launch again.

### Linux

- **AppImage:** make it executable and run it:
  ```
  chmod +x FutureOS*.AppImage
  ./FutureOS*.AppImage
  ```
- **.deb:** install it with your package manager (e.g. `sudo apt install ./FutureOS*.deb`) and launch FutureOS from your applications menu.

## 3. Sign in

The first time you use FutureOS you'll need to be online and **sign in inside the app**. See the [[Quick Start|Quick-Start]] for the walkthrough.

## Where your data lives

Your settings and conversations are stored in a folder named **`.future`** in your home directory:

- macOS / Linux: `~/.future`
- Windows: `C:\Users\<you>\.future`

## Updating

To update, download the latest release and install it over the old version (on Windows portable, replace the folder). Your data in `.future` is kept.

## Uninstalling

- **macOS:** delete `FutureOS.app` from Applications.
- **Windows:** uninstall from Settings (installer), or just delete the folder (portable).
- **Linux:** remove the AppImage, or uninstall the `.deb` package.

To erase your data too, also delete the `.future` folder above.

---

Next: **[[Quick Start|Quick-Start]]** →
