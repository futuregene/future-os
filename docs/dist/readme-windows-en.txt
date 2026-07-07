FutureOS Portable Setup Guide (Windows)
======================================

[Launch]
Extract the entire archive to any folder (e.g. D:\FutureOS) and double-click
FutureOS.exe to run — no installation needed.
Note: FutureOS.exe and future-agent.exe must stay in the same folder; do not
move them separately. future-agent.exe is the backend service and launches
automatically on startup.

[First Run]
· If you see "Windows protected your PC" (SmartScreen): click "More info"
  → "Run anyway".
· If the window opens but says "backend service not connected": the downloaded
  archive was flagged as "from the Internet". Recommended fix: before
  extracting, right-click the .zip → "Properties" → check "Unblock" at the
  bottom → OK → extract. Or, after extracting, open PowerShell in that folder
  and run:
    Get-ChildItem -Recurse | Unblock-File

[Runtime]
Requires the Microsoft Edge WebView2 Runtime (included on recent Win10 / Win11).
If you see no window or a missing-component message after double-clicking,
install "Microsoft Edge WebView2 Runtime" (Evergreen) from the Microsoft
website and try again.

[Notes]
· An internet connection is required for the first-time login.
  Personal data is stored in C:\Users\<username>\.future.
· Closing the window quits the app; the background future-agent service stops
  automatically.
· The command-line tool future.exe is included in the same directory.

If you encounter any issues, please send us a screenshot of the error window.
