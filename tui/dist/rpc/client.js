/**
 * JSON-RPC client for xihu_tui Agent.
 * Supports both TCP (http://host:port) and Unix socket.
 * Also handles SSE event streaming.
 */
import http from "node:http";
import https from "node:https";
import { URL } from "node:url";
// ─── RPC Client ─────────────────────────────────────────────────────────
export class RpcClient {
    requestId = 0;
    eventListeners = [];
    connected = false;
    // For TCP connections
    host;
    port;
    path = "/";
    useTLS = false;
    // For Unix socket
    socketPath = null;
    // SSE connection
    eventsource = null;
    constructor(baseUrl = "http://localhost:7890") {
        const url = new URL(baseUrl);
        this.host = url.hostname;
        this.port = parseInt(url.port) || (url.protocol === "https:" ? 443 : 80);
        this.path = url.pathname;
        this.useTLS = url.protocol === "https:";
        this.socketPath = process.env.XIHU_SOCKET ?? null;
    }
    // ─── SSE Events ──────────────────────────────────────────────────────
    /**
     * Connect to the SSE event stream for real-time agent events.
     * Automatically called when subscribing to events.
     */
    connectEvents() {
        if (this.eventsource)
            return;
        this.eventsource = new SSEConnection(this.socketPath ?? undefined, this.host !== "localhost" ? `${this.useTLS ? "https" : "http"}://${this.host}:${this.port}` : undefined, (event) => {
            for (const listener of this.eventListeners) {
                try {
                    listener(event);
                }
                catch {
                    // Ignore listener errors
                }
            }
        });
    }
    isConnected() {
        return this.connected;
    }
    subscribe(listener) {
        // Ensure SSE is connected
        this.connectEvents();
        this.eventListeners.push(listener);
        return () => {
            this.eventListeners = this.eventListeners.filter((l) => l !== listener);
        };
    }
    disconnect() {
        this.eventsource?.close();
        this.eventsource = null;
    }
    // ─── HTTP Request ───────────────────────────────────────────────────
    async request(body) {
        return new Promise((resolve, reject) => {
            const options = {
                socketPath: this.socketPath ?? undefined,
                hostname: this.socketPath ? undefined : this.host,
                port: this.socketPath ? undefined : this.port,
                path: this.path,
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                    "Content-Length": Buffer.byteLength(body),
                },
            };
            const transport = this.useTLS ? https : http;
            const req = transport.request(options, (res) => {
                let data = "";
                res.on("data", (chunk) => (data += chunk));
                res.on("end", () => {
                    this.connected = true;
                    resolve(data);
                });
            });
            req.on("error", reject);
            req.write(body);
            req.end();
        });
    }
    async send(cmd) {
        const full = { ...cmd, id: String(++this.requestId) };
        const body = JSON.stringify(full);
        const raw = await this.request(body);
        return JSON.parse(raw);
    }
    async call(type, cmd) {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const resp = (await this.send(cmd));
        if (!resp.success) {
            throw new Error(resp.error ?? "unknown error");
        }
        return resp.data;
    }
    // ─── RPC Methods ─────────────────────────────────────────────────────
    async prompt(message, images, streamingBehavior) {
        await this.call("prompt", { type: "prompt", message, images, streamingBehavior });
    }
    async steer(message) {
        await this.call("steer", { type: "steer", message });
    }
    async followUp(message) {
        await this.call("follow_up", { type: "follow_up", message });
    }
    async abort() {
        await this.call("abort", { type: "abort" });
    }
    async newSession() {
        return this.call("new_session", { type: "new_session" });
    }
    async getState() {
        return this.call("get_state", { type: "get_state" });
    }
    async getMessages() {
        return this.call("get_messages", { type: "get_messages" });
    }
    async setModel(modelId) {
        await this.call("set_model", { type: "set_model", modelId });
    }
    async cycleModel() {
        return this.call("cycle_model", { type: "cycle_model" });
    }
    async getAvailableModels() {
        return this.call("get_available_models", { type: "get_available_models" });
    }
    async setThinkingLevel(level) {
        await this.call("set_thinking_level", { type: "set_thinking_level", level });
    }
    async cycleThinkingLevel() {
        return this.call("cycle_thinking_level", { type: "cycle_thinking_level" });
    }
    async setSteeringMode(mode) {
        await this.call("set_steering_mode", { type: "set_steering_mode", mode });
    }
    async setFollowUpMode(mode) {
        await this.call("set_follow_up_mode", { type: "set_follow_up_mode", mode });
    }
    async compact(customInstructions) {
        return this.call("compact", { type: "compact", customInstructions });
    }
    async setAutoCompaction(enabled) {
        await this.call("set_auto_compaction", { type: "set_auto_compaction", enabled });
    }
    async setAutoRetry(enabled) {
        await this.call("set_auto_retry", { type: "set_auto_retry", enabled });
    }
    async abortRetry() {
        await this.call("abort_retry", { type: "abort_retry" });
    }
    async bash(command) {
        return this.call("bash", { type: "bash", command });
    }
    async abortBash() {
        await this.call("abort_bash", { type: "abort_bash" });
    }
    async getSessionStats() {
        return this.call("get_session_stats", { type: "get_session_stats" });
    }
    async exportHtml(outputPath) {
        return this.call("export_html", { type: "export_html", outputPath });
    }
    async switchSession(sessionPath) {
        return this.call("switch_session", { type: "switch_session", sessionPath });
    }
    async fork(entryId) {
        return this.call("fork", { type: "fork", entryId });
    }
    async clone() {
        return this.call("clone", { type: "clone" });
    }
    async getForkMessages() {
        return this.call("get_fork_messages", { type: "get_fork_messages" });
    }
    async getLastAssistantText() {
        return this.call("get_last_assistant_text", { type: "get_last_assistant_text" });
    }
    async setSessionName(name) {
        await this.call("set_session_name", { type: "set_session_name", name });
    }
    async listSessions() {
        return this.call("list_sessions", { type: "list_sessions" });
    }
    async deleteSession(sessionId) {
        return this.call("delete_session", { type: "delete_session", sessionId });
    }
    async getCommands() {
        return this.call("get_commands", { type: "get_commands" });
    }
}
// ─── SSE Connection ─────────────────────────────────────────────────────
/**
 * Server-Sent Events client using Node.js HTTP.
 * Connects to GET /events on the xihu_tui server and parses SSE data.
 */
class SSEConnection {
    socketPath;
    baseUrl;
    onEvent;
    req = null;
    buffer = "";
    constructor(socketPath, baseUrl, onEvent) {
        this.socketPath = socketPath;
        this.baseUrl = baseUrl;
        this.onEvent = onEvent;
        this.connect();
    }
    connect() {
        const options = {
            socketPath: this.socketPath,
            hostname: this.socketPath ? undefined : (this.baseUrl ? new URL(this.baseUrl).hostname : "localhost"),
            port: this.socketPath ? undefined : (this.baseUrl ? new URL(this.baseUrl).port : 7890),
            path: "/events",
            method: "GET",
            headers: {
                "Accept": "text/event-stream",
                "Cache-Control": "no-cache",
            },
        };
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const transport = this.baseUrl?.startsWith("https") ? https : http;
        this.req = transport.request(options, (res) => {
            res.on("data", (chunk) => {
                this.buffer += chunk;
                this.processBuffer();
            });
            res.on("end", () => {
                // Connection closed
            });
        });
        this.req.on("error", () => {
            // Retry connection after delay
            setTimeout(() => this.connect(), 2000);
        });
        this.req.end();
    }
    processBuffer() {
        // SSE format: "event: TYPE\ndata: JSON\n\n"
        // Extract complete events (delimited by \n\n)
        const events = this.buffer.split("\n\n");
        this.buffer = events.pop() ?? ""; // Keep incomplete last chunk
        for (const raw of events) {
            this.parseEvent(raw);
        }
    }
    parseEvent(raw) {
        let eventType = "message";
        let data = "";
        for (const line of raw.split("\n")) {
            if (line.startsWith("event: ")) {
                eventType = line.slice(7).trim();
            }
            else if (line.startsWith("data: ")) {
                data = line.slice(6).trim();
            }
        }
        if (!data)
            return;
        try {
            const event = JSON.parse(data);
            // Override type with SSE event type
            if (eventType !== "message") {
                event.type = eventType;
            }
            this.onEvent(event);
        }
        catch {
            // Ignore parse errors
        }
    }
    close() {
        this.req?.destroy();
        this.req = null;
    }
}
