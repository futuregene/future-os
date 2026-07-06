# FAQ & Troubleshooting

Quick answers to common questions. If yours isn't here, check [[Installation]], [[Quick Start|Quick-Start]], [[Using FutureOS|Using-FutureOS]], or [[Settings]].

## macOS won't open the app ("unidentified developer" / "damaged")

The current builds aren't notarized yet, so macOS blocks the first open. Right-click **FutureOS** in Applications → **Open** → **Open** again. If it says *"damaged"*, run this in Terminal, then open normally:

```
xattr -dr com.apple.quarantine /Applications/FutureOS.app
```

Full steps: [[Installation]].

## Windows says "Windows protected your PC"

That's SmartScreen on an unsigned app. Click **More info → Run anyway**.

## Windows: nothing happens when I launch it

FutureOS needs the **Microsoft Edge WebView2 Runtime**. Install "Microsoft Edge WebView2 Runtime" (Evergreen) from Microsoft's site, then try again. On the portable version, also make sure `FutureOS.exe` and `future-agent.exe` are in the **same folder**.

## I can't use any models / I'm not signed in

You need to connect a provider. Open **Settings → Providers** and click **Connect** under FutureGene to sign in, or add your own provider. See [[Settings]].

## How do I switch models?

Use the model selector in the message box, or manage the list under **Settings → Models**. See [[Settings]].

## The agent stopped and is asking me something

That's an **approval** — the agent pauses before doing anything risky and waits for you. Click **Allow** to continue or **Reject** to cancel that step. It won't time out. See [[Using FutureOS|Using-FutureOS]].

## Where are my conversations and settings stored?

In a `.future` folder in your home directory:

- macOS / Linux: `~/.future`
- Windows: `C:\Users\<you>\.future`

## How do I update?

Download the latest version from the **[Releases page](https://github.com/futuregene/future-os/releases)** and install it over the old one. Your data in `.future` is preserved. See [[Installation]].

## How do I uninstall / remove my data?

Remove the app (delete `FutureOS.app`, uninstall from Windows Settings, or remove the `.deb`). To also erase your data, delete the `.future` folder above. See [[Installation]].

## What platforms are supported?

macOS, Windows, and Linux. Download the right file for your system from the [Releases page](https://github.com/futuregene/future-os/releases).

---

Still stuck? Open an issue on the [project's GitHub](https://github.com/futuregene/future-os/issues), and include a screenshot of any error.
