import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { QUEUE_ROWS_SHOWN_BY_DEFAULT, SendQueuePanel, sendQueueListHeight } from "../SendQueuePanel";

jest.mock("lucide-react-native", () => ({
  Check: () => null,
  ChevronDown: () => null,
  ChevronUp: () => null,
  Image: () => null,
  Paperclip: () => null,
  Pencil: () => null,
  Play: () => null,
  Plus: () => null,
  Trash2: () => null,
  X: () => null,
}));
// The panel is read in one language in these tests: the point is which strings
// are painted when the list is open and when it is closed, not how they translate.
jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, values?: Record<string, unknown>) => {
      const words: Record<string, string> = {
        "chat.queue.title": "Send queue",
        "chat.queue.count": "{{count}} waiting",
        "chat.queue.hint": "Sent as one message when this reply ends",
        "chat.queue.hintParked": "Waited too long",
        "chat.queue.expand": "Show the queued messages",
        "chat.queue.collapse": "Hide the queued messages",
      };
      return (words[key] ?? key).replace(/\{\{(\w+)\}\}/g, (_, name: string) => String(values?.[name] ?? ""));
    },
  }),
}));
jest.mock("../../../../theme/ThemeContext", () => {
  const colors = {
    surface: "#fff",
    surfaceSubtle: "#f1f4f8",
    line: "#d9dee7",
    lineSoft: "#e8edf4",
    ink: "#172033",
    inkMuted: "#606c82",
    accent: "#2563eb",
    warning: "#b45309",
  };
  const theme = { colors, font: (size: number) => size };
  return {
    useTheme: () => theme,
    createThemedStyles: (factory: (t: unknown) => unknown) => Object.assign(
      () => factory(theme),
      { forTheme: (value: unknown) => factory(value) },
    ),
  };
});

let tree: ReactTestRenderer | undefined;
afterEach(() => {
  if (tree)
    act(() => tree!.unmount());
  tree = undefined;
});

function items(count: number, parked = false) {
  return Array.from({ length: count }, (_, index) => ({
    id: `run_${index + 1}`,
    content: `message ${index + 1}`,
    ...(parked ? { parked: true } : {}),
  }));
}

function render(props: Partial<Parameters<typeof SendQueuePanel>[0]> = {}) {
  const element = createElement(SendQueuePanel, {
    items: items(1),
    onRemove: jest.fn(),
    onUpdate: jest.fn(),
    ...props,
  });
  act(() => {
    tree = create(element);
  });
  return element;
}

/** Re-render the same mounted panel with new props (a queue that changed). */
function update(element: ReturnType<typeof createElement>) {
  act(() => tree!.update(element));
}

function rows() {
  return tree!.root.findAll(node => typeof node.type === "string" && node.props.testID === "send-queue-item");
}

/** The header is the expand/collapse control. */
function toggle() {
  return tree!.root.findAll(node =>
    typeof node.props.onPress === "function" && node.props.testID === "send-queue-toggle")[0]!;
}

function list() {
  const node = tree!.root.findAll(inner => inner.props.testID === "send-queue-list")[0];
  if (!node)
    throw new Error("the send queue list is not drawn");
  return node;
}

/** Whether the list is drawn at all — the closed panel draws no list. */
function hasList() {
  return tree!.root.findAll(inner => inner.props.testID === "send-queue-list").length > 0;
}

/** Every string the host text nodes paint, in render order. */
function painted() {
  return tree!.root
    .findAll(node => typeof node.type === "string" && node.props.children !== undefined)
    .flatMap(node => node.props.children)
    .filter((part: unknown) => typeof part === "string")
    .join(" ");
}

describe("the send queue above the composer", () => {
  it("opens a short queue without being asked, and can still be closed", () => {
    render({ items: items(QUEUE_ROWS_SHOWN_BY_DEFAULT) });
    expect(rows()).toHaveLength(QUEUE_ROWS_SHOWN_BY_DEFAULT);
    expect(toggle().props.accessibilityLabel).toBe("Hide the queued messages");
    expect(toggle().props.accessibilityState).toEqual({ expanded: true });

    act(() => toggle().props.onPress());
    // Closed means the rows are gone, not dimmed: the height they took is the
    // height the conversation gets back.
    expect(rows()).toHaveLength(0);
    expect(hasList()).toBe(false);
    expect(painted()).toContain("Send queue");
    expect(painted()).toContain(`${QUEUE_ROWS_SHOWN_BY_DEFAULT} waiting`);
  });

  it("starts a long queue closed, so it cannot cover the reply it waits behind", () => {
    render({ items: items(8) });
    expect(rows()).toHaveLength(0);
    expect(hasList()).toBe(false);
    // What is waiting is still readable at a glance, in the one header line.
    expect(painted()).toContain("8 waiting");
    expect(painted()).toContain("Sent as one message when this reply ends");
    expect(toggle().props.accessibilityLabel).toBe("Show the queued messages");
    expect(toggle().props.accessibilityState).toEqual({ expanded: false });
  });

  it("expands to the whole queue on a tap, and closes again on the next one", () => {
    render({ items: items(8) });
    act(() => toggle().props.onPress());
    expect(rows()).toHaveLength(8);
    expect(hasList()).toBe(true);
    expect(toggle().props.accessibilityLabel).toBe("Hide the queued messages");

    act(() => toggle().props.onPress());
    expect(rows()).toHaveLength(0);
    expect(toggle().props.accessibilityLabel).toBe("Show the queued messages");
  });

  it("keeps a parked row's warning in the header while the rows are closed", () => {
    // A parked row only runs when someone presses its own button, so closing
    // the list must not hide the fact that the queue is waiting on a person.
    render({ items: items(4, true) });
    expect(rows()).toHaveLength(0);
    expect(painted()).toContain("Waited too long");
  });

  it("draws an open list inside the height the composer measured", () => {
    // A queue of ten expanded by hand must not take the screen either: past this
    // height the rows scroll inside the panel.
    render({ items: items(10), maxListHeight: 150 });
    act(() => toggle().props.onPress());
    expect(rows()).toHaveLength(10);
    expect(list().props.style).toContainEqual({ maxHeight: 150 });
  });

  it("follows the queue until someone chooses, then keeps their choice", () => {
    const element = render({ items: items(5) });
    expect(rows()).toHaveLength(0);
    act(() => toggle().props.onPress());
    expect(rows()).toHaveLength(5);
    // The queue drains to one row: the reader opened it, so it stays open.
    update(createElement(SendQueuePanel, {
      ...(element.props as Parameters<typeof SendQueuePanel>[0]),
      items: items(1),
    }));
    expect(rows()).toHaveLength(1);
  });
});

describe("how much room an open queue may take", () => {
  it("keeps the list to a third of the room the keyboard leaves", () => {
    // 300 of 1000pt is a third of the screen for the queue, the rest for the reply.
    expect(sendQueueListHeight(1000, 0)).toBe(240);
    expect(sendQueueListHeight(600, 0)).toBe(180);
  });

  it("never shrinks below one readable row, and never outgrows its cap", () => {
    // With the keyboard up there is little room left: the floor keeps a row
    // visible rather than a sliver.
    expect(sendQueueListHeight(600, 500)).toBe(96);
    expect(sendQueueListHeight(2000, 0)).toBe(240);
  });
});
