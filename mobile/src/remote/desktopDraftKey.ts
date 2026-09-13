/** Session ids and the new-conversation placeholder are local to a desktop. */
export function desktopDraftKey(desktopId: string, sessionId = ""): string {
  return `${desktopId}:${sessionId || "draft:new"}`;
}
