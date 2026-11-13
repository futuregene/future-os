import { readableDeviceName } from "../deviceName";

/**
 * A desktop whose Chinese computer name went through a launcher's local code
 * page declares this, and it is what the standby line in a conversation showed
 * a reader: replacement boxes where the machine's name should be.
 */
const MANGLED = "FutureOS Desktop (\uFFFD\uFFFD\uFFFD\uFFFD\uFFFD\uFFFD\u013C\uFFFD)";

describe("a device name worth showing", () => {
  test("a readable name is kept, trimmed", () => {
    expect(readableDeviceName("  Studio iMac  ")).toBe("Studio iMac");
    // Not about non-ASCII: a machine named in Chinese is a name.
    expect(readableDeviceName("书房电视")).toBe("书房电视");
    expect(readableDeviceName("FutureOS Desktop (geilige的Mac Studio)"))
      .toBe("FutureOS Desktop (geilige的Mac Studio)");
  });

  test("nothing to show is not a name", () => {
    expect(readableDeviceName("")).toBeUndefined();
    expect(readableDeviceName("   ")).toBeUndefined();
    expect(readableDeviceName(null)).toBeUndefined();
    expect(readableDeviceName(undefined)).toBeUndefined();
  });

  test("a name the wire mangled is not a name", () => {
    expect(readableDeviceName(MANGLED)).toBeUndefined();
    // One lost byte anywhere is enough: the rest of the label cannot be trusted
    // to say which machine it is either.
    expect(readableDeviceName("FutureOS Desktop (a\uFFFDb)")).toBeUndefined();
    // Nor is a control character a piece of a label a person typed.
    expect(readableDeviceName("Studio\u0000iMac")).toBeUndefined();
    expect(readableDeviceName("Studio\u007FiMac")).toBeUndefined();
  });
});
