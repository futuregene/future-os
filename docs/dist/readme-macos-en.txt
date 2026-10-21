FutureOS Setup Guide (macOS)
===========================

[Installation]
Drag the FutureOS icon from this window into the "Applications" folder.

[First Launch — unsigned test packages only]
This note accompanies unsigned test DMGs, not signed/notarized official releases.
Verify the source first. Unexpected warnings on a signed release should be reported
and the artifact redownloaded, not bypassed.
This test build is not Apple-notarised, so double-clicking will be blocked with
a "unidentified developer" or "damaged" warning. This is expected. Choose one:
  A (recommended) Right-click (or Control-click) FutureOS in Applications
    → "Open" → click "Open" again in the dialog. You can then double-click
    normally going forward.
  B (if it says "damaged") Open Terminal and run the following, then
    double-click again:
    xattr -dr com.apple.quarantine /Applications/FutureOS.app

[Notes]
· An internet connection is required for the first-time login (in the app).
· Personal data is stored in ~/.future. Desktop hosts a complete in-process Agent
  by default and can attach to a compatible external service.
· Exiting an embedded owner cancels every client's work, including CLI/TUI;
  GUI prompts for active work. Attached external Agents survive; Desktop cancels
  only its own submitted runs.
· Incompatible or unavailable lock owners fail. Exit or upgrade that service;
  never delete its lock to bypass the check.
· The desktop defaults to Full access; select approvals/sandboxing in Settings.
  Online features still send requests to their corresponding services.
· The command-line tool future is included at
  FutureOS.app/Contents/MacOS/future.

[License]
FutureOS is distributed under the MIT License; the bundled future goal
component is derived from LoopX and distributed under Apache-2.0.
Full license texts and attribution notices: see the licenses/ directory.

If you encounter any issues, please send us a screenshot of the error dialog.
