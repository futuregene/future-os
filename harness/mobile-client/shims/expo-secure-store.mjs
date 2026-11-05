/**
 * The key store, in memory.
 *
 * The real one is the device keychain; a harness process only needs "the same
 * value comes back within this run", which is what the pairing code relies on
 * when it reads back the device id it just wrote.
 */
const store = new Map();
export async function getItemAsync(key) {
  return store.has(key) ? store.get(key) : null;
}
export async function setItemAsync(key, value) {
  store.set(key, value);
}
export async function deleteItemAsync(key) {
  store.delete(key);
}
export function isAvailableAsync() {
  return Promise.resolve(true);
}
