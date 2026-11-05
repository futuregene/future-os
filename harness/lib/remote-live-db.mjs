/**
 * Read the host's own `app.db` from a harness process.
 *
 * A phone-side write is only proven once the desktop's database says so: the
 * client reporting "ok" means the host accepted the command, not that it landed.
 * The host keeps its database under its `FUTURE_HOME`, so this reads that file
 * directly — the same snapshot the desktop would open.
 */
import { DatabaseSync } from "node:sqlite";
import { existsSync } from "node:fs";
import { join } from "node:path";

export function queryHostDb({ home, sql }) {
  // The host creates `app/app.db`; older layouts put it at the home root.
  const candidates = [join(home, "app", "app.db"), join(home, "app.db")];
  const path = candidates.find(item => existsSync(item));
  if (!path)
    throw new Error(`no app.db under ${home} (looked in ${candidates.join(", ")})`);
  const db = new DatabaseSync(path, { readOnly: true });
  try {
    const rows = db.prepare(sql).all();
    if (rows.length === 1 && Object.keys(rows[0]).length === 1)
      return Object.values(rows[0])[0];
    return rows;
  }
  finally {
    db.close();
  }
}
