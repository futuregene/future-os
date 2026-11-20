#!/usr/bin/env python3
"""Compare frozen prompt variants against an explicitly versioned gold review."""
import argparse
import concurrent.futures
import copy
import json
import random
from pathlib import Path

import benchmark as bench


def reviewed_cases():
    original = bench.read(bench.HERE / 'cases.json')['cases']
    review = bench.read(bench.HERE / 'reviews/gold-v2.json')
    if review['base_cases_sha256'] != bench.digest((bench.HERE / 'cases.json').read_bytes()):
        raise ValueError('Review does not match frozen inputs')
    annotations = {row['id']: row for row in review['reviews']}
    if set(annotations) != {case['id'] for case in original}:
        raise ValueError('Every original case needs an explicit review')
    revised = copy.deepcopy(original)
    for case in revised:
        row = annotations[case['id']]
        if row['original'] != case['expected']:
            raise ValueError('Review base labels changed')
        case['expected'].update(row['patch'])
        case['rationale'] = row['rationale']
        case['acceptable'] = row['acceptable']
    return revised


def evaluate(cases, records):
    metrics = bench.summarize(cases, records)
    metrics['accepted_label_correct'] = {}
    for name in ['risk', 'authorization', 'reason_code']:
        metrics['accepted_label_correct'][name] = sum(
            case['id'] in records and 'error' not in records[case['id']]
            and records[case['id']]['assessment']['reported'][name]
            in case.get('acceptable', {}).get(name, [case['expected'][name]])
            for case in cases)
    metrics['scorer_sha256'] = bench.digest(Path(__file__).read_bytes())
    metrics['classification_errors'] = []
    for case in cases:
        record = records.get(case['id'])
        if not record or 'error' in record:
            continue
        reported = record['assessment']['reported']
        differences = {key: reported[key] for key in ['risk', 'authorization', 'reason_code']
                       if reported[key] not in case.get('acceptable', {}).get(key, [case['expected'][key]])}
        if differences:
            metrics['classification_errors'].append(dict(id=case['id'], expected=case['expected'],
                differences=differences, decision=record['verdict']['decision'], rationale=case['rationale']))
    metrics['errors_by_case'] = []
    for case in cases:
        record = records.get(case['id'])
        if not record or 'error' in record:
            metrics['errors_by_case'].append(dict(id=case['id'], error=record.get('error') if record else 'missing'))
        elif record['verdict']['decision'] != case['expected']['decision']:
            metrics['errors_by_case'].append(dict(id=case['id'], expected=case['expected'],
                reported=record['assessment']['reported'], verdict=record['verdict'],
                confidence=record['assessment']['confidence']))
    return metrics


def records_at(folder, cases, questions):
    manifest = bench.read(folder.parent / 'manifest.json')
    manifest_hash = bench.digest(bench.encoded(manifest))
    expected_model = manifest.get('model')
    if expected_model is None:
        expected_model = manifest['models'][int(folder.name.removeprefix('model-'))]
    route = manifest.get('request_model') or manifest['model_routes'][expected_model]
    by_id = {case['id']: case for case in cases}
    records = {}
    for path in sorted(folder.glob('*.json')):
        record = bench.read(path)
        case = by_id[record['case_id']]
        payload = dict(model=route, state=case['state'], questions=questions)
        if (record['manifest_sha256'] != manifest_hash
                or record['request_sha256'] != bench.digest(bench.encoded(payload))
                or record['requested_model'] != expected_model):
            raise ValueError('Response identity mismatch in ' + str(path))
        records[case['id']] = record
    return records


def load_cases(dataset):
    if dataset == 'reviewed':
        return reviewed_cases()
    if dataset == 'holdout':
        return bench.read(bench.HERE / 'holdout.json')['cases']
    if dataset == 'frozen':
        lock = bench.read(bench.HERE / 'baseline-lock.json')
        path = bench.HERE / lock['suite']
        if bench.digest(path.read_bytes()) != lock['files'][lock['suite']]:
            raise ValueError('Frozen 140-case suite hash differs')
        return bench.read(path)['cases']
    raise ValueError('Unknown dataset')


def validate(cases, questions):
    if len({case['id'] for case in cases}) != len(cases):
        raise ValueError('Duplicate cases')
    for case in cases:
        expected = case['expected']
        verdict = bench.decide(bench.decode(bench.certain(expected, questions), questions))
        if verdict['decision'] != expected['decision']:
            raise ValueError('Gold/policy inconsistency in ' + case['id'])
        payload = dict(model='decider', state=case['state'], questions=questions)
        if len(bench.encoded(case['state'])) > 8000 or len(bench.encoded(payload)) > 12000:
            raise ValueError('Input budget exceeded')
    if len(bench.encoded(questions)) > 4000:
        raise ValueError('Production questions budget exceeded')
    bench.validate_policy(questions)


def run(args):
    cases = load_cases(args.dataset)
    profile = bench.read(bench.HERE / 'prompts' / (args.profile + '.json'))
    protocol = dict(questions=profile['questions'])
    validate(cases, protocol['questions'])
    key, endpoint = bench.credentials(None)
    folder = bench.HERE / 'runs' / args.name
    manifest = dict(schema_version=1, model=bench.MODELS[0], request_model='decider', endpoint=endpoint,
        profile=args.profile, profile_sha256=bench.digest(bench.encoded(profile)), dataset=args.dataset,
        cases_sha256=bench.digest(bench.encoded(cases)), gold_revision='gold-v2',
        policy_sources={source: bench.digest((bench.REPO / source).read_bytes()) for source in bench.SOURCES[1:3]},
        runner_sha256=bench.digest(Path(__file__).read_bytes()), transport_sha256=bench.digest(Path(bench.__file__).read_bytes()),
        concurrency=4, order_seed=20261120)
    path = folder / 'manifest.json'
    if path.exists() and bench.read(path) != manifest:
        raise ValueError('Run configuration changed. Use a new name.')
    bench.write(path, manifest)
    digest = bench.digest(bench.encoded(manifest))
    ordered = list(cases)
    random.Random(20261120).shuffle(ordered)
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        futures = [pool.submit(bench.run_case, case, bench.MODELS[0], 0, key, endpoint, protocol, folder, digest) for case in ordered]
        for count, future in enumerate(concurrent.futures.as_completed(futures), 1):
            record = future.result()
            print(count, '/', len(cases), record['case_id'], record.get('error') or record['verdict']['status'], flush=True)
    metrics = evaluate(cases, records_at(folder / 'model-0', cases, protocol['questions']))
    bench.write(folder / 'summary.json', metrics)
    print(json.dumps(metrics, ensure_ascii=False, indent=2))


def compare(args):
    cases = reviewed_cases()
    baseline = bench.HERE / 'runs/baseline-20261120-aliases'
    baseline_questions = bench.read(bench.HERE / 'protocol.json')['questions']
    metrics = {'original-decider-reviewed': evaluate(cases, records_at(baseline / 'model-0', cases, baseline_questions)),
               'original-jev-reviewed': evaluate(cases, records_at(baseline / 'model-1', cases, baseline_questions))}
    for name in args.runs:
        folder = bench.HERE / 'runs' / name
        manifest = bench.read(folder / 'manifest.json')
        run_cases = load_cases(manifest['dataset'])
        if manifest['cases_sha256'] != bench.digest(bench.encoded(run_cases)):
            raise ValueError('Run dataset differs from frozen gold')
        questions = bench.read(bench.HERE / 'prompts' / (manifest['profile'] + '.json'))['questions']
        metrics[name] = evaluate(run_cases, records_at(folder / 'model-0', run_cases, questions))
    output = bench.HERE / 'runs' / args.output
    bench.write(output / 'comparison.json', metrics)
    lines = ['# Decider prompt evaluation', '',
             'Baseline and reviewed runs use the same 100 original inputs with gold-v2. Holdout runs use the additional 40 inputs. Frozen runs use the locked 140-case suite. The initial gold and responses remain archived.', '',
             '| Run | Correct | False allow | False deny | Allowed / expected allow | Risk / auth / reason exact | Accepted labels |',
             '| --- | --- | --- | --- | --- | --- | --- |']
    for name, row in metrics.items():
        allowed = row['by_decision']['allow']
        strict = ' / '.join(str(row['label_correct'][key]) for key in ['risk','authorization','reason_code'])
        accepted = ' / '.join(str(row['accepted_label_correct'][key]) for key in ['risk','authorization','reason_code'])
        lines.append(f'| {name} | {row["correct"]}/{row["total"]} | {row["false_allow"]} | {row["false_deny"]} | {allowed["correct"]}/{allowed["total"]} | {strict} | {accepted} |')
    for name, row in metrics.items():
        lines += ['', '## ' + name, '', '| ID | Expected | Reported | Status |', '| --- | --- | --- | --- |']
        for error in row['errors_by_case']:
            if 'error' in error:
                lines.append(f'| {error["id"]} | | | {error["error"]} |')
            else:
                lines.append(f'| {error["id"]} | {error["expected"]["decision"]} | {json.dumps(error["reported"])} | {error["verdict"]["status"]} |')
    for name, row in metrics.items():
        lines += ['', '## Classification differences in ' + name, '',
                  '| ID | Gold labels | Predicted differences | Actual decision |', '| --- | --- | --- | --- |']
        for error in row['classification_errors']:
            lines.append(f'| {error["id"]} | {json.dumps(error["expected"])} | {json.dumps(error["differences"])} | {error["decision"]} |')
    (output / 'report.md').write_text('\n'.join(lines) + '\n')
    print(json.dumps({name:{key:row[key] for key in ['correct','false_allow','false_deny','label_correct','accepted_label_correct']} for name,row in metrics.items()},indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    runner = commands.add_parser('run')
    runner.add_argument('--name', required=True)
    runner.add_argument('--profile', required=True)
    runner.add_argument('--dataset', choices=['frozen','reviewed','holdout'], default='frozen')
    validation = commands.add_parser('validate')
    validation.add_argument('--profile', default='decider-taxonomy-v1')
    comparison = commands.add_parser('compare')
    comparison.add_argument('runs', nargs='*')
    comparison.add_argument('--output', default='decider-prompt-comparison')
    args = parser.parse_args()
    names = [getattr(args,'name',''),getattr(args,'profile',''),getattr(args,'output',''),*getattr(args,'runs',[])]
    if any(name and (not name.replace('-','').replace('_','').isalnum() or '/' in name) for name in names):
        parser.error('Names must use letters, digits, hyphens and underscores')
    if args.command == 'run':
        run(args)
    elif args.command == 'validate':
        questions = bench.read(bench.HERE / 'prompts' / (args.profile + '.json'))['questions']
        for dataset in ['reviewed', 'holdout', 'frozen']:
            cases = load_cases(dataset)
            validate(cases, questions)
            print('Validated', len(cases), dataset, 'cases')
        if questions != bench.production_questions():
            raise ValueError('Candidate differs from current production prompt')
        policy = 'agent/src/approval_review/policy.rs'
        archived = bench.read(bench.HERE / 'protocol.json')
        if bench.digest((bench.REPO / policy).read_bytes()) != archived['sources'][policy]:
            raise ValueError('Production decision policy differs from evaluated policy')
        print('Production questions match the evaluated candidate,', len(bench.encoded(questions)), 'bytes')
    else:
        compare(args)


if __name__ == '__main__':
    main()
