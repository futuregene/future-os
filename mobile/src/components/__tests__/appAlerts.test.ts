import { AppAlert, currentAppAlert, finishAppAlert, subscribeAppAlerts } from "../appAlerts";

/**
 * The queue exists because several non-React callers (updates, notifications,
 * downloads) can raise a dialog at once. A duplicate dismissal event or a late
 * native callback must never drop the dialog a user is actually looking at.
 */
afterEach(() => {
  // Drain whatever the test left queued: the queue is module state.
  for (let guard = 0; guard < 10 && currentAppAlert(); guard += 1) {
    finishAppAlert(currentAppAlert()!);
  }
});

test("concurrent alerts queue instead of replacing each other", () => {
  let emits = 0;
  const unsubscribe = subscribeAppAlerts(() => { emits += 1; });
  AppAlert.alert("first", "one");
  AppAlert.alert("second", "two", [{ text: "ok" }], { cancelable: false });
  expect(currentAppAlert()).toMatchObject({ title: "first", message: "one" });
  expect(emits).toBe(2);
  finishAppAlert(currentAppAlert()!);
  expect(emits).toBe(3);
  expect(currentAppAlert()).toMatchObject({
    title: "second", message: "two", buttons: [{ text: "ok" }], options: { cancelable: false },
  });
  unsubscribe();
  // After unsubscribing, the listener stops hearing about new alerts.
  AppAlert.alert("third");
  expect(emits).toBe(3);
});

test("a dismissal for something other than the head alert is ignored", () => {
  AppAlert.alert("head");
  AppAlert.alert("second");
  // A stale native callback (the modal that already dismissed) must not pop the
  // dialog that is on screen now.
  finishAppAlert({ title: "already gone" });
  expect(currentAppAlert()?.title).toBe("head");
  finishAppAlert(currentAppAlert()!);
  expect(currentAppAlert()?.title).toBe("second");
});

test("an empty queue stays empty and reports nothing to show", () => {
  expect(currentAppAlert()).toBeNull();
  finishAppAlert({ title: "never shown" });
  expect(currentAppAlert()).toBeNull();
});
