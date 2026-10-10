import { Platform, ToastAndroid } from "react-native";
import { act } from "react-test-renderer";
import { confirmDownload, formatBytes, formatCostCny, plainText, showToast } from "../utils";

jest.mock("../../../components/appAlerts", () => ({ AppAlert: { alert: jest.fn() } }));

const alert = (jest.requireMock("../../../components/appAlerts") as {
  AppAlert: { alert: jest.Mock };
}).AppAlert.alert;

afterEach(() => {
  jest.useRealTimers();
  jest.clearAllMocks();
});

describe("a price reads the same in both UI languages", () => {
  test.each([
    [0, "¥0"],
    [-1, "¥0"],
    [Number.NaN, "¥0"],
    [Number.POSITIVE_INFINITY, "¥0"],
    // Below a ten-thousandth of a yuan the honest answer is "less than", not "0".
    [0.00004, "¥<0.0001"],
    [0.0001, "¥0.0001"],
    [0.5, "¥0.5"],
    [1, "¥1"],
    [12.5, "¥12.5"],
    [0.105, "¥0.105"],
    [1_234.5, "¥1,234.5"],
    [1_234_567.8912, "¥1,234,567.8912"],
  ])("%s renders as %s", (value, expected) => {
    expect(formatCostCny(value)).toBe(expected);
  });

  test("trailing zeros are dropped instead of padding every price to four decimals", () => {
    expect(formatCostCny(2.5)).toBe("¥2.5");
    expect(formatCostCny(2.05)).toBe("¥2.05");
    expect(formatCostCny(2.005)).toBe("¥2.005");
    expect(formatCostCny(2.0005)).toBe("¥2.0005");
  });
});

describe("file sizes", () => {
  test.each([
    [0, "1 KB"],
    [1, "1 KB"],
    [1023, "1 KB"],
    [1024, "1 KB"],
    [1025, "2 KB"],
    [1024 * 1024 - 1, "1024 KB"],
    [1024 * 1024, "1.0 MB"],
    [1024 * 1024 * 1.5, "1.5 MB"],
    [1024 * 1024 * 1024, "1024.0 MB"],
  ])("%s bytes reads as %s", (bytes, expected) => {
    expect(formatBytes(bytes)).toBe(expected);
  });
});

describe("a text preview refuses binary content", () => {
  const encode = (text: string) => new TextEncoder().encode(text);

  test("plain ASCII, CJK and emoji survive", () => {
    expect(plainText(encode("hello"))).toBe("hello");
    expect(plainText(encode("中文标题"))).toBe("中文标题");
    expect(plainText(encode("emoji 🙂 pair 👨‍👩‍👧"))).toBe("emoji 🙂 pair 👨‍👩‍👧");
    expect(plainText(new Uint8Array())).toBe("");
  });

  test("layout whitespace is allowed, other control bytes are not", () => {
    expect(plainText(encode("a\tb\nc\rd"))).toBe("a\tb\nc\rd");
    expect(plainText(new Uint8Array([0x61, 0x00, 0x62]))).toBeNull();
    expect(plainText(new Uint8Array([0x61, 0x1b, 0x62]))).toBeNull();
    expect(plainText(new Uint8Array([0x7f]))).not.toBeNull();
  });

  test.each([
    ["a lone continuation byte", [0x80]],
    ["an invalid lead byte", [0xf5]],
    ["an overlong two-byte form", [0xc0, 0x80]],
    ["an overlong three-byte form", [0xe0, 0x80, 0x80]],
    ["a surrogate code point", [0xed, 0xa0, 0x80]],
    ["a code point past U+10FFFF", [0xf4, 0x90, 0x80, 0x80]],
    ["a bad continuation", [0xe4, 0x41]],
  ])("%s is rejected rather than replaced", (_case, bytes) => {
    expect(plainText(new Uint8Array(bytes))).toBeNull();
  });

  test("a preview cut in the middle of a code point drops the partial character", () => {
    const cut = encode("简历.docx").slice(0, 4); // three bytes of 简 plus one of 历
    expect(plainText(cut, true)).toBe("简");
    // The same bytes without the truncation flag are not decodable at all.
    expect(plainText(cut, false)).toBeNull();
  });

  test("a trailing partial code point at the very end of a truncated preview is dropped", () => {
    const cut = new Uint8Array([...encode("ok"), 0xe4]);
    expect(plainText(cut, true)).toBe("ok");
    expect(plainText(cut)).toBeNull();
  });
});

describe("a transient toast", () => {
  test("Android uses the platform toast, which needs no dismissal", () => {
    Platform.OS = "android";
    const show = jest.spyOn(ToastAndroid, "show").mockImplementation(() => {});
    showToast("attachment failed");
    expect(show).toHaveBeenCalledWith("attachment failed", ToastAndroid.SHORT);
    expect(alert).not.toHaveBeenCalled();
  });

  test("iOS has no native toast, so it falls back to the app dialog", () => {
    Platform.OS = "ios";
    showToast("attachment failed");
    expect(alert).toHaveBeenCalledWith("attachment failed");
  });
});

describe("the cellular download confirmation", () => {
  /** The alert the caller was shown, with its buttons and dismissal options. */
  function shown() {
    expect(alert).toHaveBeenCalledTimes(1);
    const [title, message, buttons, options] = alert.mock.calls[0]!;
    return {
      buttons: buttons as { text: string; onPress?: () => void }[],
      message: message as string,
      options: options as { cancelable?: boolean; onDismiss?: () => void },
      title: title as string,
    };
  }
  function press(text: string) {
    const button = shown().buttons.find(candidate => candidate.text === text);
    expect(button).toBeDefined();
    act(() => button!.onPress?.());
  }

  test("downloading is confirmed only after the dialog has dismissed", async () => {
    jest.useFakeTimers();
    Platform.OS = "ios";
    const answer = confirmDownload("Download file?", "12 MB", "Cancel", "Download");
    press("Download");
    const settled: boolean[] = [];
    void answer.then(value => settled.push(value));
    await act(async () => { await Promise.resolve(); });
    // The promise must not resolve while UIKit still owns the screen.
    expect(settled).toEqual([]);
    await act(async () => { jest.advanceTimersByTime(350); await Promise.resolve(); });
    expect(settled).toEqual([true]);
    expect(shown().options.cancelable).toBe(true);
    expect(shown().message).toBe("12 MB");
  });

  test("cancelling answers false", async () => {
    jest.useFakeTimers();
    Platform.OS = "android";
    const answer = confirmDownload("Download file?", "12 MB", "Cancel", "Download");
    press("Cancel");
    await act(async () => { jest.advanceTimersByTime(0); await Promise.resolve(); });
    await expect(answer).resolves.toBe(false);
  });

  test("dismissing without choosing answers false, not undecided", async () => {
    jest.useFakeTimers();
    const answer = confirmDownload("Download file?", "12 MB", "Cancel", "Download");
    act(() => shown().options.onDismiss?.());
    await act(async () => { jest.advanceTimersByTime(0); await Promise.resolve(); });
    await expect(answer).resolves.toBe(false);
  });

  test("a dismissal that follows a choice cannot flip the answer", async () => {
    jest.useFakeTimers();
    let resolutions = 0;
    const answer = confirmDownload("Download file?", "12 MB", "Cancel", "Download");
    void answer.then(() => { resolutions += 1; });
    press("Download");
    act(() => shown().options.onDismiss?.());
    await act(async () => { jest.advanceTimersByTime(400); await Promise.resolve(); });
    await expect(answer).resolves.toBe(true);
    expect(resolutions).toBe(1);
  });

  test("two taps on the same button resolve once", async () => {
    jest.useFakeTimers();
    let resolutions = 0;
    const answer = confirmDownload("Download file?", "12 MB", "Cancel", "Download");
    void answer.then(() => { resolutions += 1; });
    press("Download");
    press("Download");
    await act(async () => { jest.advanceTimersByTime(400); await Promise.resolve(); });
    await expect(answer).resolves.toBe(true);
    expect(resolutions).toBe(1);
  });
});
