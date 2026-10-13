/**
 * How much history a client asks the host for, and where "the newest" starts.
 *
 * These are facts about the *host's* cursor, not choices either client gets to
 * make on its own, which is why they live here rather than in each client: the
 * phone (`mobile/src/remote/timeline/historyPaging.ts`) and the desktop
 * (`desktop/src/features/remote-peer/useRemoteTimeline.ts`) are separate
 * implementations of separate clients on purpose — the desktop's has to survive
 * a minimised window, which the phone's does not — but they must agree on the
 * contract they speak. They diverged once (100 against 3) precisely because
 * neither side could see the other.
 */

/**
 * The host's page size for a backward history read, in *user exchanges* (not
 * entries) — the unit the host's cursor counts in. Asking in entries would
 * misalign the cursor.
 *
 * Three, and the difference is not cosmetic: the desktop client once asked for
 * **100** and the host answered with the whole conversation. Every open paid for
 * a hundred exchanges of history before the first line could be drawn — on a
 * long conversation, the reason a remote one took seconds to appear where the
 * phone took none. Older pages arrive as the reader scrolls up, so the cost of
 * the extra history is only paid for history that is actually looked at.
 *
 * An exchange can contain hundreds of tool steps, so this also bounds the first
 * paint.
 */
export const HISTORY_PAGE_USER_EXCHANGES = 3;

/**
 * The cursor that means "the page at the end of this session".
 *
 * A history read is *backward*: the host serves the `limit` user exchanges that
 * end at `before`, and only that cursor can name the tail — there is no offset
 * a client could pass to mean "last". Leaving `before` off does not mean the
 * newest page; it is the host's other read (a forward page from offset 0), so
 * the client was handed the *oldest* entries of a conversation as its first
 * page: one question, its reply's first frames, and nothing else. The page after
 * it then repeated the same ordinals, which is why "load earlier" showed the
 * question and the reply a second time.
 *
 * `Number.MAX_SAFE_INTEGER` is the same value the host documents as its first
 * paint (`NEWEST_PAGE_CURSOR` in
 * `desktop/src-tauri/src/remote_host/business/history.rs`): anything at or above
 * it means "the end of the session", anything below it is a scrolled-up page.
 */
export const HISTORY_TAIL_CURSOR = Number.MAX_SAFE_INTEGER;
