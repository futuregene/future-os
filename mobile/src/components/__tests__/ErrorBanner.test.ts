import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ErrorBanner } from "../ErrorBanner";
import i18n from "../../i18n";
import { friendlyError, friendlyRunError } from "../errorMessage";

const t = (key: string, opts?: Record<string, unknown>): string =>
  opts?.code ? `${key}:${String(opts.code)}` : key;

jest.mock("lucide-react-native", () => ({ X: () => null }));

describe("friendlyError", () => {
  test.each([
    ["HTTP 503", "connection.errorServiceLater:SV001"],
    ["nats_connection_exhausted", "connection.errorNetwork:NW001"],
    ["remote_state_subscription_ended", "connection.errorNetwork:NW001"],
    ["invalid_remote_credential", "connection.errorPairing:PA001"],
    ["pairing_confirmation_mismatch", "connection.errorPairing:PA001"],
    ["generation_unhealthy:protocol", "connection.errorServiceSupport:PT001"],
    ["generation_unhealthy:subscription", "connection.errorServiceLater:RT001"],
  ])("maps %s to %s", (message, expected) => {
    expect(friendlyError(message, t)).toBe(expected);
  });

  test("does not expose an unknown internal error", () => {
    expect(friendlyError("sqlite row decode exploded", t)).toBe("connection.errorGeneric:LC999");
  });
});

describe("friendlyRunError", () => {
  test.each([
    ["Authentication failed (401). Check your API key.", "failure.auth"],
    ["API request failed (HTTP 429). Too many requests.", "failure.rateLimited"],
    ["API request failed (HTTP 503).", "failure.serverError"],
    ["[CTX_LIMIT] context too large", "failure.contextLimit"],
    ["Unable to connect to Future Agent", "failure.agentInterrupted"],
    ["error decoding response body", "failure.upstreamDisconnected"],
    ["[MODEL_RESPONSE_ERROR] invalid provider stream", "failure.modelResponseError"],
    ["[AGENT_INTERRUPTED] run ended", "failure.agentInterrupted"],
  ])("maps %s to %s", (message, expected) => {
    expect(friendlyRunError(message, t)).toBe(expected);
  });

  test("falls back to the generic failure for unknown errors", () => {
    expect(friendlyRunError("model exploded", t)).toBe("failure.run");
    expect(friendlyRunError(undefined, t)).toBe("failure.unknown");
  });
});

describe("the rendered banner", () => {
  let tree: ReactTestRenderer;
  afterEach(() => { if (tree) act(() => tree.unmount()); });

  function render(props: Parameters<typeof ErrorBanner>[0]) {
    act(() => { tree = create(createElement(ErrorBanner, props)); });
  }
  function painted() {
    return tree.root
      .findAll(node => typeof node.type === "string" && typeof node.props.children === "string")
      .map(node => node.props.children as string);
  }

  test("a raw backend error is shown as its localized classification, not verbatim", () => {
    render({ message: "HTTP 503" });
    // The shipped copy for the classified error, from the real locale deck.
    expect(painted()).toContain(i18n.t("connection.errorServiceLater", { code: "SV001" }));
    expect(painted()).not.toContain("HTTP 503");
  });

  test("an unknown internal error never reaches the screen", () => {
    render({ message: "sqlite row decode exploded" });
    expect(painted()).toContain(i18n.t("connection.errorGeneric", { code: "LC999" }));
    expect(painted().some(line => line.includes("sqlite"))).toBe(false);
  });

  test("the banner only offers a dismiss control when the caller can dismiss it", () => {
    const onDismiss = jest.fn();
    render({ message: "HTTP 503", onDismiss });
    const close = tree.root.findAll(node =>
      node.props.accessibilityLabel === i18n.t("common.close")
      && typeof node.props.onPress === "function")[0]!;
    expect(close).toBeDefined();
    act(() => close.props.onPress());
    expect(onDismiss).toHaveBeenCalledTimes(1);
    act(() => tree.unmount());
    render({ message: "HTTP 503" });
    expect(tree.root.findAll(node => typeof node.props.onPress === "function")).toHaveLength(0);
  });
});

