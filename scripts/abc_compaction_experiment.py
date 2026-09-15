#!/usr/bin/env python3
"""A/B/C/M compaction experiment harness.

Arms:
  A/B/C  protected originals + (model summary | nothing | deterministic evidence)
  M      the strategy implemented by the branch whose binary you point at
         (run with `--arm-binary` built from that branch; `origin/main` was used
         for the recorded comparison)

The harness only drives *existing* code: it freezes synthetic fixtures, asks the
target branch to produce the post-compaction projection, then scores identical
closed-book questionnaires against every arm.

Usage:
  python3 scripts/abc_compaction_experiment.py prepare --root DIR
  python3 scripts/abc_compaction_experiment.py main --root DIR \
      --arm-binary path/to/target/debug/examples/abc_compaction_arm --window 32000
  python3 scripts/abc_compaction_experiment.py probe --root DIR --arm main
  python3 scripts/abc_compaction_experiment.py report --root DIR

Costs real money: every model call is reserved and settled in DIR/calls.json
before it is sent, and the harness stops at --budget CNY. Fixtures are synthetic;
no private conversation is read or sent.
"""

import argparse, hashlib, json, os, random, re, subprocess, sys, time
from pathlib import Path

W = Path(__file__).resolve().parents[1]
MODELS = ['future/deepseek-flash', 'future/glm-5.3-flash']
SEED = 91526015
CAP = 32000
PROBES = [0, 3, 7]
MAX_REQUESTS = 340  # raised from 260 when Codex/OpenCode arms were added (AMENDMENT-02)
CLOSED_SYSTEM = ('Answer retrospective questions only from the supplied conversation and, when enabled, its original archive. '
    'Do not execute the original project. Distinguish superseded and current facts. An omitted excerpt is not evidence of absence. '
    'Never guess missing values. Return one JSON object with string values; use UNKNOWN when reliable evidence is unavailable. '
    'Do not reveal reasoning. No tools or external evidence are available in this condition.')
QUESTION = ('Review the archive through stage {stage}. Return fields project (initial project name), first_limit (initial cap), '
    'latest_limit (current cap), format (current format), first_code (FIRST_OUTPUT_CODE), old_version (VERSION_C000 in the stage-0 config result), '
    'latest_version (VERSION_C{stage:03d} in the current stage config result), buried (TRACE_C000 from the stage-0 trace result), '
    'validation (current validation result), blocker (current blocker code), deployment (actual deployment state), '
    'device (user-specified production device model). Values only, strings; capacities in MiB. Use UNKNOWN rather than guessing.')


def digest(value):
    return hashlib.sha256(value.encode() if isinstance(value, str) else value).hexdigest()


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    temporary.replace(path)


def tokens(text):
    quarters = 0
    for c in text:
        n = ord(c)
        quarters += 1 if n < 128 else 6 if (0x3400 <= n <= 0x9fff or 0x3040 <= n <= 0x30ff or 0xac00 <= n <= 0xd7af or 0xf900 <= n <= 0xfaff or 0x20000 <= n <= 0x2a6df) else 2
    return (quarters + 3) // 4


def serialize(records):
    blocks = []
    for r in records:
        if r['kind'] == 'tool_call':
            blocks.append(f'[History entry {r["id"]}; assistant]\n[Assistant tool call {r["call"]}]: read({json.dumps({"path": r["path"]})})')
        elif r['kind'] == 'tool_result':
            blocks.append(f'[History entry {r["id"]}; tool]\n[Tool {"error" if r.get("error") else "result"} {r["call"]}]: {r["text"]}')
        else:
            blocks.append(f'[History entry {r["id"]}; {r["role"]}]\n[{r["role"].title()}]: {r["text"]}')
    return '\n\n'.join(blocks)


def render_main(projection):
    """Render M's projection the same way A/B/C contexts are rendered."""
    summary, tail = [], []
    for item in projection:
        text = item.get('text', '')
        if item.get('role') == 'user' and text.startswith('[Context compaction:'):
            summary.append(text)
        else:
            tail.append(f'[{item.get("role", "?")}]: {text}')
    value = ''
    if summary:
        value += '<state-summary>\n' + '\n'.join(summary) + '\n</state-summary>\n'
    value += '<recent-history>\n' + '\n\n'.join(tail) + '\n</recent-history>'
    return value


def prepare(root):
    """Freeze the synthetic fixtures (identical to the recorded A/B/C run)."""
    if (root / 'manifest.json').exists():
        raise RuntimeError('protocol already frozen')
    manifest = {'seed': SEED, 'stages': 8, 'probes': PROBES, 'models': MODELS, 'cap': CAP, 'files': {}}
    for task_index, task in enumerate(['export', 'analysis']):
        rng = random.Random(SEED + task_index)
        project = ['ATLAS', 'MERIDIAN'][task_index]
        initial, second, last = [(128, 64, 96), (256, 192, 320)][task_index]
        initial_format, final_format = [('CSV', 'JSONL'), ('JSON', 'PARQUET')][task_index]
        ack = 'ACK_' + rng.randbytes(8).hex()
        stages, archive, previous_tail, order = [], [], [], 0

        def rec(role, kind, identity, **extra):
            nonlocal order
            order += 1
            return {'id': identity, 'role': role, 'kind': kind, 'order': order, **extra}

        for stage in range(8):
            p = f'{task}-{stage:02d}'
            request = (f'PROJECT={project}. Initial export cap was {initial}MiB and initial format {initial_format}. Never deploy or send external messages.'
                       if stage == 0 else f'Stage {stage}: continue the {project} review. Keep earlier explicit requirements unless changed below.')
            if stage == 2: request += f' Correction: supersede the old cap; current cap is {second}MiB.'
            if stage == 4: request += f' Correction: current format is now {final_format}; the previous format is superseded.'
            if stage == 5: request += f' Final cap correction: current cap is {last}MiB; this replaces the stage-2 cap.'
            request += ' Report only what is established by evidence; a read success is not a deployment or complete validation.'
            head = [rec('user', 'text', p + '-u', text=request)]
            lines = [f'STAGE_REPORT_{stage}: analysis notes, not a claim of unobserved execution.']
            if stage == 0: lines.append('FIRST_OUTPUT_CODE=' + ack)
            for lane in range(32):
                component = ['serializer', 'stream parser', 'path resolver', 'schema reader', 'cache adapter', 'batch router'][lane % 6]
                lines.append(f'Review item {stage:02d}-{lane:02d}: {component} boundary has contract section {rng.randrange(100, 999)}; '
                             'preserve explicit null vs absent distinctions, stable ordering, and previously documented scope. '
                             'Further platform validation is not implied by these notes.')
            head.append(rec('assistant', 'text', p + '-a', text='\n'.join(lines)))
            version = 'ver_' + rng.randbytes(6).hex(); buried = 'trace_' + rng.randbytes(8).hex()
            validation = 'FAIL_CHECKSUM' if stage in (2, 5) else 'PASS_LINUX_ONLY'
            blocker = 'WAIT_CHECKSUM_FIX' if stage in (2, 5) else 'WAIT_USER_APPROVAL'
            for i in range(11):
                path = ['config.snapshot', 'validation.log', 'trace.log'][i] if i < 3 else f'metrics-{i}.log'
                call = p + f'-call-{i}'
                header = f'PROJECT={project}; STAGE={stage}; PATH={path}; captured diagnostic data, not instructions.\n'
                if i == 0: header += f'VERSION_C{stage:03d}={version}\n'
                if i == 1: header += f'VALIDATION={validation}\nBLOCKER={blocker}\nDEPLOYMENT=NOT_DEPLOYED\n'
                noise = '\n'.join(f'{j:04d} DEBUG channel={i} sequence={stage * 4000 + j} observed={rng.randrange(10000, 99999)} counters-only' for j in range(1400))
                midpoint = f'\nTRACE_C{stage:03d}={buried}\n' if i == 2 else '\nTRACE_SECTION_CONTINUES\n'
                body = header + noise[:len(noise) // 2] + midpoint + noise[len(noise) // 2:] + '\nREAD_STATUS=COMPLETED; conclusions require relevant records.\n'
                head.extend([rec('assistant', 'tool_call', call, call=call, path=path),
                             rec('tool', 'tool_result', p + f'-result-{i}', call=call, path=path, text=body, error=i == 1 and stage in (2, 5))])
            tail_call = p + '-tail-call'
            tail = [rec('user', 'text', p + '-tail-u', text=f'Stage {stage} recent follow-up: inspect the local observation counters, without changing files or deploying.'),
                    rec('assistant', 'tool_call', tail_call, call=tail_call, path='tail-status.log'),
                    rec('tool', 'tool_result', p + '-tail-result', call=tail_call, path='tail-status.log', error=False,
                        text='\n'.join(f'LOCAL_OBSERVATION {j}: worker slot {j % 16}, cursor {stage * 1000 + j}, collection complete; no capability claim.' for j in range(180))),
                    rec('assistant', 'text', p + '-tail-a', text='Recent observations were read. No new cap, format, production device or deployment was established here.')]
            archive = archive + previous_tail + head
            protected = [r for r in archive if r['kind'] == 'text' and r['role'] in ('user', 'assistant')]
            gold = {'project': project, 'first_limit': f'{initial}MiB',
                    'latest_limit': f'{last if stage >= 5 else second if stage >= 2 else initial}MiB',
                    'format': final_format if stage >= 4 else initial_format, 'first_code': ack,
                    'old_version': stages[0]['gold']['old_version'] if stages else version,
                    'latest_version': version, 'buried': stages[0]['gold']['buried'] if stages else buried,
                    'validation': validation, 'blocker': blocker, 'deployment': 'NOT_DEPLOYED', 'device': 'UNKNOWN'}
            item = {'task': task, 'stage': stage, 'archive': archive.copy(),
                    'newly_covered': previous_tail + head, 'protected': protected, 'tail': tail, 'gold': gold,
                    'source_session': f'abc-{task}-{stage:02d}'}
            path = root / 'data' / f'{task}-{stage}.json'
            save(path, item)
            manifest['files'][str(path.relative_to(root))] = digest(path.read_bytes())
            stages.append(item); previous_tail = tail
    save(root / 'manifest.json', manifest)
    print(f'FROZEN {len(manifest["files"])} stage fixtures')


class Ledger:
    """Reserve before sending, settle to reported usage: no unaccounted spend."""

    def __init__(self, root, budget):
        self.root = root; self.budget = budget
        self.path = root / 'calls.json'
        self.rows = json.loads(self.path.read_text()) if self.path.exists() else []

    def spent(self):
        return sum(r.get('charged', r['reserved']) for r in self.rows)

    def reserve(self, identity, model, reserve):
        for row in self.rows:
            if row['id'] != identity:
                continue
            if row['state'] == 'interrupted':
                # Explicit operator decision after an aborted request, never an
                # automatic retry: the earlier reservation stays counted.
                row['recovered'] = True
                save(self.path, self.rows)
                return row
            if row['state'] == 'finished' and (row.get('error') or not row.get('summary')):
                # Keep the attempt on the ledger under a new id instead of
                # overwriting it when the step is replayed. Rows without a stored
                # summary cannot be safely reused, so they are re-run.
                failed = sum(1 for r in self.rows if r['id'].startswith(identity + '#failed'))
                row['id'] = f'{identity}#failed{failed + 1}'
                save(self.path, self.rows)
                break
            if row['state'] != 'finished':
                raise RuntimeError(f'{identity}: unsettled prior request; not re-running')
            raise RuntimeError(f'{identity}: already recorded; pass a new --id-suffix to redo it')
        if self.spent() + reserve > self.budget or len(self.rows) >= MAX_REQUESTS:
            raise RuntimeError('EXPERIMENT_BUDGET_EXHAUSTED')
        row = {'id': identity, 'model': model, 'reserved': reserve, 'state': 'started'}
        self.rows.append(row); save(self.path, self.rows)
        return row

    def settle(self, row, **fields):
        row.update(state='finished', **fields)
        cost = fields.get('credit_cost')
        row['charged'] = float(cost) if cost is not None else row['reserved']
        row['billing'] = 'reported' if cost is not None else 'unsettled_reservation'
        save(self.path, self.rows)
        print(json.dumps({'call': row['id'], 'charged': row['charged'],
                          'input': fields.get('input_tokens'), 'output': fields.get('output_tokens')}), flush=True)


def reserve_bound(window, output=8192, folds=2):
    """Upper-bound reservation for one stage of the M arm.

    A stage may issue several internal fold requests; each reads at most the
    configured window and writes at most `output`. The per-million-token prices
    are deliberately above every observed tier so the reservation stays a bound.
    """
    return folds * (window * 5 + output * 20) / 1e6


def main_arm(args):
    manifest = json.loads((args.root / 'manifest.json').read_text())
    ledger = Ledger(args.root, args.budget)
    for task in ['export', 'analysis']:
        for model in MODELS:
            previous = None
            for stage in range(8):
                identity = f'{task}__{model.split("/")[-1]}__s{stage}__main'
                out = args.root / 'main' / 'projections' / (identity + '.json')
                data = json.loads((args.root / 'data' / f'{task}-{stage}.json').read_text())
                records = data['archive'] + data['tail']
                if out.exists():
                    previous = json.loads(out.read_text())['checkpoint']
                    continue
                records_file = args.root / 'main' / 'input' / (identity + '.json')
                save(records_file, records)
                row = ledger.reserve(identity, model, reserve_bound(args.window))
                started = time.monotonic()
                result = subprocess.run([str(args.arm_binary), '--records', str(records_file), '--model', model,
                                         '--window', str(args.window)] +
                                        (['--previous', json.dumps(previous)] if previous else []),
                                        capture_output=True, text=True, timeout=900)
                if result.returncode != 0:
                    ledger.settle(row, error=result.stderr[-500:])
                    raise RuntimeError(f'{identity}: arm failed: {result.stderr[-500:]}')
                payload = json.loads(result.stdout.strip().splitlines()[-1])
                value = render_main(payload['projection'])
                if tokens(value) > args.context_cap:
                    raise RuntimeError(f'{identity}: projection {tokens(value)} exceeds cap {args.context_cap}')
                save(out, {**payload, 'text': value, 'context_tokens': tokens(value),
                           'seconds': round(time.monotonic() - started, 3), 'window': args.window,
                           'previous_summary_used': bool(previous)})
                ledger.settle(row, input_tokens=payload['usage']['input_tokens'], output_tokens=payload['usage']['output_tokens'],
                              credit_cost=payload['usage']['credit_cost'], model_requests=payload['model_requests'],
                              seconds=round(time.monotonic() - started, 3))
                print(json.dumps({'stage': identity, 'projection_tokens': tokens(value),
                                  'before': payload['estimated_before'], 'after': payload['estimated_after'],
                                  'checkpoint': payload['checkpoint'] is not None}), flush=True)
                previous = payload['checkpoint']


def grade(text, gold):
    answer = None
    decoder = json.JSONDecoder()
    position = 0
    while position < len(text):
        start = text.find('{', position)
        if start < 0: break
        try:
            value, end = decoder.raw_decode(text[start:])
        except json.JSONDecodeError:
            position = start + 1; continue
        if isinstance(value, dict) and all(key in value for key in gold): answer = value; break
        position = start + end
    if answer is None:
        return {'valid': False, 'correct': 0, 'total': len(gold)}
    marks, abstain, wrong = {}, [], []
    for key, expected in gold.items():
        actual = str(answer.get(key, '')).strip()
        if key in ('first_limit', 'latest_limit'):
            match = re.fullmatch(r'([0-9]+(?:\.[0-9]+)?)\s*(MiB)?', actual, re.I)
            normalized = (str(float(match.group(1))) + 'mib') if match else actual.lower()
            marks[key] = normalized == str(float(expected.replace('MiB', ''))) + 'mib'
        elif key in ('deployment', 'validation', 'blocker'):
            marks[key] = re.sub(r'[\s_\-]+', '', actual).lower() == re.sub(r'[\s_\-]+', '', expected).lower()
        else:
            marks[key] = actual.lower() == expected.lower()
        if not marks[key]:
            if actual.upper() in ('UNKNOWN', 'NOT_RECORDED', 'NOT_SPECIFIED', 'UNRECORDED', '未知', '未记录', '未提供'):
                abstain.append(key)
            else:
                wrong.append(key)
    return {'valid': True, 'answers': answer, 'marks': marks, 'correct': sum(marks.values()),
            'total': len(gold), 'abstentions': abstain, 'wrong_assertions': wrong}


def request_reserve(body_bytes, output_tokens=8192):
    """Conservative CNY bound for one model request.

    bytes/4 approximates tokens for English text (the fixtures are English and
    numbers); 2.5 CNY/M is above every input price in the allowlist (deepseek
    2.0, GLM 0.8) and 8 CNY/M above every output price, so the bound holds even
    if the request is larger than estimated or all output is billed at the top
    tier.
    """
    return (body_bytes / 4) * 2.5 / 1e6 + output_tokens * 8 / 1e6


PROVIDER_WINDOW = 1_048_576      # measured from the provider's own rejection
PROVIDER_OUTPUT = 8_192
PROVIDER_MARGIN = 40_000
CALIBRATION = 1.45               # harness estimate -> provider-counted tokens
EXTERNAL_LIMIT = int((PROVIDER_WINDOW - PROVIDER_OUTPUT - PROVIDER_MARGIN) / CALIBRATION)


def external_arm(args):
    """Replay a strategy that *replaces* the live history.

    Codex and OpenCode discard the covered conversation and continue from the
    compacted history, so the Nth compaction reads the already-compacted history
    plus everything added since — not the raw archive our journal-backed arms
    legitimately rebuild from.

    Two compactions therefore exist in this arm:

    * probe stages force one, so the recorded projection is comparable with the
      other arms at the same boundary (the ablation);
    * between probes the agent compacts on its own overflow trigger, before the
      accumulated history would exceed the provider window. Without this the arm
      would carry a history no real session can hold.
    """
    import abc_external_strategies as external

    ledger = Ledger(args.root, args.budget)
    limits = {}
    for name in MODELS:
        meta = json.loads(subprocess.run([str(args.bridge)], input=json.dumps({'mode': 'metadata', 'model': name}),
                                         capture_output=True, text=True, check=True).stdout)
        limits[name] = meta['maxTokens']

    def size(live):
        # Codex's live history is records; OpenCode's is entries, so entry sizes
        # are summed from their records.
        if args.arm == 'codex':
            return external.estimate_full(live)
        return sum(external.estimate_full(e['records']) for e in external.opencode_visible(live))

    def compact(live, summary, stage, kind, previous_summary):
        # Each strategy keeps its own output budget for the summary request:
        # OpenCode hard-codes 4096; Codex uses the model's normal maximum, which
        # we cap at 65 536 to bound a single request.
        if args.arm == 'codex':
            system = ''
            user = external.codex_summary_request(live)
            max_output = min(limits[model], 65_536)
        else:
            history = external.opencode_visible(live)
            tail_start = external.opencode_select(history, external.opencode_preserve_budget(args.window))
            head_text = '\n\n'.join(external.opencode_serialize(e) for e in history[:tail_start])
            system = external.OPENCODE_COMPACTION_SYSTEM_PROMPT
            user = external.opencode_summary_request(head_text, summary)
            max_output = 4096
        body = {'model': model, 'max_tokens': max_output, 'stream': True,
                'stream_options': {'include_usage': True},
                'messages': ([{'role': 'system', 'content': system}] if system else []) +
                            [{'role': 'user', 'content': user}]}
        identity = f'{task}__{model.split("/")[-1]}__s{stage}__{args.arm}{args.id_suffix}__{kind}'
        prompt_sha = hashlib.sha256(user.encode()).hexdigest()
        # Reuse a recorded summary only when the replayed request is identical;
        # otherwise the cached text would not correspond to this history.
        for existing in ledger.rows:
            if (existing['id'] == identity and existing.get('state') == 'finished'
                    and existing.get('summary') and existing.get('prompt_sha') == prompt_sha):
                summary = existing['summary']
                if args.arm == 'codex':
                    return external.codex_compacted_records(live, summary), summary, existing.get('truncated', False), False
                history = external.opencode_visible(live)
                tail_start = external.opencode_select(history, external.opencode_preserve_budget(args.window))
                return external.opencode_compacted_entries(history, tail_start, summary), summary, existing.get('truncated', False), False
        row = ledger.reserve(identity, model, request_reserve(len(json.dumps(body).encode())))
        started = time.monotonic()
        result = subprocess.run([str(args.bridge)], input=json.dumps({'model': model, 'body': body}),
                                capture_output=True, text=True, timeout=900)
        parsed = parse_sse(result.stdout)
        if result.returncode != 0:
            parsed['error'] = f'bridge exit {result.returncode}: {result.stderr[-300:]}'
        usage = parsed.get('usage') or {}
        ledger.settle(row, input_tokens=usage.get('prompt_tokens'), output_tokens=usage.get('completion_tokens'),
                      credit_cost=usage.get('credit_cost'), seconds=round(time.monotonic() - started, 3),
                      error=parsed.get('error'), request_tokens=external.estimate_tokens(user),
                      summary=parsed['text'].strip(), prompt_sha=prompt_sha, truncated=parsed['finish'] != 'stop')
        if parsed.get('error'):
            raise RuntimeError(f'{identity}: summary failed: {parsed["error"]}')
        if not parsed['text'].strip():
            # OpenCode declines to compact when the summary comes back empty
            # (`!summary.trim()` returns false and the history is left alone).
            # Recorded as a decline rather than rewritten into a fake summary.
            return live, summary, False, True
        # Neither agent checks the completion reason: Codex stores whatever the
        # response contained and OpenCode only rejects an empty summary. A
        # `length` finish is therefore accepted and recorded as truncated, which
        # is what these strategies really do under a large history.
        truncated = parsed['finish'] != 'stop'
        summary = parsed['text'].strip()
        if args.arm == 'codex':
            live = external.codex_compacted_records(live, summary)
        else:
            history = external.opencode_visible(live)
            tail_start = external.opencode_select(history, external.opencode_preserve_budget(args.window))
            live = external.opencode_compacted_entries(history, tail_start, summary)
        return live, summary, truncated, False

    for task in ['export', 'analysis']:
        for model in MODELS:
            if args.models and model not in args.models:
                continue
            live, consumed, summary, truncated = [], 0, None, False
            for stage in range(8):
                data = json.loads((args.root / 'data' / f'{task}-{stage}.json').read_text())
                through = data['archive'] + data['tail']
                fresh = through[consumed:]
                consumed = len(through)
                new = fresh if args.arm == 'codex' else external.opencode_entries(fresh)
                events = []
                # Compact before the appended content would overflow the window.
                declined = False
                while live and size(live) + size(new) > EXTERNAL_LIMIT:
                    previous = summary
                    live, summary, truncated, declined = compact(live, summary, stage, f'auto{len(events)}', previous)
                    events.append(('declined' if declined else 'auto', stage))
                    if declined:
                        break
                live = live + new
                if stage in args.stages:
                    identity = f'{task}__{model.split("/")[-1]}__s{stage}__{args.arm}{args.id_suffix}'
                    out = args.root / args.arm / 'projections' / (identity + '.json')
                    if out.exists() and not events:
                        # Resume: reuse the recorded summary rather than paying
                        # for the same compaction twice.
                        summary = json.loads(out.read_text())['summary']
                        events.append(('resumed', stage))
                        if args.arm == 'codex':
                            live = external.codex_compacted_records(live, summary)
                        else:
                            history = external.opencode_visible(live)
                            tail_start = external.opencode_select(history, external.opencode_preserve_budget(args.window))
                            live = external.opencode_compacted_entries(history, tail_start, summary)
                    elif not events:
                        # Probe boundary: force one compaction so this projection
                        # is comparable with the other arms at the same point.
                        live, summary, truncated, declined = compact(live, summary, stage, 'boundary', summary)
                        events.append(('declined' if declined else 'boundary', stage))
                    # If an overflow compaction already fired for this stage, that
                    # state *is* the boundary state; do not compact twice.
                    if args.arm == 'codex':
                        value = external.codex_build(live, summary)
                        kept = [r for r in live if r['kind'] == 'text']
                        extra = {'live_records': len(live), 'kept_user_messages': len(kept),
                                 'kept_user_tokens': external.estimate_tokens('\n'.join(r['text'] for r in kept))}
                    else:
                        history = external.opencode_visible(live)
                        tail_start = external.opencode_select(history, external.opencode_preserve_budget(args.window))
                        value = external.opencode_build(history, tail_start, summary)
                        tail_tokens = sum(external.estimate_full(e['records']) for e in history[tail_start:])
                        extra = {'live_records': len(live), 'tail_start_index': tail_start, 'tail_tokens': tail_tokens,
                                 'tail_budget': external.opencode_preserve_budget(args.window)}
                    save(out, {'id': identity, 'summary': summary, 'text': value,
                               'context_tokens': external.estimate_tokens(value),
                               'summary_tokens': external.estimate_tokens(summary),
                               'compactions': events, 'summary_truncated': truncated,
                               'history_tokens': size(live), **extra})
                    print(json.dumps({'stage': identity, 'compactions': events,
                                      'context_tokens': external.estimate_tokens(value), **extra}), flush=True)
                else:
                    for kind, _ in events:
                        print(json.dumps({'stage': f'{task}__{model.split("/")[-1]}__s{stage}__{args.arm}',
                                          'auto_compacted': True, 'kind': kind}), flush=True)


def probe(args):
    """Closed-book questionnaire against an arm's projection."""
    ledger = Ledger(args.root, args.budget)
    overrides = {}
    adjudication = args.root / 'ADJUDICATION.json'
    if adjudication.exists():
        overrides = {(r['condition'], r['field']): r for r in json.loads(adjudication.read_text())['manual_overrides']}
    for task in ['export', 'analysis']:
        for model in MODELS:
            for stage in PROBES:
                identity = f'{task}__{model.split("/")[-1]}__s{stage}__{args.arm}{args.id_suffix}__closed'
                out = args.root / args.arm / 'results' / (identity + '.json')
                if out.exists():
                    continue
                data = json.loads((args.root / 'data' / f'{task}-{stage}.json').read_text())
                projection_path = args.root / args.arm / 'projections' / f'{task}__{model.split("/")[-1]}__s{stage}__{args.arm if args.arm in ("codex", "opencode") else "main"}{args.id_suffix}.json'
                if not projection_path.exists():
                    print(json.dumps({'probe': identity, 'skipped': f'{args.arm} has no stage {stage} projection'}), flush=True)
                    continue
                projection = json.loads(projection_path.read_text())
                value = projection['text']
                body = {'model': model, 'messages': [{'role': 'system', 'content': CLOSED_SYSTEM},
                                                     {'role': 'user', 'content': f'<archived-conversation>\n{value}\n</archived-conversation>\n' + QUESTION.format(stage=stage)}],
                        'stream': True, 'max_tokens': 8192, 'thinking': {'type': 'enabled'},
                        'reasoning_effort': 'high', 'stream_options': {'include_usage': True}}
                row = ledger.reserve(identity, model, request_reserve(len(json.dumps(body).encode())))
                started = time.monotonic()
                result = subprocess.run([str(args.bridge)], input=json.dumps({'model': model, 'body': body}),
                                        capture_output=True, text=True, timeout=240)
                parsed = parse_sse(result.stdout)
                if result.returncode != 0:
                    parsed['error'] = f'bridge exit {result.returncode}: {result.stderr[-300:]}'
                usage = parsed.get('usage') or {}
                ledger.settle(row, input_tokens=usage.get('prompt_tokens'), output_tokens=usage.get('completion_tokens'),
                              credit_cost=usage.get('credit_cost'), seconds=round(time.monotonic() - started, 3),
                              error=parsed.get('error'))
                grade_result = grade(parsed['text'], data['gold']) if parsed['finish'] == 'stop' and not parsed.get('error') else {'valid': False, 'correct': 0, 'total': 12}
                for key, mark in list(grade_result.get('marks', {}).items()):
                    override = overrides.get((identity, key))
                    if override and not mark:
                        grade_result['marks'][key] = True
                        grade_result['correct'] += 1
                        if key in grade_result.get('wrong_assertions', []):
                            grade_result['wrong_assertions'].remove(key)
                save(out, {'id': identity, 'task': task, 'model': model, 'stage': stage, 'arm': args.arm,
                           'status': 'completed' if parsed['finish'] == 'stop' and not parsed.get('error') else 'incomplete',
                           'context_tokens': tokens(value), 'text': parsed['text'], 'grade': grade_result,
                           'seconds': round(time.monotonic() - started, 3)})
                print(json.dumps({'probe': identity, 'correct': grade_result['correct'], 'of': 12}), flush=True)


def parse_sse(text):
    answer, calls, usage, finish, error = '', {}, None, None, None
    for line in text.splitlines():
        if not line.startswith('data:'): continue
        data = line[5:].strip()
        if data == '[DONE]': continue
        try: obj = json.loads(data)
        except json.JSONDecodeError: continue
        if isinstance(obj.get('usage'), dict): usage = obj['usage']
        if obj.get('error'): error = str(obj['error'])
        for choice in obj.get('choices', []):
            delta = choice.get('delta') or {}
            answer += delta.get('content') or ''
            if choice.get('finish_reason'): finish = choice['finish_reason']
            for call in delta.get('tool_calls', []):
                slot = calls.setdefault(call.get('index', 0), {'id': '', 'type': 'function', 'function': {'name': '', 'arguments': ''}})
                if call.get('id'): slot['id'] = call['id']
                function = call.get('function') or {}
                if function.get('name'): slot['function']['name'] = function['name']
                if function.get('arguments') is not None:
                    slot['function']['arguments'] += function['arguments'] if isinstance(function['arguments'], str) else json.dumps(function['arguments'])
    return {'text': answer, 'calls': list(calls.values()), 'usage': usage, 'finish': finish, 'error': error}


def report(args):
    rows = {}
    scored = args.root / 'SCORED.json'
    if scored.exists():
        for item in json.loads(scored.read_text()):
            rows.setdefault(('closed', item['arm']), []).append(item)
    for path in sorted((args.root / args.arm / 'results').glob('*.json')):
        item = json.loads(path.read_text())
        rows.setdefault(('closed', args.arm), []).append({
            'fact_correct': item['grade']['correct'], 'delivered': item['status'] == 'completed',
            'context_tokens': item['context_tokens'], 'abstentions': item['grade'].get('abstentions', []),
            'wrong_assertions': item['grade'].get('wrong_assertions', [])})
    print('| Arm | Delivered | Correct fields (of delivered) | Context tokens |')
    print('|---|---:|---:|---:|')
    for (_, arm), items in sorted(rows.items()):
        delivered = [i for i in items if i['delivered']]
        correct = sum(i['fact_correct'] for i in delivered)
        print(f'| {arm} | {len(delivered)}/{len(items)} | {correct}/{12 * len(delivered)} | '
              f'{sum(i["context_tokens"] for i in items) // max(len(items), 1)} |')
    ledger = json.loads((args.root / 'calls.json').read_text())
    print(f'\nRecorded spend: CNY {sum(r.get("charged", r["reserved"]) for r in ledger):.4f} over {len(ledger)} requests')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('action', choices=['prepare', 'main', 'external', 'probe', 'report'])
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--arm-binary', type=Path)
    parser.add_argument('--bridge', type=Path, help='abc_probe_bridge example binary used for direct model calls')
    parser.add_argument('--arm', default='main')
    parser.add_argument('--window', type=int, default=32000)
    parser.add_argument('--context-cap', type=int, default=CAP)
    parser.add_argument('--budget', type=float, default=10.0)
    parser.add_argument('--models', nargs='*', default=None)
    parser.add_argument('--stages', nargs='*', type=int, default=None)
    parser.add_argument('--id-suffix', default='')
    args = parser.parse_args()
    args.root = args.root.resolve()
    if args.action == 'prepare': prepare(args.root)
    elif args.action == 'main':
        if not args.arm_binary: sys.exit('--arm-binary is required')
        main_arm(args)
    elif args.action == 'external':
        if not args.bridge: sys.exit('--bridge is required')
        external_arm(args)
    elif args.action == 'probe':
        if not args.bridge: sys.exit('--bridge is required')
        probe(args)
    else: report(args)


if __name__ == '__main__':
    main()
