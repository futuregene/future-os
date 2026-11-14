// @vitest-environment jsdom
import type { CustomProvider, ThinkingLevelProbeInput, ThinkingLevelProbeOutcome } from "../../integrations/agent/providers";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CustomProviderDialog } from "./CustomProviderDialog";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const LABEL = "Skip TLS certificate verification";

function provider(patch: Partial<CustomProvider> = {}): CustomProvider {
  return {
    id: "gateway",
    name: "Gateway",
    api: "openai-completions",
    baseUrl: "https://gateway.internal/v1",
    skipTlsVerify: false,
    hasApiKey: true,
    models: [],
    ...patch,
  };
}

let root: ReturnType<typeof createRoot>;
afterEach(() => {
  act(() => root.unmount());
  document.body.innerHTML = "";
});

function switchControl(): HTMLButtonElement {
  const found = document.querySelector<HTMLButtonElement>(`button[role="switch"][aria-label="${LABEL}"]`);
  expect(found, "the TLS switch").toBeTruthy();
  return found!;
}

function saveButton(): HTMLButtonElement {
  const found = [...document.querySelectorAll("button")].find(button => button.textContent === "Save");
  expect(found, "button \"Save\"").toBeTruthy();
  return found!;
}

function detectButton(): HTMLButtonElement {
  const found = [...document.querySelectorAll("button")].find(button => button.textContent === "Detect");
  expect(found, "button \"Detect\"").toBeTruthy();
  return found!;
}

function mount(initial: CustomProvider | null, options: {
  onProbe?: (input: ThinkingLevelProbeInput) => Promise<ThinkingLevelProbeOutcome>;
  onSubmit?: (input: unknown) => Promise<void>;
} = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  act(() => root.render(
    <CustomProviderDialog
      existing={[]}
      initial={initial}
      onClose={vi.fn()}
      onProbe={options.onProbe}
      onSubmit={options.onSubmit ?? vi.fn().mockResolvedValue(undefined)}
      open
    />,
  ));
}

describe("skipTlsVerify in the custom-provider editor", () => {
  it("explains what turning it on gives up, not just what it does", () => {
    mount(provider());
    // The setting silently weakens the connection, so the consequence has to be
    // on screen next to the switch rather than in documentation.
    expect(document.body.textContent).toContain(LABEL);
    expect(document.body.textContent).toContain("self-signed certificate");
    expect(document.body.textContent).toContain("encrypted");
    expect(switchControl().getAttribute("aria-checked")).toBe("false");
  });

  it("shows the stored value when editing", () => {
    mount(provider({ skipTlsVerify: true }));
    expect(switchControl().getAttribute("aria-checked")).toBe("true");
  });

  it("treats a provider from a host that predates the field as verified", () => {
    // `undefined`, not `false`: an older host simply never sends it, and the
    // safe reading is "verification is on".
    mount(provider({ skipTlsVerify: undefined }));
    expect(switchControl().getAttribute("aria-checked")).toBe("false");
  });

  it("sends the switch state on save, including when it is off", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    mount(provider({ skipTlsVerify: true }), { onSubmit });

    await act(async () => saveButton().click());
    // Stated, never omitted: the form owns this answer, and an absent value
    // would mean "keep whatever is stored" on the host.
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({ skipTlsVerify: true }));
  });

  it("sends an explicit false when the switch is turned back off", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    mount(provider({ skipTlsVerify: true }), { onSubmit });

    act(() => switchControl().click());
    expect(switchControl().getAttribute("aria-checked")).toBe("false");

    await act(async () => saveButton().click());
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({ skipTlsVerify: false }));
  });

  it("detects thinking levels over the same TLS decision the save will use", async () => {
    // This is the ordering that matters in practice: detection runs before Save,
    // and on a self-signed endpoint a verified probe fails the handshake and is
    // reported as a bad key — so the switch has to reach the probe too.
    const probes: ThinkingLevelProbeInput[] = [];
    const onProbe = vi.fn(async (input: ThinkingLevelProbeInput) => {
      probes.push(input);
      return { level: input.level ?? "default", ok: true } as ThinkingLevelProbeOutcome;
    });
    mount(provider({ skipTlsVerify: true, models: [] }), { onProbe });

    act(() => [...document.querySelectorAll("button")].find(button => button.textContent === "+ Add model")!.click());
    const modelId = document.querySelector<HTMLInputElement>("input[placeholder=\"e.g. qwen\"]")!;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(modelId, "qwen");
      modelId.dispatchEvent(new Event("input", { bubbles: true }));
    });

    await act(async () => detectButton().click());

    expect(probes.length).toBeGreaterThan(0);
    for (const probe of probes)
      expect(probe.skipTlsVerify).toBe(true);
  });

  it("probes with verification on when the switch is off", async () => {
    const probes: ThinkingLevelProbeInput[] = [];
    const onProbe = vi.fn(async (input: ThinkingLevelProbeInput) => {
      probes.push(input);
      return { level: input.level ?? "default", ok: true } as ThinkingLevelProbeOutcome;
    });
    mount(provider({ skipTlsVerify: false }), { onProbe });

    act(() => [...document.querySelectorAll("button")].find(button => button.textContent === "+ Add model")!.click());
    const modelId = document.querySelector<HTMLInputElement>("input[placeholder=\"e.g. qwen\"]")!;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(modelId, "qwen");
      modelId.dispatchEvent(new Event("input", { bubbles: true }));
    });

    await act(async () => detectButton().click());
    expect(probes.length).toBeGreaterThan(0);
    for (const probe of probes)
      expect(probe.skipTlsVerify).toBe(false);
  });

  it("drops a detection result when the TLS decision changes under it", async () => {
    // The result was produced against one TLS setting; flipping the switch makes
    // it describe a request the form no longer represents.
    const onProbe = vi.fn(async (input: ThinkingLevelProbeInput) =>
      ({ level: input.level ?? "default", ok: true }) as ThinkingLevelProbeOutcome);
    mount(provider({ skipTlsVerify: false }), { onProbe });

    act(() => [...document.querySelectorAll("button")].find(button => button.textContent === "+ Add model")!.click());
    const modelId = document.querySelector<HTMLInputElement>("input[placeholder=\"e.g. qwen\"]")!;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(modelId, "qwen");
      modelId.dispatchEvent(new Event("input", { bubbles: true }));
    });

    await act(async () => detectButton().click());
    expect(document.body.textContent).toContain("accepted level");

    act(() => switchControl().click());
    expect(document.body.textContent).not.toContain("accepted level");
  });
});
