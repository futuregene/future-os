"""Offline verification of synthetic native-history trigger calibration."""
import argparse
import json
from pathlib import Path
import sqlite3

import history_trigger_calibration as c
b=c.b


def verify(root,prior):
    manifest=b.load(root/'manifest.json'); ledger=b.load(root/'ledger.json'); report=b.load(root/'report.json')
    assert b.sha(b.load(prior/'ledger.json'))==manifest['prior_ledger_sha256']
    assert b.load(prior/'verified-report.json')['total_spent_or_reserved']==manifest['opening_spend']
    for name,digest in manifest['code_hashes'].items(): assert b.sha((b.REPO/name).read_bytes())==digest,name
    for info in manifest['binaries'].values(): assert b.sha(Path(info['path']).read_bytes())==info['sha256']
    assert all(row['state']=='finished' for row in ledger.values())
    calls={row['identity']:(key,row) for key,row in ledger.items()}; seen=set(); dbs=0; query_count=0; search_count=0
    for job in manifest['jobs']:
        result=b.load(root/'results'/f'{job["id"]}.json')
        initial=b.load(root/'initial-messages'/f'{job["id"]}.json')
        assert result['initial_messages_sha256']==b.sha(initial)
        expected=c.FACTS[job['fact']]['value']
        if not job['visible'] and job['mode']!='recognition': assert expected not in json.dumps(initial,ensure_ascii=False)
        trace=b.load(root/'traces'/f'{job["id"]}.json') if (root/'traces'/f'{job["id"]}.json').exists() else []
        messages=list(initial); offset=0; recovered=False; final=None
        for turn in range(result['model_turns']):
            identity=job['id']+f'-turn{turn}'; key,row=calls[identity]; seen.add(identity)
            request=b.load(root/'requests'/f'{key}.json'); body=request['body']
            assert b.sha(request)==row['request_sha256']
            assert body['model']==manifest['model'] and body['max_tokens']==8192 and body['thinking']=={'type':'disabled'}
            assert body.get('tool_choice') in (None,'auto')
            if turn==0:
                assert body['messages']==initial
                assert ('tools' in body)==job['tools']
            assert body['messages']==messages
            payload=b.load(root/row['artifact'])
            if payload.get('calls'):
                messages.append({'role':'assistant','content':payload['text'] or None,'tool_calls':payload['calls']})
                for call in payload['calls']:
                    item=trace[offset]; offset+=1; fn=call['function']
                    assert item['name']==fn['name'] and item['arguments']==fn.get('arguments')
                    if item['status']!='budget_denied': query_count+=1
                    if item['status']=='ok':
                        native=json.loads(item['output']); args=json.loads(item['arguments'])
                        assert native['sessionId']=='trigger-'+job['id']
                        if item['name']=='history_search':
                            search_count+=1; assert native['query']==args['query']
                        else: assert native['entryId']==args['entry_id']
                        evidence=[x.get('snippet','') for x in native.get('matches',[])]+[x.get('text','') for x in native.get('chunks',[])]
                        recovered |= any(expected in text for text in evidence)
                    messages.append({'role':'tool','tool_call_id':call['id'],'content':item['output']})
            else:
                assert turn==result['model_turns']-1
                final=payload['text']
        assert offset==len(trace) and final==result['answer']
        assert sum(x['status']!='budget_denied' for x in trace)==result['logical_queries']
        if not job['absent']:
            assert recovered==result['hidden_value_retrieved']
            assert (expected in final)==result['expected_value_in_answer']
            assert (recovered and expected in final)==result['successful_auto_recall']
        proof=b.load(root/'database-proofs'/f'{job["id"]}.json')
        with sqlite3.connect(Path(proof['database']).as_uri()+'?mode=ro',uri=True) as db:
            state=db.execute('select status from legacy_imports where session_id=?',('trigger-'+job['id'],)).fetchone()
            count=db.execute("select count(*) from entries where session_id=? and entry_type in ('user','assistant','tool')",('trigger-'+job['id'],)).fetchone()[0]
        assert state==('imported',) and count==proof['expected']==len(manifest['fixture']); dbs+=1
    assert seen==set(calls) and len(manifest['jobs'])==12
    spend=sum(row.get('charged',row['reserved']) for row in ledger.values())
    assert abs(report['new_spend']-spend)<1e-9
    assert abs(report['total_spend']-(spend+manifest['opening_spend']))<1e-9
    assert report['total_spend']<=manifest['total_budget']<=300
    report.update(artifact_consistent=True,native_databases_verified=dbs,model_requests_verified=len(calls),
                  actual_tool_queries=query_count,actual_history_searches=search_count,forced_tool_choice=False,
                  limitations='short synthetic contexts; not a production reliability estimate; visible-answer control still performed redundant lookup')
    b.save(root/'verified-report.json',report)
    return {'verified':True,'cases':12,'model_requests':len(calls),'queries':query_count,'searches':search_count,'new_spend':spend,'total_spend':report['total_spend']}


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--output',type=Path,required=True); parser.add_argument('--prior',type=Path,required=True)
    args=parser.parse_args(); print(json.dumps(verify(args.output,args.prior),indent=2))
