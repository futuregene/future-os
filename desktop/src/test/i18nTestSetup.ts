import i18n from "../i18n";

// Tests assert against the canonical English wording, so pin the test locale to
// English regardless of the app's default (Chinese) language. Resources are
// bundled inline, so changeLanguage applies synchronously.
void i18n.changeLanguage("en");

// Vitest's jsdom environment does not expose Web Storage (window.localStorage
// is undefined there), but app code reads/writes it defensively. Provide a
// minimal in-memory Storage so those code paths run under jsdom tests.
if (typeof globalThis.localStorage === "undefined") {
  const store = new Map<string, string>();
  const storage: Storage = {
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
  Object.defineProperty(globalThis, "localStorage", { value: storage, configurable: true });
}
