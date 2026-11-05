/**
 * Test-data preparation: a consistent copy of the real desktop database.
 *
 * `app.db` is 100 MB of real conversations/workspaces because that is the point
 * — a list with 700 workspaces scrolls, virtualises and pages differently from
 * a list with four. The copy is made with SQLite's own backup API so the WAL is
 * folded in and the file is a single consistent snapshot; every case then gets
 * its own copy of that snapshot.
 */
import { copyFileSync, cpSync, existsSync, mkdirSync, rmSync, statSync, writeFileSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { DatabaseSync, backup } from "node:sqlite";

export const DATA_DIR = resolve(import.meta.dirname, "..", ".data");
export const BASE_DB = join(DATA_DIR, "base.db");
export const BASE_META = join(DATA_DIR, "base.json");
export const MODELS_COPY = join(DATA_DIR, "models.json");
export const CLI_SNAPSHOT = join(DATA_DIR, "cli");

export function defaultSource() {
  return join(homedir(), ".future", "app", "app.db");
}

export async function prepare({ source = defaultSource(), force = false } = {}) {
  mkdirSync(DATA_DIR, { recursive: true });
  if (existsSync(BASE_DB) && !force && existsSync(BASE_META))
    return { path: BASE_DB, meta: JSON.parse(readFileSync(BASE_META, "utf8")), reused: true };

  if (!existsSync(source))
    throw new Error(`source database not found: ${source}`);

  const temp = `${BASE_DB}.tmp`;
  const db = new DatabaseSync(source, { readOnly: true });
  try {
    await backup(db, temp);
  }
  finally {
    db.close();
  }
  copyFileSync(temp, BASE_DB);
  try {
    execFileSync("rm", ["-f", temp]);
  }
  catch {}

  const copy = new DatabaseSync(BASE_DB, { readOnly: true });
  const scalar = sql => copy.prepare(sql).get();
  const meta = {
    source,
    preparedAt: new Date().toISOString(),
    bytes: statSync(BASE_DB).size,
    threads: scalar("SELECT COUNT(*) AS n FROM threads WHERE deleted_at IS NULL AND status != 'deleted'").n,
    workspaces: scalar("SELECT COUNT(*) AS n FROM workspaces WHERE deleted_at IS NULL").n,
    runs: scalar("SELECT COUNT(*) AS n FROM runs").n,
    archivedThreads: scalar("SELECT COUNT(*) AS n FROM threads WHERE archived_at IS NOT NULL AND deleted_at IS NULL").n,
    chatThreads: scalar("SELECT COUNT(*) AS n FROM threads WHERE mode = 'chat' AND deleted_at IS NULL").n,
    newestThread: scalar("SELECT title FROM threads WHERE deleted_at IS NULL ORDER BY COALESCE(last_message_at, updated_at) DESC LIMIT 1")?.title ?? null,
  };
  copy.close();

  const modelsSource = join(homedir(), ".future", "agent", "models.json");
  if (existsSync(modelsSource))
    copyFileSync(modelsSource, MODELS_COPY);

  // The CLI surface needs the agent's settings/catalogue (read-only) and, when
  // present, the scheduled-task database. Snapshotted here so a CLI run is not
  // affected by the user editing those files mid-run.
  mkdirSync(CLI_SNAPSHOT, { recursive: true });
  for (const name of ["settings.json", "models.json", "skills-catalogue.json"]) {
    const from = join(homedir(), ".future", "agent", name);
    if (existsSync(from))
      copyFileSync(from, join(CLI_SNAPSHOT, name));
  }
  const skillsDir = join(homedir(), ".future", "agent", "skills");
  if (existsSync(skillsDir))
    cpSync(skillsDir, join(CLI_SNAPSHOT, "skills"), { recursive: true });
  const tasksDb = join(homedir(), ".future", "tasks", "tasks.db");
  if (existsSync(tasksDb))
    copyFileSync(tasksDb, join(CLI_SNAPSHOT, "tasks.db"));

  writeFileSync(BASE_META, JSON.stringify(meta, null, 2));
  return { path: BASE_DB, meta, reused: false };
}

/**
 * An isolated FutureOS home for one CLI case; the real app.db copy plus config.
 *
 * An existing home is removed first. Re-running a case under the same run name
 * reuses this directory, and the skills snapshot holds *symlinks* into the
 * checkout: copying them a second time made `fs.cpSync` resolve the destination
 * through the leftover link and refuse with "cannot copy … to a subdirectory of
 * itself", so every second run of every CLI case died before its first command.
 */
export function createCliHome(target) {
  const home = resolve(target);
  rmSync(home, { recursive: true, force: true });
  mkdirSync(join(home, "app"), { recursive: true });
  mkdirSync(join(home, "agent"), { recursive: true });
  mkdirSync(join(home, "tasks"), { recursive: true });
  mkdirSync(join(home, "run"), { recursive: true });
  copyDatabase(join(home, "app", "app.db"));
  for (const name of ["settings.json", "models.json", "skills-catalogue.json"]) {
    const from = join(CLI_SNAPSHOT, name);
    if (existsSync(from))
      copyFileSync(from, join(home, "agent", name));
  }
  if (existsSync(join(CLI_SNAPSHOT, "skills")))
    cpSync(join(CLI_SNAPSHOT, "skills"), join(home, "agent", "skills"), { recursive: true });
  if (existsSync(join(CLI_SNAPSHOT, "tasks.db")))
    copyFileSync(join(CLI_SNAPSHOT, "tasks.db"), join(home, "tasks", "tasks.db"));
  return { home, appDb: join(home, "app", "app.db") };
}

/** A private copy of the snapshot for one case, so writes cannot leak. */
export function copyDatabase(target) {
  mkdirSync(dirname(target), { recursive: true });
  if (process.platform === "darwin") {
    // APFS clone: instantaneous, copy-on-write, still an independent file.
    try {
      execFileSync("cp", ["-c", BASE_DB, target]);
      return target;
    }
    catch {}
  }
  copyFileSync(BASE_DB, target);
  return target;
}
