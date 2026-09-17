"""Offline audit of the stronger prompt-only C3 experiment."""
import argparse
import json
from pathlib import Path
import sqlite3

import c3_archive_audit as x
b=x.b


def verify(root,prior,source):
    config=b.load(root/'manifest.json'); ledger=b.load(root/'ledger.json'); previous=b.load(prior/'manifest.json')
    assert b.sha(b.load(prior/'ledger.json'))==config['prior_ledger_sha256']
    assert b.sha(previous)==config['prior_manifest_sha256']
    assert b.load(prior/'verified-report.json')['total_spend']==config['opening_spend']
    for name,digest in config['code_hashes'].items(): assert b.sha((b.REPO/name).read_bytes())==digest
    for info in config['binaries'].values(): assert b.sha(Path(info['path']).read_bytes())==info['sha256']
    assert all(row['state']=='finished' for row in ledger.values())
    calls={row['identity']:(key,row) for key,row in ledger.items()}; seen=set(); pairs={}; query_count=0
    reference=Path(config['question_root'])
    for job in config['jobs']:
        old=next(row for row in previous['jobs'] if row['id']==job['prior_identity']); assert old['policy']
        original=b.load(prior/'initial-bodies'/f'{old["id"]}.json')
        initial=x.make_body(original,job['policy'],old['session_id'],job['session_id'])
        assert initial==b.load(root/'initial-bodies'/f'{job["id"]}.json') and b.sha(initial)==job['initial_body_sha256']
        assert initial['messages'][1:]==original['messages'][1:] and initial['tools']==original['tools']
        question=b.load(reference/'questions'/f'{job["chain"]}-{job["stage"]}.json')
        projection=b.load(source/'projections'/f'{job["chain"]}-{job["step"]}-C3-compact.json')['text']
        assert b.sha(question)==job['question_sha256'] and b.sha(projection)==job['projection_sha256']
        row=b.load(root/'results'/f'{job["id"]}.json')
        for key,value in job.items(): assert row[key]==value
        pairs.setdefault((job['chain'],job['stage']),{})[job['policy']]=row
        trace=b.load(root/'traces'/f'{job["id"]}.json') if (root/'traces'/f'{job["id"]}.json').exists() else []
        messages=list(initial['messages']); offset=0; used=0; size=0; answer=None
        for turn in range(row['model_turns']):
            identity=job['id']+f'-turn{turn}'; key,call=calls[identity]; seen.add(identity)
            request=b.load(root/'requests'/f'{key}.json'); assert b.sha(request)==call['request_sha256']
            expected=dict(initial)
            if used>=config['logical_query_limit'] or size>=config['byte_stop']:
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
                    assert item['bytes']==len(item['output'].encode()); size+=item['bytes']
                    if item['status']!='budget_denied': used+=1; query_count+=1
                    if item['status']=='ok':
                        data=json.loads(item['output']); args=json.loads(item['arguments'])
                        assert data['sessionId']==job['session_id']
                        if item['name']=='history_search': assert data['query']==args['query']
                        else: assert data['entryId']==args['entry_id']
                    messages.append({'role':'tool','tool_call_id':tool['id'],'content':item['output']})
            else:
                assert turn==row['model_turns']-1
                answer=b.parse_answer(payload['text'])
        assert offset==len(trace) and used==row['logical_queries'] and size==row['returned_bytes']
        assert answer==row['answer'] and bool(used)==row['lookup_used']
        score=b.exam.score(answer,dict.fromkeys(question['present']),dict.fromkeys(question['decoys']))
        assert all(row[key]==value for key,value in score.items())
        proof=b.load(root/'database-proofs'/f'{job["id"]}.json')
        with sqlite3.connect(Path(proof['database']).as_uri()+'?mode=ro',uri=True) as db:
            status=db.execute('select status from legacy_imports where session_id=?',(job['session_id'],)).fetchone()
            count=db.execute("select count(*) from entries where session_id=? and entry_type in ('user','assistant','tool')",(job['session_id'],)).fetchone()[0]
        assert status==('imported',) and count==proof['expected']
    assert len(config['jobs'])==36 and len(pairs)==18 and seen==set(calls)
    result=x.c.report(root); assert result['complete'] and result['total_spend']<=config['total_budget']<=300
    result.update(artifact_consistent=True,model_requests_verified=len(calls),native_databases_verified=36,
        queries_verified=query_count,forced_tool_choice=False,pairs=[{'chain':chain,'stage':stage,
        'control_queries':pair[False]['logical_queries'],'strong_queries':pair[True]['logical_queries'],
        'control_hits':pair[False]['hits'],'strong_hits':pair[True]['hits'],
        'control_fp':pair[False]['false_positives'],'strong_fp':pair[True]['false_positives']}
        for (chain,stage),pair in sorted(pairs.items())])
    b.save(root/'verified-report.json',result)
    return result


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for key in ('output','prior','source'): parser.add_argument('--'+key,type=Path,required=True)
    args=parser.parse_args(); print(json.dumps(verify(args.output,args.prior,args.source),ensure_ascii=False,indent=2))
