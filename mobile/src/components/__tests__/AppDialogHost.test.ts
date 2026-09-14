import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Alert, Modal, Platform, Text } from "react-native";
import { AppDialogHost } from "../AppDialogHost";
import { AppAlert, currentAppAlert, finishAppAlert } from "../appAlerts";
import { Button } from "../Button";
import { DialogSurface } from "../DialogSurface";

jest.mock("react-native-safe-area-context", () => ({ useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }) }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

let tree: ReactTestRenderer;
const platform = Platform.OS;
beforeEach(() => {
  jest.useFakeTimers();
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
  act(() => { tree = create(createElement(AppDialogHost)); });
});
afterEach(() => {
  act(() => tree.unmount());
  let request = currentAppAlert();
  while (request) { finishAppAlert(request); request = currentAppAlert(); }
  Platform.OS = platform;
  jest.restoreAllMocks();
  jest.useRealTimers();
});
function dismiss() {
  act(() => {
    if (Platform.OS === "ios") tree.root.findByType(Modal).props.onDismiss();
    else jest.runOnlyPendingTimers();
  });
}

test.each(["ios", "android"] as const)("%s shows a selectable app-styled error, never a native alert", os => {
  Platform.OS = os;
  act(() => AppAlert.alert("Cannot share", "Permission denied\n".repeat(100)));
  expect(tree.root.findByType(Modal).props.visible).toBe(true);
  expect(tree.root.findAllByType(DialogSurface)).toHaveLength(1);
  expect(tree.root.findAllByType(Text).some(node => node.props.selectable)).toBe(true);
  expect(Alert.alert).not.toHaveBeenCalled();
  act(() => tree.root.findByType(Button).props.onPress());
  expect(currentAppAlert()).not.toBeNull();
  dismiss();
  expect(currentAppAlert()).toBeNull();
});

test.each(["ios", "android"] as const)("%s queues errors without replacing a pending confirmation", os => {
  Platform.OS = os;
  const confirm = jest.fn();
  act(() => {
    AppAlert.alert("Confirm", "Download?", [{ text: "Download", onPress: confirm }]);
    AppAlert.alert("Error", "Another operation failed");
  });
  act(() => tree.root.findByType(Button).props.onPress());
  expect(confirm).not.toHaveBeenCalled();
  expect(currentAppAlert()?.title).toBe("Confirm");
  dismiss();
  expect(confirm).toHaveBeenCalledTimes(1);
  expect(currentAppAlert()?.title).toBe("Error");
  expect(tree.root.findByType(Modal).props.visible).toBe(true);
  act(() => tree.root.findByType(Button).props.onPress());
  dismiss();
  expect(currentAppAlert()).toBeNull();
});

test("system back settles cancellation once, after dismissal", () => {
  Platform.OS = "android";
  const cancelled = jest.fn();
  const accepted = jest.fn();
  act(() => AppAlert.alert("Download", "Cellular", [{ text: "Yes", onPress: accepted }], { cancelable: true, onDismiss: cancelled }));
  act(() => { tree.root.findByType(Modal).props.onRequestClose(); tree.root.findByType(Modal).props.onRequestClose(); });
  expect(cancelled).not.toHaveBeenCalled();
  dismiss();
  expect(cancelled).toHaveBeenCalledTimes(1);
  expect(accepted).not.toHaveBeenCalled();
  expect(currentAppAlert()).toBeNull();
});

test("noncancelable dialogs ignore system back", () => {
  const cancelled = jest.fn();
  act(() => AppAlert.alert("Required", undefined, undefined, { cancelable: false, onDismiss: cancelled }));
  act(() => tree.root.findByType(Modal).props.onRequestClose());
  expect(tree.root.findByType(Modal).props.visible).toBe(true);
  expect(cancelled).not.toHaveBeenCalled();
});

test("an action failure can enqueue a new error after its confirmation closes", () => {
  Platform.OS = "ios";
  act(() => AppAlert.alert("Install", undefined, [{ text: "OK", onPress: () => AppAlert.alert("Install failed") }]));
  act(() => tree.root.findByType(Button).props.onPress());
  dismiss();
  expect(currentAppAlert()?.title).toBe("Install failed");
  expect(tree.root.findByType(Modal).props.visible).toBe(true);
});
