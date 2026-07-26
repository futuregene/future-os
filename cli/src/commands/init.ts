import { lstat, mkdir, readlink, realpath, symlink, unlink } from "node:fs/promises";
import { homedir, platform as osPlatform } from "node:os";
import { basename, dirname, join, resolve } from "node:path";

import { installBuiltinSkills } from "./skills.js";

interface InitOptions {
  executablePath?: string;
  homeDir?: string;
  installBuiltins?: () => Promise<void>;
  platform?: NodeJS.Platform;
}

export async function init(options: InitOptions = {}): Promise<void> {
  const installBuiltins = options.installBuiltins ?? installBuiltinSkills;
  await installBuiltins();

  const platform = options.platform ?? osPlatform();
  if (platform !== "darwin" && platform !== "linux") {
    return;
  }

  const homeDir = options.homeDir ?? homedir();
  const executablePath = await realpath(options.executablePath ?? process.execPath);
  if (basename(executablePath) !== "future") {
    throw new Error(
      `Cannot initialize command links from ${executablePath}. Run the standalone future executable.`,
    );
  }

  const executableDir = dirname(executablePath);
  const expectedAgentPath = join(executableDir, "future-agent");
  let agentPath: string | undefined;
  try {
    agentPath = await realpath(expectedAgentPath);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
      throw error;
    }
  }
  const binDir = join(homeDir, ".future", "bin");

  await mkdir(binDir, { recursive: true });
  await ensureSymlink(executablePath, join(binDir, "future"));
  if (agentPath) {
    await ensureSymlink(agentPath, join(binDir, "future-agent"));
  }

  console.log(
    `Linked future${agentPath ? " and future-agent" : ""} into ${binDir}.`,
  );
  console.log("You can add ~/.future/bin/ to your PATH:");
  console.log('  export PATH="$HOME/.future/bin:$PATH"');
}

async function ensureSymlink(source: string, destination: string): Promise<void> {
  try {
    const destinationStat = await lstat(destination);
    if (!destinationStat.isSymbolicLink()) {
      throw new Error(
        `Cannot create command link: ${destination} already exists and is not a symbolic link.`,
      );
    }

    const currentTarget = await readlink(destination);
    if (resolve(dirname(destination), currentTarget) === source) {
      return;
    }

    await unlink(destination);
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code !== "ENOENT") {
      throw error;
    }
  }

  await symlink(source, destination);
}
