import i18n from "../i18n";

// Tests assert against the canonical English wording, so pin the test locale to
// English regardless of the app's default (Chinese) language. Resources are
// bundled inline, so changeLanguage applies synchronously.
void i18n.changeLanguage("en");

// A minimal in-memory Storage for a global the runtime did not provide.
function createMemoryStorage(): Storage {
  const store = new Map<string, string>();
  return {
    get length() {
      return store.size;
    },
    clear: () => store.clear(),
    getItem: key => (store.has(key) ? store.get(key)! : null),
    key: (index) => {
      const keys = [...store.keys()];
      const found = index >= 0 && index < keys.length ? keys[index] : undefined;
      return found ?? null;
    },
    removeItem: key => void store.delete(key),
    setItem: (key, value) => void store.set(key, String(value)),
  };
}

function installMemoryStorageIfMissing(name: "localStorage" | "sessionStorage"): void {
  if (typeof globalThis[name] !== "undefined")
    return;
  Object.defineProperty(globalThis, name, { value: createMemoryStorage(), configurable: true });
}

// Web Storage is not guaranteed by the test environment: most of this suite
// runs in Vitest's default `node` environment (only tests that need a DOM opt
// into jsdom), and Node defines neither storage globally on 24 — while 26 ships
// an experimental `sessionStorage` only. So a storage-backed module's tests
// used to pass or fail depending on the Node version the suite ran on, and app
// code that reads/writes storage defensively had its paths skipped there.
// Provide whichever global is missing, so both storages behave the same
// everywhere. jsdom keeps its own real storage: this only fills in an absent
// global.
installMemoryStorageIfMissing("localStorage");
installMemoryStorageIfMissing("sessionStorage");
