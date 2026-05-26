import { spawn } from "node:child_process";
export async function runProcess(command, args, options = {}) {
    return new Promise((resolve, reject) => {
        const child = spawn(command, args, {
            ...options,
            stdio: ["ignore", "pipe", "pipe"],
        });
        let stdout = "";
        let stderr = "";
        child.stdout.setEncoding("utf8");
        child.stdout.on("data", (chunk) => {
            stdout += chunk;
        });
        child.stderr.setEncoding("utf8");
        child.stderr.on("data", (chunk) => {
            stderr += chunk;
        });
        child.once("error", reject);
        child.once("close", (code) => {
            resolve({
                code: code ?? 1,
                stdout,
                stderr,
            });
        });
    });
}
export async function runInheritedProcess(command, args) {
    return new Promise((resolve, reject) => {
        const child = spawn(command, args, {
            stdio: "inherit",
        });
        child.once("error", reject);
        child.once("close", (code) => {
            resolve({
                code: code ?? 1,
                stdout: "",
                stderr: "",
            });
        });
    });
}
export function formatProcessOutput(result) {
    return [result.stdout.trim(), result.stderr.trim()].filter(Boolean).join("\n");
}
