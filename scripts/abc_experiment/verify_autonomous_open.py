"""Offline audit of standard autonomous open book; no new model calls."""
import argparse
import json
from pathlib import Path
import sqlite3
import tempfile

import autonomous_open_exam as a
b=a.b; t=a.t


def verify(root,closed,prior):
    config=b.load(root/'manifest.json'); ledger=b.load(root/'ledger.json')
    assert b.sha(b.load(closed/'fidelity-manifest.json'))==config['closed_manifest_sha256']
    assert b.sha(b.load(closed/'ledger.json'))==config['closed_ledger_sha256']
    assert b.sha(b.load(prior/'ledger.json'))==config['prior_ledger_sha256']
    assert b.load(prior/'verified-report.json')['total_spent_or_reserved']==config['opening_spend']
    for name,digest in config['code_hashes'].items(): assert b.sha((b.REPO/name).read_bytes())==digest,name
    for info in config['binaries'].values(): assert b.sha(Path(info['path']).read_bytes())==info['sha256']
    assert all(row['state']=='finished' for row in ledger.values())
    calls={row['identity']:(key,row) for key,row in ledger.items()}; assert len(calls)==len(ledger)
    schedule=b.load(closed/'schedule.json'); seen=set(); dbs=0; replayed=0
    with tempfile.TemporaryDirectory(prefix='verify-files-',dir=root) as temporary:
        for job in config['jobs']:
            identity=job['identity']; chain=job['chain']; stage=job['stage']; arm=job['arm']
            row=b.load(root/'scores'/f'{identity}.json')
            request=b.load(closed/'requests'/f'{job["closed_request_key"]}.json')
            assert request==b.load(root/'closed-requests'/f'{identity}.json') and b.sha(request)==job['closed_request_sha256']
            baseline=b.load(closed/'scores'/f'{chain}-{stage}-{arm}-closed.json')
            projection=b.load(closed/'projections'/f'{chain}-{job["step"]}-{arm}-compact.json')['text']
            question=b.load(closed/'questions'/f'{chain}-{stage}.json')
            assert b.sha(projection)==job['projection_sha256']==baseline['projection_sha256']
            assert b.sha(question)==job['question_sha256']==baseline['question_sha256']
            assert b.sha(baseline)==row['baseline_sha256']
            for key,value in job.items(): assert row[key]==value
            data=b.load(closed/'corpus'/f'{chain}.json'); records=data['records'][:job['cut']]
            if arm=='codex':
                backend=t.CodexHistory(records,[end for end in schedule[chain] if end<=job['cut']])
                lookup=[x['content'] for x in backend.flat]
            elif arm=='opencode':
                backend=t.ObservedFiles(records,Path(temporary)/identity)
                visibility=b.load(root/'file-visibility'/f'{identity}.json')
                assert visibility['content_sha256']==b.sha(backend.files)
                lookup=list(backend.files)+list(backend.files.values())
            else:
                lookup=[t.body(x) for x in records]
                proof=b.load(root/'database-proofs'/f'{identity}.json')
                with sqlite3.connect(Path(proof['database']).as_uri()+'?mode=ro',uri=True) as db:
                    state=db.execute('select status from legacy_imports where session_id=?',('autonomous-'+identity,)).fetchone()
                    count=db.execute("select count(*) from entries where session_id=? and entry_type in ('user','assistant','tool')",('autonomous-'+identity,)).fetchone()[0]
                assert state==('imported',) and count==proof['expected']; dbs+=1
            trace=b.load(root/'traces'/f'{identity}.json') if (root/'traces'/f'{identity}.json').exists() else []
            offset=0; queries=0; total=0; messages=list(request['body']['messages']); final=None
            for turn in range(row['model_turns']):
                call_id=f'{identity}-turn{turn}'; key,entry=calls[call_id]; seen.add(call_id)
                actual=b.load(root/'requests'/f'{key}.json'); body=actual['body']
                assert b.sha(actual)==entry['request_sha256']
                assert body.get('tool_choice') in (None,'auto')
                if turn==0:
                    a.assert_initial_parity(request['body'],body,a.schemas(arm))
                    assert b.sha(body)==job['initial_open_body_sha256']
                expected=a.open_body(request['body'],a.schemas(arm))
                if queries>=config['logical_query_limit'] or total>=config['byte_stop']:
                    expected.pop('tools'); expected.pop('tool_choice')
                    messages.append({'role':'user','content':'The lookup allowance is exhausted. Answer the original question using the evidence already available.'})
                expected['messages']=messages
                assert body==expected,'unexpected message/control mutation'
                payload=b.load(root/entry['artifact'])
                if payload.get('calls'):
                    messages.append({'role':'assistant','content':payload['text'] or None,'tool_calls':payload['calls']})
                    for call in payload['calls']:
                        item=trace[offset]; offset+=1; fn=call['function']
                        assert item['name']==fn['name'] and item['arguments']==fn.get('arguments')
                        assert item['bytes']==len(item['output'].encode()); total+=item['bytes']
                        if item['status']!='budget_denied': queries+=1
                        if item['status']=='ok':
                            args=json.loads(item['arguments'])
                            if arm in ('C','C3'):
                                native=json.loads(item['output']); assert native['sessionId']=='autonomous-'+identity
                                if item['name']=='history_search': assert native['query']==args['query']
                                else: assert native['entryId']==args['entry_id']
                            else:
                                assert backend.call(item['name'],args)==item['output']; replayed+=1
                        messages.append({'role':'tool','tool_call_id':call['id'],'content':item['output']})
                else:
                    assert turn==row['model_turns']-1,'no-tool answer was unnecessarily retried'
                    final=b.parse_answer(payload['text'])
            assert offset==len(trace) and queries==row['logical_queries'] and total==row['returned_bytes']
            assert final==row['answer'] and bool(queries)==row['lookup_used']
            computed=a.evaluate({'answer':final},question,baseline,projection,lookup)
            assert all(row[key]==value for key,value in computed.items())
    assert seen==set(calls) and len(config['jobs'])==72 and len(list((root/'scores').glob('*.json')))==72
    result=a.report(root)
    assert result['complete'] and result['total_spent_or_reserved']<=config['budget']
    result.update(artifact_consistent=True,initial_request_parity_cases=72,model_requests_verified=len(calls),
        native_databases_verified=dbs,local_tool_calls_replayed=replayed,prior_answer_injected=False,
        pending_list_injected=False,forced_tool_choice=False,
        interpretation='standard autonomous tool-availability comparison with frozen lexical scoring and limited local replicas; no-use is valid')
    b.save(root/'verified-report.json',result)
    return result


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for name in ('output','closed','prior'): parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args()
    print(json.dumps(verify(args.output,args.closed,args.prior),ensure_ascii=False,indent=2))
