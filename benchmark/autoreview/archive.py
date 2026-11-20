#!/usr/bin/env python3
"""Verify the selected frozen 140-case suite and its original paired results."""
import json

import benchmark as bench
import experiment


def main():
    root = bench.HERE
    lock = bench.read(root / 'baseline-lock.json')
    for relative, expected in lock['files'].items():
        path = root / relative
        if bench.digest(path.read_bytes()) != expected:
            raise ValueError('Frozen file hash differs in ' + relative)
    cases = bench.read(root / lock['suite'])['cases']
    archive = root / lock['baseline']
    manifest = bench.read(archive / 'manifest.json')
    questions = bench.read(archive / 'questions.json')
    saved = bench.read(archive / 'comparison.json')
    runner = bench.read(archive / 'runner-source.json')
    if bench.digest(runner['source'].encode()) != runner['sha256'] or runner['sha256'] != manifest['runner_sha256']:
        raise ValueError('Original runner source differs')
    if manifest['run_id'] != lock['source_run_id'] or saved['manifest'] != manifest:
        raise ValueError('Original run identity differs')
    if bench.digest(bench.encoded(cases)) != manifest['cases_sha256']:
        raise ValueError('Frozen case identity differs')
    profile = dict(revision=manifest['profile'], questions=questions)
    if bench.digest(bench.encoded(profile)) != manifest['profile_sha256']:
        raise ValueError('Frozen prompt identity differs')
    experiment.validate(cases, questions)
    by_id = {case['id']: case for case in cases}
    if len(cases) != 140 or len(by_id) != 140:
        raise ValueError('Expected 140 unique frozen cases')
    records = [{}, {}]
    for line in (archive / 'responses.jsonl').read_text().splitlines():
        row = json.loads(line)
        index, record = row['model_index'], row['record']
        model = manifest['models'][index]
        case = by_id[record['case_id']]
        payload = dict(model=manifest['model_routes'][model], state=case['state'], questions=questions)
        if (record['manifest_sha256'] != bench.digest(bench.encoded(manifest))
                or record['request_sha256'] != bench.digest(bench.encoded(payload))
                or record['requested_model'] != model
                or record['served_model'] != model
                or record['response']['model'] != model):
            raise ValueError('Original response identity differs in ' + case['id'])
        if case['id'] in records[index] or 'error' in record:
            raise ValueError('Duplicate or failed archived response')
        decoded = bench.decode(record['response'], questions)
        if decoded != record['assessment']:
            raise ValueError('Stored classification differs from original response')
        records[index][case['id']] = record
    counts = dict(cases=len(cases), expected_allow=sum(case['expected']['decision']=='allow' for case in cases),
                  expected_deny=sum(case['expected']['decision']=='deny' for case in cases),
                  responses=sum(len(rows) for rows in records))
    if counts != lock['counts'] or counts != dict(cases=140, expected_allow=71, expected_deny=69, responses=280):
        raise ValueError('Frozen suite counts differ')
    for model, rows in zip(manifest['models'], records):
        if set(rows) != set(by_id):
            raise ValueError('Incomplete original result set')
        metrics = experiment.evaluate(cases, rows)
        expected = saved['results'][model]
        for key in ['total','valid','correct','false_allow','false_deny','uncertainty',
                    'label_correct','joint_labels_correct','accepted_label_correct','authorization_confusion']:
            if metrics[key] != expected[key]:
                raise ValueError('Archived aggregate differs for ' + model + '/' + key)
        print(model, str(metrics['correct'])+'/140', 'false_allow',metrics['false_allow'], 'false_deny',metrics['false_deny'])
    print('Verified',len(lock['files']),'file hashes, 140 fixed cases and 280 original responses')
    print('Suite',lock['suite'])
    print('Baseline',lock['baseline'])


if __name__ == '__main__':
    main()
