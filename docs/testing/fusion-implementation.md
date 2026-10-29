# Fusion implementation verification

Date: 2026-10-29. Development models: Future DeepSeek Flash, GLM 5.3 Flash and Qwen 3.7 Plus; moderator: DeepSeek Flash.

## Isolated live exercise

A separate temporary FutureOS home, IPC endpoint and workspace were used. The user task explicitly required one Fusion consultation on SQLite versus an external database, with `read_scope: ["facts.txt"]`, followed by a parent-written decision file only on success.

Observed database evidence:

- One successful consultation reached `completed` with `full` completeness.
- Three independent analyses, one issue mapping, three reviews and one final synthesis completed, using exactly the configured model references.
- The main Agent received the final tool result and wrote `verified-decision.txt` in the temporary workspace.
- Original `facts.txt` SHA-256 remained `0defa612947854d6be7c6f079b37d4ae6eba6c635012c769a5d7b913d6a049c9`.
- `future fusion show <parent-session-id> --json` returned all eight steps. Original step paging returned an 8554-byte transcript containing a snapshot `read` call.
- Offline backup and verification passed with seven Fusion database/snapshot files. Restore completed, and the original parent still returned all eight completed steps with full completeness under the new restore identity.
- An earlier attempt exposed an unsupported `reject_if_busy` submission value; it was corrected to the existing `enqueue_if_busy` contract. Its failure attempts remained individually diagnosable rather than appearing as a completed discussion.

## Verification boundaries

Local checks cover Rust fmt/Clippy with all targets, Desktop/Mobile type-check and ESLint, and documentation validation. Regression tests are authored for command boundaries, immutable snapshots, scope/link/tool rejection, idempotent admission, exclusive claiming, cancellation fencing and moderator validation. Per repository instructions, unit tests are not executed locally; CI owns their execution.

Screenshot fixtures exercise the shipping Desktop and React Native components with fixed discussion data. They validate presentation and navigation, not provider orchestration. Windows/Linux builds and Android/iOS device execution remain CI/device verification; this macOS exercise does not claim those devices were run.
