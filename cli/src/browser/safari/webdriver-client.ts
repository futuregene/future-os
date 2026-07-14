/**
 * Lightweight W3C WebDriver HTTP client.
 *
 * Uses Bun fetch() — zero npm dependencies.
 * All WebDriver error responses are converted to structured errors
 * preserving HTTP status, WebDriver error code, message, and stacktrace.
 */
export interface WebDriverError {
  httpStatus: number;
  error: string;
  message: string;
  stacktrace?: string;
}

export class WebDriverErrorResponse extends Error {
  constructor(public readonly wd: WebDriverError) {
    super(`WebDriver [${wd.httpStatus}] ${wd.error}: ${wd.message}`);
    this.name = "WebDriverErrorResponse";
  }
}

export class WebDriverClient {
  constructor(private baseUrl: string) {}

  // ── Session ────────────────────────────────────────────────────────

  async createSession(capabilities?: Record<string, unknown>): Promise<string> {
    const caps = capabilities ?? { browserName: "safari" };
    const data = await this.post("/session", { capabilities: { alwaysMatch: caps } });
    return data.sessionId as string;
  }

  async deleteSession(sessionId: string): Promise<void> {
    await this.fetch("DELETE", `/session/${sessionId}`);
  }

  // ── Navigation ─────────────────────────────────────────────────────

  async navigateTo(sessionId: string, url: string): Promise<void> {
    await this.post(`/session/${sessionId}/url`, { url });
  }

  async getCurrentUrl(sessionId: string): Promise<string> {
    const data = await this.get(`/session/${sessionId}/url`);
    return data.value as string;
  }

  async getTitle(sessionId: string): Promise<string> {
    const data = await this.get(`/session/${sessionId}/title`);
    return (data.value as string) ?? "";
  }

  async getPageSource(sessionId: string): Promise<string> {
    const data = await this.get(`/session/${sessionId}/source`);
    return (data.value as string) ?? "";
  }

  // ── Execute script ─────────────────────────────────────────────────

  async executeScript<T = unknown>(
    sessionId: string,
    script: string,
    args: unknown[] = [],
  ): Promise<T> {
    const data = await this.post(`/session/${sessionId}/execute/sync`, {
      script,
      args,
    });
    return data.value as T;
  }

  // ── Elements ───────────────────────────────────────────────────────

  async findElement(
    sessionId: string,
    using: string,
    value: string,
  ): Promise<string> {
    const data = await this.post(`/session/${sessionId}/element`, {
      using,
      value,
    });
    // W3C WebDriver returns {element-6066-11e4-a52e-4f735466cecf: "element-id"}
    const elementId = extractElementId(data.value);
    if (!elementId) throw new Error(`Could not extract element ID from response`);
    return elementId;
  }

  async findElements(
    sessionId: string,
    using: string,
    value: string,
  ): Promise<string[]> {
    const data = await this.post(`/session/${sessionId}/elements`, {
      using,
      value,
    });
    if (!Array.isArray(data.value)) return [];
    return data.value.map((v: unknown) => extractElementId(v)).filter(Boolean) as string[];
  }

  async clickElement(sessionId: string, elementId: string): Promise<void> {
    await this.post(`/session/${sessionId}/element/${elementId}/click`);
  }

  async getElementText(sessionId: string, elementId: string): Promise<string> {
    const data = await this.get(`/session/${sessionId}/element/${elementId}/text`);
    return (data.value as string) ?? "";
  }

  async getElementAttribute(
    sessionId: string,
    elementId: string,
    name: string,
  ): Promise<string | null> {
    const data = await this.get(`/session/${sessionId}/element/${elementId}/attribute/${name}`);
    return (data.value as string | null) ?? null;
  }

  async sendKeysToElement(
    sessionId: string,
    elementId: string,
    text: string,
  ): Promise<void> {
    await this.post(`/session/${sessionId}/element/${elementId}/value`, {
      text,
    });
  }

  async clearElement(sessionId: string, elementId: string): Promise<void> {
    await this.post(`/session/${sessionId}/element/${elementId}/clear`);
  }

  async isElementEnabled(sessionId: string, elementId: string): Promise<boolean> {
    const data = await this.get(`/session/${sessionId}/element/${elementId}/enabled`);
    return Boolean(data.value);
  }

  // ── Screenshot ─────────────────────────────────────────────────────

  async takeScreenshot(sessionId: string): Promise<Uint8Array> {
    const data = await this.get(`/session/${sessionId}/screenshot`);
    const base64 = data.value as string;
    return decodeBase64(base64);
  }

  // ── Window / tab management ────────────────────────────────────────

  async getWindowHandles(sessionId: string): Promise<string[]> {
    const data = await this.get(`/session/${sessionId}/window/handles`);
    return (data.value as string[]) ?? [];
  }

  async getCurrentWindowHandle(sessionId: string): Promise<string> {
    const data = await this.get(`/session/${sessionId}/window`);
    return data.value as string;
  }

  async switchToWindow(sessionId: string, handle: string): Promise<void> {
    await this.post(`/session/${sessionId}/window`, { handle });
  }

  async newWindow(sessionId: string): Promise<{ handle: string }> {
    const data = await this.post(`/session/${sessionId}/window/new`, { type: "tab" });
    return { handle: (data.value as Record<string, unknown>).handle as string };
  }

  async closeWindow(sessionId: string): Promise<string[]> {
    const data = await this.fetch("DELETE", `/session/${sessionId}/window`);
    // W3C returns remaining handles
    return (data.value as string[]) ?? [];
  }

  // ── Low-level ──────────────────────────────────────────────────────

  private async get(path: string): Promise<Record<string, unknown>> {
    return this.fetch("GET", path);
  }

  private async post(
    path: string,
    body?: Record<string, unknown>,
  ): Promise<Record<string, unknown>> {
    return this.fetch("POST", path, body);
  }

  private async fetch(
    method: string,
    path: string,
    body?: Record<string, unknown>,
  ): Promise<Record<string, unknown>> {
    const url = `${this.baseUrl}${path}`;
    const options: RequestInit = {
      method,
      headers: { "Content-Type": "application/json; charset=utf-8" },
    };
    if (body) {
      options.body = JSON.stringify(body);
    }

    const response = await fetch(url, options);
    const text = await response.text();
    let data: Record<string, unknown> = {};

    try {
      data = JSON.parse(text) as Record<string, unknown>;
    } catch {
      throw new WebDriverErrorResponse({
        httpStatus: response.status,
        error: "invalid response",
        message: text.slice(0, 200),
      });
    }

    // Check for WebDriver error
    if (data.value && typeof data.value === "object") {
      const val = data.value as Record<string, unknown>;
      if (val.error) {
        throw new WebDriverErrorResponse({
          httpStatus: response.status,
          error: String(val.error),
          message: String(val.message ?? ""),
          stacktrace: typeof val.stacktrace === "string" ? val.stacktrace : undefined,
        });
      }
    }

    return data;
  }
}

// ── Helpers ──────────────────────────────────────────────────────────

const W3C_ELEMENT_KEY = "element-6066-11e4-a52e-4f735466cecf";

function extractElementId(value: unknown): string | null {
  if (typeof value === "string") return value;
  if (value && typeof value === "object") {
    const id = (value as Record<string, unknown>)[W3C_ELEMENT_KEY];
    if (typeof id === "string") return id;
    // Some drivers use "ELEMENT" key (JSON Wire Protocol)
    const legacy = (value as Record<string, unknown>)["ELEMENT"];
    if (typeof legacy === "string") return legacy;
  }
  return null;
}

/** Decode base64 string to Uint8Array. */
function decodeBase64(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}
