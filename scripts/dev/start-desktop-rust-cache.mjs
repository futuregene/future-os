#!/usr/bin/env node
// Repair Cargo's cached failed compiler probes, preserving sccache itself.
import { randomUUID } from 'node:crypto';
import { appendFileSync, readFileSync, realpathSync, renameSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export function repairRustcInfo(targetDirs) {
  const diagnostics = [];
  for (const directory of new Set(targetDirs.map(dir => resolve(dir)))) {
    const file = join(directory, '.rustc_info.json');
    let original;
    try {
      original = readFileSync(file, 'utf8');
    } catch (error) {
      if (error.code === 'ENOENT') continue;
      throw error;
    }
    let data;
    try {
      data = JSON.parse(original);
    } catch {
      diagnostics.push(`Unrecognized compiler metadata left to Cargo: ${file}`);
      continue;
    }
    const outputs = data?.outputs;
    if (!outputs || typeof outputs !== 'object' || Array.isArray(outputs)) continue;
    const failures = Object.values(outputs).filter(output => output?.success === false
      && typeof output.stderr === 'string' && /sccache:\s*error:/i.test(output.stderr));
    if (!failures.length) continue;
    // A parallel Cargo process may already have regenerated this tiny file.
    if (readFileSync(file, 'utf8') !== original) {
      diagnostics.push(`Compiler metadata changed concurrently; leaving it intact: ${file}`);
      continue;
    }
    const backup = `${file}.sccache-failure-${randomUUID()}.bak`;
    renameSync(file, backup);
    diagnostics.push(`Recovered cached sccache probe failure: ${file}`);
    diagnostics.push(`Original metadata preserved at: ${backup}`);
    for (const failure of failures) diagnostics.push(failure.stderr.trim());
    diagnostics.push('Cargo will regenerate compiler metadata; sccache and compiled artifacts remain enabled and intact.');
  }
  return diagnostics;
}

// macOS /var is a symlink to /private/var; Node resolves the module URL.
if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [log, ...targets] = process.argv.slice(2);
    if (!log || targets.length === 0) throw new Error('Expected log path and Cargo target directories');
    const diagnostics = repairRustcInfo(targets);
    if (diagnostics.length) {
      const message = `${diagnostics.join('\n')}\n`;
      appendFileSync(log, message);
      process.stderr.write(message);
    }
  } catch (error) {
    console.error(`Cargo compiler metadata recovery failed: ${error.message}`);
    process.exitCode = 1;
  }
}
