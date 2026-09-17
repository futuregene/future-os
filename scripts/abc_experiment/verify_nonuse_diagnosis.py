"""Offline verification of the real-context prompt diagnostic."""
import argparse
import json
from pathlib import Path
import sqlite3

import nonuse_diagnosis as d
b=d.b


def verify(root,prior,closed):
    config=b.load(root/'manifest.json'); ledger=b.load(root/'ledger.json'); previous=b.load(prior/'manifest.json')
    assert b.sha(b.load(prior/'ledger.json'))==config['prior_ledger_sha256']
    assert b.sha(previous)==config['prior_manifest_sha256']
    assert b.load(prior/'verified-report.json')['total_spent_or_reserved']==config['opening_spend']
    for name,digest in config['code_hashes'].items(): assert b.sha((b.REPO/name).read_bytes())==digest
    for info in config['binaries'].values(): assert b.sha(Path(info['path']).read_bytes())==info['sha256']
    assert all(row['state']=='finished' for row in ledger.values())
    calls={row['identity']:(key,row) for key,row in ledger.items()}; seen=set(); dbs=0; queries=0; errors=0
    for job in config['jobs']:
        source=next(x for x in previous['jobs'] if x['chain']==job['chain'] and x['stage']==0 and x['arm']=='C3')
        original=b.load(prior/'initial-open-bodies'/f'{source["identity"]}.json')
        projection=b.load(closed/'projections'/f'{job["chain"]}-{source["step"]}-C3-compact.json')['text']
        question=b.load(closed/'questions'/f'{job["chain"]}-0.json')['prompt']
        initial,target=d.make_body(original,projection,question,job['allow_history'],job['framing'],source['session_id'],job['session_id'])
        assert initial==b.load(root/'initial-bodies'/f'{job["id"]}.json') and target==job['target']
        result=b.load(root/'results'/f'{job["id"]}.json')
        for key,value in job.items(): assert result[key]==value
        assert b.sha(initial)==result['initial_body_sha256']
        trace=b.load(root/'traces'/f'{job["id"]}.json') if (root/'traces'/f'{job["id"]}.json').exists() else []
        messages=list(initial['messages']); offset=0; used=0; final=None
        for turn in range(result['model_turns']):
            identity=job['id']+f'-turn{turn}'; key,row=calls[identity]; seen.add(identity)
            request=b.load(root/'requests'/f'{key}.json'); body=request['body']; assert b.sha(request)==row['request_sha256']
            expected=dict(initial,messages=messages)
            if used>=8: expected.pop('tools'); expected.pop('tool_choice')
            assert body==expected and body.get('tool_choice') in (None,'auto')
            payload=b.load(root/row['artifact'])
            if payload.get('calls'):
                messages.append({'role':'assistant','content':payload['text'] or None,'tool_calls':payload['calls']})
                for call in payload['calls']:
                    item=trace[offset]; offset+=1; fn=call['function']
                    assert item['name']==fn['name'] and item['arguments']==fn.get('arguments')
                    if item['status']!='budget_denied': used+=1; queries+=1
                    if item['status']=='tool_error': errors+=1
                    if item['status']=='ok':
                        native=json.loads(item['output']); args=json.loads(item['arguments'])
                        assert native['sessionId']==job['session_id']
                        if item['name']=='history_search': assert native['query']==args['query']
                        else: assert native['entryId']==args['entry_id']
                    messages.append({'role':'tool','tool_call_id':call['id'],'content':item['output']})
            else:
                assert turn==result['model_turns']-1
                final=payload['text']
        assert offset==len(trace) and used==result['logical_queries']<=8 and final==result['answer_text']
        proof=b.load(root/'database-proofs'/f'{job["id"]}.json')
        with sqlite3.connect(Path(proof['database']).as_uri()+'?mode=ro',uri=True) as db:
            state=db.execute('select status from legacy_imports where session_id=?',(job['session_id'],)).fetchone()
            count=db.execute("select count(*) from entries where session_id=? and entry_type in ('user','assistant','tool')",(job['session_id'],)).fetchone()[0]
        assert state==('imported',) and count==proof['expected']; dbs+=1
    assert len(config['jobs'])==18 and seen==set(calls)
    out=d.report(root); assert out['complete'] and out['total_spend']<=config['total_budget']<=300
    out.update(artifact_consistent=True,native_databases_verified=dbs,model_requests_verified=len(calls),
               actual_queries=queries,tool_errors=errors,forced_tool_choice=False,
               interpretation='controlled task-framing diagnostic on three real-session first boundaries, single draw per cell; not a new main score round')
    b.save(root/'verified-report.json',out)
    return out


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for key in ('output','prior','closed'): parser.add_argument('--'+key,type=Path,required=True)
    args=parser.parse_args(); print(json.dumps(verify(args.output,args.prior,args.closed),ensure_ascii=False,indent=2))
