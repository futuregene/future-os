import { useEffect, useSyncExternalStore } from "react";
import { currentAppAlert, finishAppAlert, subscribeAppAlerts } from "./appAlerts";
import { useAppDialog } from "./useAppDialog";

/** One persistent host, including during navigation and desktop switches. */
export function AppDialogHost() {
  const request = useSyncExternalStore(subscribeAppAlerts, currentAppAlert);
  const { alert, dialog } = useAppDialog();
  useEffect(() => {
    if (!request) return;
    const finish = (action?: () => void) => () => {
      finishAppAlert(request);
      action?.();
    };
    alert(request.title, request.message, (request.buttons?.length ? request.buttons : [{}]).map(button => ({
      ...button,
      onPress: finish(button.onPress),
    })), {
      ...request.options,
      onDismiss: finish(request.options?.onDismiss ?? request.buttons?.find(button => button.style === "cancel")?.onPress),
    });
  }, [request, alert]);
  return dialog;
}
