import { constants as fsConstants } from "node:fs";
import { access } from "node:fs/promises";
export async function assertReadableFile(path, label, hint) {
    try {
        await access(path);
    }
    catch {
        throw new Error(`${label} not found at ${path}.${hint ? ` ${hint}` : ""}`);
    }
}
export async function assertExecutableFile(path, label) {
    if (!(await canAccess(path, fsConstants.X_OK))) {
        throw new Error(`${label} not found or not executable at ${path}.`);
    }
}
export async function canAccess(path, mode) {
    try {
        await access(path, mode);
        return true;
    }
    catch {
        return false;
    }
}
export { fsConstants };
