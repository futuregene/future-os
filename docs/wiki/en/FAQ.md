# FAQ & Troubleshooting

Quick answers to common questions. If you're stuck, you can [report an issue](https://github.com/futuregene/future-os/issues).

---

### macOS won't open the app ("unidentified developer" / "damaged")

The current build isn't notarized, so this is expected.

- **Right-click** (or Control-click) **FutureOS** in Applications → **Open** → **Open** again. After the first time it launches normally.
- If it says **"damaged"**, run this once in the **Terminal** app, then open it again:

  ```bash
  xattr -dr com.apple.quarantine /Applications/FutureOS.app
  ```

### Windows says "Windows protected your PC"

That's **SmartScreen**. Click **More info → Run anyway**.

### Windows: nothing happens when I launch it

- Install the **Microsoft Edge WebView2 Runtime** (Evergreen version) from Microsoft's website — the app needs it. Recent Windows 10 and Windows 11 usually already have it.
- On the **portable** version, make sure `FutureOS.exe` and `future-agent.exe` are in the **same folder**.
- If the window opens but says the background service isn't connected, the `.zip` was tagged "came from the Internet". Right-click the `.zip` → **Properties** → tick **Unblock** → unzip again.

### I can't use any model / I'm not signed in

Open **Settings → Providers → FutureGene → Connect** to sign in, or add your own provider. See [[Settings]].

### How do I switch models?

Use the **model selector** inside the input box, or manage which models appear in **Settings → Models**.

### The agent stopped and is asking me something

That's the **approval mechanism** — the agent pauses before risky actions and waits for you (no timeout). Choose **Allow once**, **Deny**, or (when offered) allow it for this project. See [[Using FutureOS|Using-FutureOS]].

### Where are my conversations and settings stored?

In a `.future` folder in your home directory:

- **macOS:** `~/.future`
- **Windows:** `C:\Users\<you>\.future`

### How do I update?

Download the latest version and install it over the old one (replace the folder for the portable version). Your `.future` data is kept. You can also check from **Settings → Check for updates**.

### How do I uninstall or clear my data?

Delete the app (macOS: remove `FutureOS.app`; Windows: uninstall or delete the portable folder). To also remove your data, delete the `.future` folder. Inside the app, **Settings → Reset** can clear local data too.

### Which platforms are supported?

**macOS and Windows.**

---

## See also

- [[Install FutureOS|Installation]]
- [[Quick Start|Quick-Start]]
- [[Using FutureOS|Using-FutureOS]]
