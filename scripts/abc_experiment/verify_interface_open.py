"""Offline verification of the interface-level condition. No model calls."""
import argparse
import json
from pathlib import Path
import sqlite3
import tempfile

import interface_open_exam as e
b=e.b; t=e.t


def verify(root,closed,prior):
    config=b.load(root/'manifest.json'); ledger=b.load(root/'ledger.json')
    assert b.sha(b.load(prior/'ledger.json'))==config['prior_ledger_sha256']
    assert b.sha(b.load(closed/'fidelity-manifest.json'))==config['closed_manifest_sha256']
    for name,digest in config['code_hashes'].items(): assert b.sha((b.REPO/name).read_bytes())==digest,name
    assert all(row['state']=='finished' for row in ledger.values())
    calls={row['identity']:row for row in ledger.values()}; assert len(calls)==len(ledger)
    schedule=b.load(closed/'schedule.json'); seen=set(); dbs=0; replayed=0
    with tempfile.TemporaryDirectory(prefix='verify-files-',dir=root) as temporary:
        for chain in config['chains']:
            data=b.load(closed/'corpus'/f'{chain}.json')
            for stage,cut in enumerate(data['cuts']):
                records=data['records'][:cut]; q=b.load(closed/'questions'/f'{chain}-{stage}.json')
                codex=t.CodexHistory(records,[x for x in schedule[chain] if x<=cut])
                files=t.ObservedFiles(records,Path(temporary)/f'{chain}-{stage}')
                for arm in b.ARMS:
                    identity=f'{chain}-{stage}-{arm}-interfaces'; row=b.load(root/'scores'/f'{identity}.json')
                    assert (row['chain'],row['stage'],row['arm'])==(chain,stage,arm)
                    seen.add((chain,stage,arm))
                    original=b.load(closed/'scores'/f'{chain}-{stage}-{arm}-closed.json')
                    assert row['projection_sha256']==original['projection_sha256'] and row['question_sha256']==b.sha(q)
                    payload=b.load(root/calls[identity+f'-turn{row["model_turns"]-1}']['artifact'])
                    answer=b.parse_answer(payload['text']); assert answer==row['answer']
                    score=b.exam.score(answer,dict.fromkeys(q['present']),dict.fromkeys(q['decoys']))
                    assert all(row[k]==v for k,v in score.items())
                    trace=b.load(root/'traces'/f'{identity}.json') if (root/'traces'/f'{identity}.json').exists() else []
                    assert sum(x['bytes'] for x in trace)==row['returned_bytes']
                    assert all(x['bytes']==len(x['output'].encode()) for x in trace)
                    assert sum(x['status']!='budget_denied' for x in trace)==row['logical_queries']
                    projection=b.load(closed/'projections'/f'{chain}-{schedule[chain].index(cut)}-{arm}-compact.json')['text']
                    lookup=([t.body(x) for x in records] if arm in ('C','C3') else
                            [x['content'] for x in codex.flat] if arm=='codex' else list(files.files)+list(files.files.values()))
                    available={v for v in q['present'] if v in projection or any(v in text for text in lookup)}
                    assert len(available)==row['reachable']
                    assert len(available & set(answer['appeared']))==row['reachable_hits']
                    if arm in ('C','C3'):
                        proof=b.load(root/'database-proofs'/f'{identity}.json')
                        with sqlite3.connect(Path(proof['database']).as_uri()+'?mode=ro',uri=True) as db:
                            status=db.execute('select status from legacy_imports where session_id=?',('interface-'+identity,)).fetchone()
                            count=db.execute("select count(*) from entries where session_id=? and entry_type in ('user','assistant','tool')",('interface-'+identity,)).fetchone()[0]
                        assert status==('imported',) and count==proof['expected']; dbs+=1
                    else:
                        dispatcher=codex.call if arm=='codex' else files.call
                        for item in trace:
                            if item['status']=='ok':
                                assert dispatcher(item['name'],json.loads(item['arguments']))==item['output']
                                replayed+=1
    expected={(c,s,a) for c in config['chains'] for s in range(3) for a in b.ARMS}
    assert seen==expected and len(list((root/'scores').glob('*.json')))==len(expected)
    result=e.report(root)
    result.update(artifact_consistent=True,database_cases_verified=dbs,local_tool_calls_replayed=replayed,
                  interpretation='Tool availability is not tool use. No-lookup cases do not measure retrieval quality; replicas and file coverage are explicit limitations.')
    assert result['total_spent_or_reserved']<=config['budget']
    b.save(root/'verified-report.json',result)
    return result


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for name in ('output','closed','prior'): parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args()
    print(json.dumps(verify(args.output,args.closed,args.prior),indent=2))
