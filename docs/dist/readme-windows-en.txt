FutureOS Portable Setup Guide (Windows)
======================================

[Launch]
Extract the entire archive to any folder (e.g. D:\FutureOS) and double-click
FutureOS.exe to run — no installation needed.
Note: FutureOS.exe and future.exe must stay in the same folder; do not move
them separately. Desktop hosts the complete Agent in its own process by default;
future.exe remains available for skills, maintenance and independent CLI use.

[First Run]
· SmartScreen reputation warnings can occur even on signed builds. Verify the
  official source/publisher first; choose "More info" → "Run anyway" only for
  a trusted artifact. Do not bypass an unexpected signature/publisher mismatch.
· For an incompatible backend or held instance lock, identify GUI/host or
  independently managed Agents using the same data root. Exit or upgrade the
  owner, then reopen. Never delete the lock. Desktop does not start a second
  Agent when an external service is incompatible or its lock owner unavailable.

[Runtime]
Requires the Microsoft Edge WebView2 Runtime (included on recent Win10 / Win11).
If you see no window or a missing-component message after double-clicking,
install "Microsoft Edge WebView2 Runtime" (Evergreen) from the Microsoft
website and try again.

[Notes]
· An internet connection is required for the first-time login; your own provider
  can be configured in Settings instead.
  Personal data is stored in C:\Users\<username>\.future.
· Exiting an embedded owner cancels every client's active, queued and approval
  work, including CLI/TUI conversations; GUI prompts when work exists. Attached
  external Agents survive; Desktop cancels only its own submitted runs.
· The desktop defaults to Full access; select approvals/write protection in
  Settings. Online features still send requests to their corresponding services.
· The command-line tool future.exe is included in the same directory.

[License]
FutureOS is distributed under the MIT License; the bundled future goal
component is derived from LoopX and distributed under Apache-2.0.
Full license texts and attribution notices: see the licenses/ directory.

If you encounter any issues, please send us a screenshot of the error window.
