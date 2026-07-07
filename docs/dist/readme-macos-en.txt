FutureOS Setup Guide (macOS)
===========================

[Installation]
Drag the FutureOS icon from this window into the "Applications" folder.

[First Launch]
This build is not Apple-notarised, so double-clicking will be blocked with
a "unidentified developer" or "damaged" warning. This is expected. Choose one:
  A (recommended) Right-click (or Control-click) FutureOS in Applications
    → "Open" → click "Open" again in the dialog. You can then double-click
    normally going forward.
  B (if it says "damaged") Open Terminal and run the following, then
    double-click again:
    xattr -dr com.apple.quarantine /Applications/FutureOS.app

[Notes]
· An internet connection is required for the first-time login (in the app).
· Personal data is stored in ~/.future. The background future-agent service
  stops automatically when you quit the app.
· The command-line tool future is included at
  FutureOS.app/Contents/MacOS/future.

If you encounter any issues, please send us a screenshot of the error dialog.
