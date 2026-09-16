"""Fetch pinned PUBLIC upstream evidence/fixtures, not mutable default branches."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import subprocess

SOURCES = {
    'codex-base.md': ('openai/codex', 'b13164d86f9a70adc48d22f4a5a07ed0c001a1d0', 'codex-rs/models-manager/prompt.md'),
    'codex-compact.md': ('openai/codex', 'b13164d86f9a70adc48d22f4a5a07ed0c001a1d0', 'codex-rs/prompts/templates/compact/prompt.md'),
    'codex-truncate.rs': ('openai/codex', 'b13164d86f9a70adc48d22f4a5a07ed0c001a1d0', 'codex-rs/utils/string/src/truncate.rs'),
    'opencode-system.txt': ('anomalyco/opencode', 'e03db9bc6908f75c9334d8aa997deeaac81c0298', 'packages/opencode/src/agent/prompt/compaction.txt'),
    'opencode-compaction.ts': ('anomalyco/opencode', 'e03db9bc6908f75c9334d8aa997deeaac81c0298', 'packages/opencode/src/session/compaction.ts'),
    'opencode-token.ts': ('anomalyco/opencode', 'e03db9bc6908f75c9334d8aa997deeaac81c0298', 'packages/core/src/util/token.ts'),
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--output', type=Path, required=True)
    args = ap.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    manifest = {}
    for filename, (repo, commit, path) in SOURCES.items():
        dest = args.output / filename
        if dest.exists():
            raise RuntimeError(f'refuse overwrite: {dest}')
        raw = subprocess.check_output(['gh', 'api', f'repos/{repo}/contents/{path}?ref={commit}'], text=True)
        item = json.loads(raw)
        data = base64.b64decode(item['content'])
        assert hashlib.sha1(f'blob {len(data)}\0'.encode()+data).hexdigest() == item['sha']
        dest.write_bytes(data)
        manifest[filename] = {'repo': repo, 'commit': commit, 'path': path, 'blob_sha': item['sha'],
                              'sha256': hashlib.sha256(data).hexdigest()}
        print(filename, item['sha'], flush=True)
    (args.output/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')


if __name__ == '__main__':
    main()
