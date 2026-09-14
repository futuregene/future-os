# macOS / Linux sandbox security audit — 2026-09-14

Scope: the agent's shared path rules, macOS Seatbelt compiler and Linux Bubblewrap helper. This is a targeted implementation review and regression exercise, not a certification of all sandbox escape classes or release platforms.

## Findings and fixes

| Finding | Impact | Fix / regression |
|---|---|---|
| macOS regex literals were escaped as ordinary SBPL strings | `*.pem`, `*.key`, `.env.*`, etc. could fail to match even ordinary lowercase filenames. Existing smoke tests mostly checked literal `.env`, so they missed this. | Pass the escaped string to `(regex "...")`, rather than putting a second escape layer inside `#"..."`. Kernel tests check denial, empty stdout, and absence of newly created files. |
| RegexBuilder's case-insensitive flag was absent from the emitted SBPL source | ASCII case variants such as `secret.PEM` could bypass the glob guard. | Emit explicit ASCII case classes. Test uppercase extensions and `.ENV.PRODUCTION`. This does not claim full Unicode case-fold equivalence between Rust and Seatbelt. |
| Shared `**` used a dot that excludes newline | Native file tools and Seatbelt could miss a secret beneath a newline-containing directory. Linux's separate scanner already matched newlines. | Encode newline-inclusive repetition in syntax understood by both engines; shared rule and real Seatbelt regressions. |
| macOS ignored rule-file resolution errors | A corrupt/unreadable policy or invalid rule entry could remove an intended restriction while still launching the command. | Production preparation returns an infrastructure error; the public profile builder returns a deny-all profile. Explicitly approved unsandboxed execution remains separate. |
| Linux FD cleanup stopped at `min(_SC_OPEN_MAX, 65536)` | High-numbered mount/source descriptors could survive inner cleanup. The outer old-kernel fallback had the same issue, including after RLIMIT_NOFILE was lowered. An open descriptor can bypass pathname/mount restrictions. | Enumerate actual `/proc/self/fd` entries, independent of limits. Snapshot before fork for the CLOEXEC fallback; propagate cleanup errors. An isolated child test opens FD 70000 where allowed, lowers its limit to 1024, and verifies fallback CLOEXEC, closing, and the retained allowlist. |

The FD finding is a boundary-cleanup defect demonstrated in isolation, not a claim of a default-configuration end-to-end exploit. Existing production smoke tests continue to exercise capability dropping, `no_new_privs`, private reports, mount protections and process cleanup.

## Local evidence

- macOS 26.6.2 (25G83), arm64, Rust 1.97.0: real Seatbelt security regressions **3/3 PASS**, existing ignored Seatbelt smoke **9/9 PASS**.
- Docker Desktop Linux VM kernel `6.12.76-linuxkit`, aarch64; Debian trixie, Bubblewrap **0.12.0**, Rust **1.97.0**: ignored Bubblewrap smoke **11/11 PASS**, with no unavailable-backend skips. Tests include the actual outer → bwrap → inner → shell flow.
- Full `cargo test -p future-agent`: macOS **1768 library tests PASS**, Linux **1762 library tests PASS**, and all respective integration targets PASS. Each leaves the explicitly ignored large-workspace test out of the default run; platform smoke tests were run separately as above. `cargo fmt -p future-agent --check` and `cargo clippy -p future-agent --all-targets -- -D warnings` passed on both platforms.
- The original macOS smoke invocation exposed an inherited `CARGO_TARGET_DIR` fixture issue: cargo tried writing to the parent runner's build directory outside the sandbox. The helper now removes that variable for its disposable workspace, and the rerun passed. This was not a sandbox bypass.
- Fresh Docker defaults correctly refused user namespaces. The test container alone was recreated with `--cap-add SYS_ADMIN --security-opt seccomp=unconfined --security-opt systempaths=unconfined` to allow nested Bubblewrap namespaces and a fresh proc mount. Host source was mounted read-only; build/test output used a dedicated disposable Docker volume. The first full-suite attempt as container root on the read-only checkout had five environmental failures (three tests write in the checkout, two require ordinary-user DAC denial). Repeating the complete suite and all 11 Bubblewrap smoke tests as an unprivileged `audit` user on a writable container-only source copy passed. No host policy, existing agent or credentials were changed.

Container results verify the Linux kernel/helper behavior in this environment; they do **not** replace native distro, x86-64, desktop or installation-package certification. The small offline `seatbelt_security` suite runs automatically with `cargo test -p future-agent` on macOS. Wiring it into the current macOS CI build job is deferred: the available GitHub credential lacks `workflow` scope, and the workflow change was removed rather than broadening credentials. Existing CI Linux tests cannot validate SBPL kernel behavior; use the macOS command below.

## Reproduction

```sh
cargo test -p future-agent --test seatbelt_security
cargo test -p future-agent --test sandbox_smoke -- --ignored --test-threads=1 --nocapture
cargo test -p future-agent --lib sandbox::
cargo test -p future-agent --test linux_sandbox_smoke -- --ignored --test-threads=1 --nocapture
cargo fmt -p future-agent --check
cargo clippy -p future-agent --all-targets -- -D warnings
cargo test -p future-agent
```

The first two are macOS-only; the Linux smoke must show real executions, not `skipping Linux sandbox smoke`. On macOS raise the runner's file-descriptor limit before the full agent suite; use a disposable HOME for full-suite credential isolation.

## Remaining boundaries (not fixed or newly promised)

The previously documented product decisions remain: unrestricted network; broad macOS Mach/IPC permissions; temporarily readable agent `auth.json`; explicit approval unwraps the entire command; Linux has no seccomp filter, and missing protected paths/new glob matches receive detection-only rescans, not prevention or rollback. Linux complex deny/reopen combinations and missing allow roots may fail preparation. The root-owned Bubblewrap path-chain hardening, full Unicode case equivalence, concurrent alias/inode mutation and broader IPC attack surfaces are not certified by this pass. See the existing platform design documents under `desktop/DEV_MD/SANDBOX/` for those accepted limitations; this report supersedes their historical macOS regex/malformed-policy behavior and Linux FD-cleanup descriptions only.
