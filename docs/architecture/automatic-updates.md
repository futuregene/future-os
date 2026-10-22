# Automatic update transactions

The desktop update coordinator owns a durable transaction. Platform adapters
prepare an executor while the application is still usable, and receive explicit
commit authority only after the shutdown barrier succeeds. Download signature
verification remains mandatory. Discovery is independent of whether the current
installation supports replacement; portable and protected installs still offer
manual downloads.

## Protocol and recovery

`checking → downloading → verified → preparing → prepared → cleanup →
awaiting-exit/installing → awaiting-validation → finished`

The versioned JSON record remains compatible with the previous updater. Rust
validates stage names and rejects backwards transitions. A process-wide flag and
an OS file lease serialize transactions; an independent executor holds a second
observer lease through installation. A live observer is never mistaken for a
crashed operation.

On the next status query, an abandoned record through `cleanup` becomes
`not-installed`: no adapter can replace files until its commit stage. Abandoned
replacement remains interrupted and blocks another install. A user repair
acknowledgement is recorded as an acknowledgement, never installation success.

Before an idle installation, terminal admission is reserved reversibly and the
owned Agent atomically compares its activity revision and closes admission. A new
CLI command or terminal prevents silent interruption. Confirmed interruption
still follows the shared shutdown barrier. User cancellation drops the prepared
executor without granting commit authority.

`pending-validation` means replacement completed but a restart is needed. Only a
process whose compiled version matches the transaction target can confirm GUI
startup and Agent readiness. Platform checks run before confirmation. The old
process cannot confirm the new version. An executor holds the existing Agent
instance lease during Unix replacement; another host starting in the gap makes
the update fail before program files change. Windows NSIS retains its lease.

## User experience

The settings page shows versions, download progress and the next useful action.
Journal identifiers, executable paths, internal stages and raw diagnostics stay
in maintenance records and logs. The stopped update screen follows the same rule.
Failures distinguish retrying, restarting a safely restored version and
reinstalling through an installer; a failed update check still leaves the official
website available for recovery. Native confirmations use the selected UI language
and the same translation dictionary. Deferring an installation records a normal
`cancelled` outcome without an error banner. Recovery uses one screen, without a
second native error dialog.

## Platform adapters

| Platform | Preparation | Commit and failure semantics |
| --- | --- | --- |
| Windows NSIS | Launch an independent PowerShell observer, verify program readability and create a persistent backup before readiness. Inspect Job membership and use explicit file handles for detached-process I/O. | A durable commit marker and the exact parent process handle authorize installation after exit. NSIS validates executables; observer restoration verifies the complete original snapshot. Unrecognized files are retained, so an incomplete rollback requires repair. Keep the backup until startup confirmation. |
| macOS app | Unpack beside the installed bundle on the same volume; verify code signature, executable identity and bounded GUI/CLI help probes; sync staged files; prepare a native helper. Protected directories offer a manual installer. | `renameatx_np(RENAME_SWAP)` atomically exchanges complete bundles. The installed pathname never disappears. Failure before exchange leaves it intact; a journal-persistence failure attempts an exchange back. Retain the previous bundle until matching healthy startup. |
| Debian family DEB | One `pkexec` authorization prepares a native executor. It makes a private root-owned copy, checks its SHA-256, independently verifies its signature with the compiled updater public key and checks package identity/version, and performs a non-installing dpkg check. | EOF cancels without install; COMMIT permits dpkg. User-controlled journal/lease paths are accessed with the caller's effective identity, and root identity is permanently dropped before publishing results. A successful exit also requires the expected configured package version. Package failure requires repair, never an inference from two executable hashes. No password capture or authentication fallback. |

## Data and backup boundaries

Application rollback does not imply database rollback. A new process may have
started migrations before a later failure. Therefore startup failures retain the
old bundle/Windows backup and transaction evidence; they do not automatically
launch old code against potentially migrated data. Use the platform installer or
package manager to repair the installation. This is intentionally distinct from
a failed pre-start replacement, where intact originals can be safely relaunched.

macOS keeps its previous bundle at `<install-parent>/.futureos-<operation-id>/candidate.app`.
Windows keeps its snapshot under `<FUTURE_HOME>/app/updates/<operation-id>/program-backup`.
Results and native diagnostics remain under the operation directory or
`app/logs/update-recovery.log`. Unknown outcomes keep recovery evidence.

## Acceptance

Local validation is compilation, type checking, linting and formatting only.
Regular CI runs these unit tests:

- Agent admission revision races and reversible terminal admission.
- Interrupted preparation, invalid state regressions and version-bound startup confirmation.
- Prepared helper cancellation and explicit commit, without a real installation.
- macOS atomic exchange/reversal and failure preserving the original directory.
Native installation fixtures run through the explicitly dispatched Misc
`windows-update` and `linux-update` suites:

- Windows NSIS preflight/observer, full backup recovery and incomplete restoration.
- Linux authorized native executor with disposable DEBs: EOF cancellation,
  conflicting Agent lock, success awaiting validation, and failed configuration.
  The script refuses non-CI execution, signs fixtures with an ephemeral CI identity embedded in the test build, and purges only its fixture package.

Signed macOS updater artifact validation is a publication requirement. These
fixtures do not substitute for real signed old-to-new acceptance on Windows and
macOS, authorization-agent coverage on Linux, or database migration compatibility
validation. No such real-install acceptance is claimed by a static check.
