export function isRecord(value) {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}
export function getRecord(value) {
    return isRecord(value) ? value : undefined;
}
export function ensureRecordProperty(parent, key) {
    const current = getRecord(parent[key]);
    if (current) {
        return current;
    }
    const next = {};
    parent[key] = next;
    return next;
}
export function isNodeError(error) {
    return error instanceof Error && "code" in error;
}
