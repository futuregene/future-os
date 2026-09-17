"""Offline audit of C3-only fresh control/policy optimization."""
import argparse
import json
from pathlib import Path
import sqlite3

import c3_query_priority as c
b=c.b


def verify(root,prior,source):
    config=b.load(root/'manifest.json'); ledger=b.load(root/'ledger.json'); original_manifest=b.load(prior/'manifest.json')
    assert b.sha(b.load(prior/'ledger.json'))==config['prior_ledger_sha256']
    assert b.sha(original_manifest)==config['prior_manifest_sha256']
    assert b.load(prior/'verified-report.json')['total_spend']==config['opening_spend']
    for name,digest in config['code_hashes'].items(): assert b.sha((b.REPO/name).read_bytes())==digest
    for info in config['binaries'].values(): assert b.sha(Path(info['path']).read_bytes())==info['sha256']
    assert all(row['state']=='finished' for row in ledger.values())
    calls={row['identity']:(key,row) for key,row in ledger.items()}; seen=set(); queries=0; paired={}
    for job in config['jobs']:
        original=next(x for x in original_manifest['jobs'] if x['identity']==job['source_identity'])
        initial_old=b.load(prior/'initial-bodies'/f'{original["identity"]}.json')
        initial=c.make_body(initial_old,job['policy'],original['session_id'],job['session_id'])
        assert initial==b.load(root/'initial-bodies'/f'{job["id"]}.json') and b.sha(initial)==job['initial_body_sha256']
        assert initial['messages'][1:]==initial_old['messages'][1:]
        assert initial['tools']==c.e.t.FUTURE_TOOLS and initial['tool_choice']=='auto'
        projection=b.load(source/'projections'/f'{job["chain"]}-{job["step"]}-C3-compact.json')['text']
        question=b.load(prior/'questions'/f'{job["chain"]}-{job["stage"]}.json')
        assert b.sha(projection)==job['projection_sha256'] and b.sha(question)==job['question_sha256']
        row=b.load(root/'results'/f'{job["id"]}.json'); paired.setdefault((job['chain'],job['stage']),{})[job['policy']]=row
        for key,value in job.items(): assert row[key]==value
        trace=b.load(root/'traces'/f'{job["id"]}.json') if (root/'traces'/f'{job["id"]}.json').exists() else []
        messages=list(initial['messages']); offset=0; used=0; total_bytes=0; answer=None
        for turn in range(row['model_turns']):
            identity=job['id']+f'-turn{turn}'; key,call=calls[identity]; seen.add(identity)
            request=b.load(root/'requests'/f'{key}.json'); assert b.sha(request)==call['request_sha256']
            expected=dict(initial)
            if used>=config['logical_query_limit'] or total_bytes>=config['byte_stop']:
                expected.pop('tools'); expected.pop('tool_choice')
                messages.append({'role':'user','content':'The lookup allowance is exhausted. Answer the original question using the evidence already available.'})
            expected['messages']=messages
            assert request['body']==expected and expected.get('tool_choice') in (None,'auto')
            payload=b.load(root/call['artifact'])
            if payload.get('calls'):
                messages.append({'role':'assistant','content':payload['text'] or None,'tool_calls':payload['calls']})
                for tool in payload['calls']:
                    item=trace[offset]; offset+=1; fn=tool['function']
                    assert item['name']==fn['name'] and item['arguments']==fn.get('arguments')
                    assert item['bytes']==len(item['output'].encode()); total_bytes+=item['bytes']
                    if item['status']!='budget_denied': used+=1; queries+=1
                    if item['status']=='ok':
                        data=json.loads(item['output']); args=json.loads(item['arguments'])
                        assert data['sessionId']==job['session_id']
                        if item['name']=='history_search': assert data['query']==args['query']
                        else: assert data['entryId']==args['entry_id']
                    messages.append({'role':'tool','tool_call_id':tool['id'],'content':item['output']})
            else:
                assert turn==row['model_turns']-1
                answer=b.parse_answer(payload['text'])
        assert offset==len(trace) and used==row['logical_queries'] and total_bytes==row['returned_bytes']
        assert answer==row['answer'] and bool(used)==row['lookup_used']
        score=b.exam.score(answer,dict.fromkeys(question['present']),dict.fromkeys(question['decoys']))
        assert all(row[key]==value for key,value in score.items())
        proof=b.load(root/'database-proofs'/f'{job["id"]}.json')
        with sqlite3.connect(Path(proof['database']).as_uri()+'?mode=ro',uri=True) as db:
            status=db.execute('select status from legacy_imports where session_id=?',(job['session_id'],)).fetchone()
            count=db.execute("select count(*) from entries where session_id=? and entry_type in ('user','assistant','tool')",(job['session_id'],)).fetchone()[0]
        assert status==('imported',) and count==proof['expected']
    assert seen==set(calls) and len(config['jobs'])==36 and len(paired)==18
    result=c.report(root); assert result['complete'] and result['total_spend']<=config['total_budget']<=300
    result.update(artifact_consistent=True,model_requests_verified=len(calls),native_databases_verified=36,queries_verified=queries,
                  forced_tool_choice=False,pairs=[{'chain':chain,'stage':stage,'control_queries':pair[False]['logical_queries'],
                  'policy_queries':pair[True]['logical_queries'],'control_hits':pair[False]['hits'],'policy_hits':pair[True]['hits'],
                  'control_false_positives':pair[False]['false_positives'],'policy_false_positives':pair[True]['false_positives']}
                  for (chain,stage),pair in sorted(paired.items())])
    b.save(root/'verified-report.json',result)
    return result


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for key in ('output','prior','source'): parser.add_argument('--'+key,type=Path,required=True)
    args=parser.parse_args(); print(json.dumps(verify(args.output,args.prior,args.source),ensure_ascii=False,indent=2))
