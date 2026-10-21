import { spawn } from 'node:child_process';
import { appendFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { once } from 'node:events';
import { performance } from 'node:perf_hooks';

// One record per command and native Rust target summary, never per event/test.
// Keep the original output and exit status. No test flags or scheduling changes.
export async function timedCommand(command, args, options = {}) {
  const directory = join(process.env.RUNNER_TEMP, 'ci-timings');
  let recording = true;
  try { mkdirSync(directory, { recursive: true }); } catch (error) {
    recording = false;
    console.log(`::warning::Command timing directory unavailable: ${error.message}`);
  }
  const record = row => {
    if (!recording) return;
    try {
      appendFileSync(join(directory, 'commands.jsonl'), `${JSON.stringify({
        schema: 1, platform: process.env.RUNNER_OS,
        phase: process.argv[2], packages: process.env.CI_PACKAGES, host_suite: process.env.CI_HOST_SUITE,
        directory: process.env.CI_DIRECTORY, ...row,
      })}\n`);
    } catch (error) {
      recording = false;
      console.log(`::warning::Command timing could not be recorded: ${error.message}`);
    }
  };
  const started = performance.now();
  const child = spawn(command, args, { ...options, stdio: ['inherit', 'pipe', 'pipe'] });
  const forward = signal => child.kill(signal);
  const interrupt = () => forward('SIGINT');
  const terminate = () => forward('SIGTERM');
  process.on('SIGINT', interrupt);
  process.on('SIGTERM', terminate);
  for (const [stream, destination, name] of [[child.stdout, process.stdout, 'stdout'], [child.stderr, process.stderr, 'stderr']]) {
    stream.pipe(destination, { end: false });
    const lines = createInterface({ input: stream, crlfDelay: Infinity });
    lines.on('line', line => {
      // Preserve the framework's reported elapsed seconds. Output arrival time
      // is not a per-test duration, especially with parallel tests/child output.
      if (/^(?:test result: |\s*(?:Running |Doc-tests |Finished `))/.test(line)) {
        record({ kind: 'native_summary', stream: name, line });
      }
    });
  }
  let spawnError;
  child.on('error', error => { spawnError = error; });
  const result = await new Promise(resolve => child.on('close', (status, signal) => resolve({ status, signal })));
  process.off('SIGINT', interrupt);
  process.off('SIGTERM', terminate);
  record({ kind: 'command', command, args, wall_seconds: (performance.now() - started) / 1000,
    ...result, error: spawnError?.message });
  for (const stream of [process.stdout, process.stderr]) {
    if (stream.writableNeedDrain) await once(stream, 'drain');
  }
  if (spawnError) throw spawnError;
  return result;
}
