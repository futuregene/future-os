import { build } from "esbuild";
import { writeFile } from "node:fs/promises";
const result = await build({
  entryPoints: ["src-ts/client.ts"], outfile: ".artifacts/client.cjs",
  platform: "node", target: "node24", format: "cjs", bundle: true,
  alias: { "sodium-universal": "sodium-javascript" },
  logLevel: "info", metafile: true,
});
await writeFile(".artifacts/meta.json", JSON.stringify(result.metafile, null, 2));
