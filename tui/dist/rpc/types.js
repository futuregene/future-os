/**
 * JSON-RPC types for xihu_tui Agent communication.
 * Mirrors internal/rpc/types.go on the Go server side.
 */
// ============================================================================
// Specific command creators (for type safety at call sites)
// ============================================================================
export function promptCmd(message, images, streamingBehavior) {
    return { type: "prompt", message, images, streamingBehavior };
}
export function steerCmd(message) {
    return { type: "steer", message };
}
export function followUpCmd(message) {
    return { type: "follow_up", message };
}
