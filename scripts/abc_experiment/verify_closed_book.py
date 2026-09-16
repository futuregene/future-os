#!/usr/bin/env python3
"""Recompute closed-book results and check frozen chain/call provenance offline."""
import argparse
from collections import Counter
import json
from pathlib import Path
import statistics

import four_arm_rerun as run


def verify(root):
    config = run.load(root/'manifest.json')
    ledger = run.load(root/'ledger.json')
    receipt = run.load(root/'resume-receipt.json')
    amendment = run.load(root/'budget-amendment-001.json')
    assert run.load(root/'report.json')['complete'], 'closed book incomplete'
    assert receipt['primary_complete'], 'controller has not completed primary phase'
    assert all(r['state'] == 'finished' for r in ledger.values()), 'unfinished call'
    assert not any('-open-turn' in r['identity'] for r in ledger.values()), 'open book already called'
    original = dict(list(ledger.items())[:receipt['original_operations']])
    assert run.sha(original) == receipt['original_ledger_sha256'], 'old ledger changed'
    assert run.sha(config) == receipt['manifest_sha256']
    assert run.sha(amendment) == receipt['authorization_sha256']
    for path, expected in config['code_hashes'].items():
        assert run.sha((run.REPO/path).read_bytes()) == expected, path
    calls = {r['identity']:r for r in ledger.values()}
    assert len(calls) == len(ledger), 'duplicate operation identity'
    schedule = run.load(root/'schedule.json')
    rows, containment = [], []
    algorithms = Counter()
    for chain in config['chains']:
        data = run.load(root/'corpus'/f'{chain}.json')
        assert run.sha(data) == config['input_hashes'][chain]
        assert run.schedule(data['records'], data['cuts']) == schedule[chain]
        prior = {arm:None for arm in run.ARMS}
        for step, end in enumerate(schedule[chain]):
            states = {}
            start = schedule[chain][step-1] if step else 0
            fresh = data['records'][start:end]
            input_hash = run.sha(data['records'][:end])
            for arm in run.ARMS:
                identity = f'{chain}-{step}-{arm}-compact'
                p = run.load(root/'projections'/f'{identity}.json')
                assert p['input_sha256'] == input_hash
                assert p['previous_sha256'] == run.sha(prior[arm])
                assert (p['chain'],p['end'],p['arm']) == (chain,end,arm)
                response = run.load(root/calls[identity]['artifact'])
                if arm in ('C3','C'):
                    assert response['text'] == p['text']
                    if arm == 'C3':
                        algorithms[(p.get('checkpoint') or {}).get('algorithm_version','unchanged')] += 1
                elif arm == 'codex':
                    live = (prior[arm]['live'] if prior[arm] else []) + fresh
                    assert p['summary'] == response['text'].strip()
                    assert p['text'] == run.ext.codex_build(live, p['summary'])
                    assert p['live'] == run.ext.codex_compacted_records(live, p['summary'])
                else:
                    live = (prior[arm]['live'] if prior[arm] else []) + run.ext.opencode_entries(fresh)
                    if p.get('declined'):
                        assert not response['text'].strip()
                        assert p['live'] == live
                    else:
                        history = run.ext.opencode_visible(live)
                        tail = run.ext.opencode_select(history, run.ext.opencode_preserve_budget(config['window']))
                        tail = tail or len(history)
                        assert p['summary'] == response['text'].strip()
                        assert p['text'] == run.ext.opencode_build(history, tail, p['summary'])
                        assert p['live'] == run.ext.opencode_compacted_entries(history, tail, p['summary'])
                states[arm] = p
            prior = states
            if end not in data['cuts']:
                continue
            stage = data['cuts'].index(end)
            q = run.load(root/'questions'/f'{chain}-{stage}.json')
            assert q == run.questionnaire(data['records'][:end], stage)
            c, c3 = states['C']['text'], states['C3']['text']
            containment.append({'chain':chain, 'stage':stage,
                'C_missing':sum(v not in c for v in q['present']),
                'rescued_by_C3':sum(v not in c and v in c3 for v in q['present']),
                'C_only':sum(v in c and v not in c3 for v in q['present'])})
            for arm in run.ARMS:
                identity = f'{chain}-{stage}-{arm}-closed'
                row = run.load(root/'scores'/f'{identity}.json')
                response = run.load(root/calls[identity]['artifact'])
                answer = run.parse_answer(response['text'])
                expected = run.exam.score(answer, dict.fromkeys(q['present']), dict.fromkeys(q['decoys']))
                assert all(row[k] == v for k,v in expected.items())
                assert row['answer'] == answer
                assert row['projection_sha256'] == run.sha(states[arm]['text'])
                assert row['question_sha256'] == run.sha(q)
                assert row['contained'] == sum(v in states[arm]['text'] for v in q['present'])
                assert row['projection_estimated_tokens'] == run.tokens(states[arm]['text'])
                rows.append(row)
    expected_cases = {(c,s,a) for c in config['chains'] for s in range(3) for a in run.ARMS}
    assert {(r['chain'],r['stage'],r['arm']) for r in rows} == expected_cases
    assert len(list((root/'scores').glob('*.json'))) == len(rows) == 72
    spend = sum(r.get('charged',r['reserved']) for r in ledger.values())
    assert spend <= amendment['authorized_total_budget']
    result = {'verified':True,'closed_cases':72,'open_model_calls':0,
              'original_operations_unchanged':receipt['original_operations'],
              'ledger_operations':len(ledger),'spent_or_reserved':spend,
              'C3_checkpoint_algorithms':dict(algorithms),'arms':{},'by_chain':{},
              'fixed_target_containment':containment}
    for arm in run.ARMS:
        xs = [r for r in rows if r['arm']==arm]
        result['arms'][arm] = {k:sum(r[k] for r in xs) for k in ('hits','of_present','false_positives','contained')}
        result['arms'][arm]['invalid'] = sum(not r['valid_answer'] for r in xs)
        result['arms'][arm]['median_estimated_tokens'] = statistics.median(r['projection_estimated_tokens'] for r in xs)
        for chain in config['chains']:
            ys = [r for r in xs if r['chain']==chain]
            result['by_chain'].setdefault(chain,{})[arm] = {k:sum(r[k] for r in ys) for k in ('hits','of_present','false_positives')}
    run.save(root/'verified-closed-report.json',result)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output',type=Path,required=True)
    args = parser.parse_args()
    print(json.dumps(verify(args.output.resolve()),indent=2))
