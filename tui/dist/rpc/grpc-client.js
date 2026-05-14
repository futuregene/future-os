/**
 * gRPC client for xihu_tui Agent.
 * Uses @grpc/grpc-js with proto descriptor.
 * Only supports gRPC (no JSON-RPC or Unix socket).
 */
import * as grpc from "@grpc/grpc-js";
import * as protoLoader from "@grpc/proto-loader";
// Load proto descriptor
const PROTO_PATH = process.env.XIHU_PROTO_PATH ?? "/Users/geilige/xihu_tui/proto/proto/xihu_tui.proto";
// ─── Proto Setup ─────────────────────────────────────────────────────────
const packageDefinition = protoLoader.loadSync(PROTO_PATH, {
    keepCase: false,
    longs: String,
    enums: String,
    defaults: true,
    oneofs: true,
});
const protoDescriptor = grpc.loadPackageDefinition(packageDefinition);
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const proto = protoDescriptor.proto;
// ─── RPC Client ─────────────────────────────────────────────────────────
export class GrpcClient {
    client;
    eventListeners = [];
    streamCall = null;
    connected = false;
    currentSessionId = "";
    constructor(address = "localhost:50051") {
        const credentials = grpc.credentials.createInsecure();
        this.client = new proto.XihuAgent(address, credentials);
    }
    // ─── Session Management ───────────────────────────────────────────────
    getCurrentSessionId() {
        return this.currentSessionId;
    }
    setCurrentSessionId(sessionId) {
        this.currentSessionId = sessionId;
    }
    // ─── Event Streaming ─────────────────────────────────────────────────
    connectEvents() {
        if (this.streamCall)
            return;
        this.streamCall = this.client.StreamEvents({
            session_id: this.currentSessionId,
        });
        this.streamCall.on("data", (response) => {
            try {
                const event = {
                    type: response.type || "message",
                    ...(typeof response.data === "string" ? JSON.parse(response.data) : response.data),
                };
                for (const listener of this.eventListeners) {
                    try {
                        listener(event);
                    }
                    catch {
                        // Ignore listener errors
                    }
                }
            }
            catch {
                // Ignore parse errors
            }
        });
        this.streamCall.on("end", () => {
            this.streamCall = null;
            // Reconnect after delay
            setTimeout(() => this.connectEvents(), 2000);
        });
        this.streamCall.on("error", (err) => {
            console.error("Stream error:", err);
            this.streamCall = null;
        });
        this.connected = true;
    }
    isConnected() {
        return this.connected;
    }
    subscribe(listener) {
        this.connectEvents();
        this.eventListeners.push(listener);
        return () => {
            this.eventListeners = this.eventListeners.filter((l) => l !== listener);
        };
    }
    disconnect() {
        this.streamCall?.cancel();
        this.streamCall = null;
    }
    // ─── RPC Call Helper ─────────────────────────────────────────────────
    async call(type, cmd) {
        return new Promise((resolve, reject) => {
            const request = {
                id: String(Date.now()),
                type,
                sessionId: this.currentSessionId || undefined,
                ...cmd,
            };
            this.client.ExecuteCommand(request, (err, response) => {
                if (err) {
                    reject(err);
                    return;
                }
                if (!response.success) {
                    reject(new Error(response.error || "unknown error"));
                    return;
                }
                if (response.data && typeof response.data === "string") {
                    try {
                        resolve(JSON.parse(response.data));
                    }
                    catch {
                        resolve(response.data);
                    }
                }
                else {
                    resolve(response.data);
                }
            });
        });
    }
    // ─── Session Management RPC Methods ──────────────────────────────────
    async newSession() {
        const result = await this.call("new_session", {});
        if (result?.sessionId) {
            this.currentSessionId = result.sessionId;
        }
        return result || { cancelled: false };
    }
    async switchSession(sessionId) {
        const result = await this.call("switch_session", { sessionId });
        if (result && !result.cancelled) {
            this.currentSessionId = sessionId;
        }
        return result || { cancelled: false };
    }
    async fork(entryId) {
        const result = await this.call("fork", { entryId });
        if (result?.sessionId) {
            this.currentSessionId = result.sessionId;
        }
        return result || { text: "", cancelled: true };
    }
    async clone() {
        const result = await this.call("clone", {});
        if (result?.sessionId) {
            this.currentSessionId = result.sessionId;
        }
        return result || { cancelled: true };
    }
    async getForkMessages() {
        return this.call("get_fork_messages", {});
    }
    async getLastAssistantText() {
        return this.call("get_last_assistant_text", {});
    }
    async setSessionName(name) {
        await this.call("set_session_name", { name });
    }
    async listSessions() {
        return this.call("list_sessions", {});
    }
    async deleteSession(sessionId) {
        return this.call("delete_session", { sessionId });
    }
    // ─── Core RPC Methods ────────────────────────────────────────────────
    async prompt(message, images, streamingBehavior) {
        await this.call("prompt", { message, images, streamingBehavior });
    }
    async steer(message) {
        await this.call("steer", { message });
    }
    async followUp(message) {
        await this.call("follow_up", { message });
    }
    async abort() {
        await this.call("abort", {});
    }
    async getState() {
        return this.call("get_state", {});
    }
    async getMessages() {
        return this.call("get_messages", {});
    }
    async setModel(modelId) {
        await this.call("set_model", { modelId });
    }
    async cycleModel() {
        return this.call("cycle_model", {});
    }
    async getAvailableModels() {
        return this.call("get_available_models", {});
    }
    async setThinkingLevel(level) {
        await this.call("set_thinking_level", { level });
    }
    async cycleThinkingLevel() {
        return this.call("cycle_thinking_level", {});
    }
    async setSteeringMode(mode) {
        await this.call("set_steering_mode", { mode });
    }
    async setFollowUpMode(mode) {
        await this.call("set_follow_up_mode", { mode });
    }
    async compact(customInstructions) {
        return this.call("compact", { customInstructions });
    }
    async setAutoCompaction(enabled) {
        await this.call("set_auto_compaction", { enabled });
    }
    async setAutoRetry(enabled) {
        await this.call("set_auto_retry", { enabled });
    }
    async abortRetry() {
        await this.call("abort_retry", {});
    }
    async bash(command) {
        return this.call("bash", { command });
    }
    async abortBash() {
        await this.call("abort_bash", {});
    }
    async getSessionStats() {
        return this.call("get_session_stats", {});
    }
    async exportHtml(outputPath) {
        return this.call("export_html", { outputPath });
    }
    async getCommands() {
        return this.call("get_commands", {});
    }
}
