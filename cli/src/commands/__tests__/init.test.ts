import { afterEach, describe, expect, test } from "bun:test";
import {
  mkdtemp,
  mkdir,
  readFile,
  readlink,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { init } from "../init.js";

const temporaryDirectories: string[] = [];

async function createUnixFixture(): Promise<{
  executablePath: string;
  homeDir: string;
  root: string;
}> {
  const root = await mkdtemp(join(tmpdir(), "future-init-"));
  temporaryDirectories.push(root);

  const executableDirectory = join(root, "app");
  const homeDir = join(root, "home");
  await mkdir(executableDirectory, { recursive: true });
  await mkdir(homeDir, { recursive: true });

  const executablePath = join(executableDirectory, "future");
  await writeFile(executablePath, "");
  await writeFile(join(executableDirectory, "future-agent"), "");

  return { executablePath, homeDir, root };
}

afterEach(async () => {
  await Promise.all(
    temporaryDirectories.splice(0).map(directory =>
      rm(directory, { recursive: true, force: true }),
    ),
  );
});

describe("future init", () => {
  test("installs builtins and creates idempotent macOS command links", async () => {
    const fixture = await createUnixFixture();
    let installCount = 0;
    const installBuiltins = async () => {
      installCount++;
    };

    await init({
      executablePath: fixture.executablePath,
      homeDir: fixture.homeDir,
      installBuiltins,
      platform: "darwin",
    });

    const binDir = join(fixture.homeDir, ".future", "bin");
    const executablePath = await realpath(fixture.executablePath);
    const agentPath = await realpath(join(fixture.root, "app", "future-agent"));
    expect(installCount).toBe(1);
    expect(await readlink(join(binDir, "future"))).toBe(executablePath);
    expect(await readlink(join(binDir, "future-agent"))).toBe(agentPath);

    await init({
      executablePath: join(binDir, "future"),
      homeDir: fixture.homeDir,
      installBuiltins,
      platform: "darwin",
    });

    expect(installCount).toBe(2);
    expect(await readlink(join(binDir, "future"))).toBe(executablePath);
    expect(await readlink(join(binDir, "future-agent"))).toBe(agentPath);
  });

  test("installs builtins and creates command links on Linux", async () => {
    const fixture = await createUnixFixture();
    let installCount = 0;

    await init({
      executablePath: fixture.executablePath,
      homeDir: fixture.homeDir,
      installBuiltins: async () => {
        installCount++;
      },
      platform: "linux",
    });

    expect(installCount).toBe(1);
    const binDir = join(fixture.homeDir, ".future", "bin");
    expect(await readlink(join(binDir, "future"))).toBe(
      await realpath(fixture.executablePath),
    );
    expect(await readlink(join(binDir, "future-agent"))).toBe(
      await realpath(join(fixture.root, "app", "future-agent")),
    );
  });

  test("links future when the sibling future-agent is missing", async () => {
    const fixture = await createUnixFixture();
    await rm(join(fixture.root, "app", "future-agent"));

    await init({
      executablePath: fixture.executablePath,
      homeDir: fixture.homeDir,
      installBuiltins: async () => {},
      platform: "darwin",
    });

    const binDir = join(fixture.homeDir, ".future", "bin");
    expect(await readlink(join(binDir, "future"))).toBe(
      await realpath(fixture.executablePath),
    );
    await expect(readlink(join(binDir, "future-agent"))).rejects.toThrow();
  });

  test("installs builtins without creating command links on Windows", async () => {
    const fixture = await createUnixFixture();
    let installCount = 0;

    await init({
      executablePath: fixture.executablePath,
      homeDir: fixture.homeDir,
      installBuiltins: async () => {
        installCount++;
      },
      platform: "win32",
    });

    expect(installCount).toBe(1);
    await expect(
      readlink(join(fixture.homeDir, ".future", "bin", "future")),
    ).rejects.toThrow();
  });

  test("does not overwrite an existing regular command file", async () => {
    const fixture = await createUnixFixture();
    const binDir = join(fixture.homeDir, ".future", "bin");
    const existingCommand = join(binDir, "future");
    await mkdir(binDir, { recursive: true });
    await writeFile(existingCommand, "keep me");

    await expect(
      init({
        executablePath: fixture.executablePath,
        homeDir: fixture.homeDir,
        installBuiltins: async () => {},
        platform: "darwin",
      }),
    ).rejects.toThrow("already exists and is not a symbolic link");
    expect(await readFile(existingCommand, "utf8")).toBe("keep me");
  });

  test("rejects an interpreter path instead of linking it as future", async () => {
    const fixture = await createUnixFixture();
    const interpreterPath = join(fixture.root, "app", "bun");
    await writeFile(interpreterPath, "");

    await expect(
      init({
        executablePath: interpreterPath,
        homeDir: fixture.homeDir,
        installBuiltins: async () => {},
        platform: "darwin",
      }),
    ).rejects.toThrow("Run the standalone future executable");
  });
});
