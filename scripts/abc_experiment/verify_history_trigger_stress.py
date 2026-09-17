"""Offline verification for held-out length/policy/visibility calibration."""
import argparse
import json
from pathlib import Path
import sqlite3

import history_trigger_stress as s
b=s.b


def verify(root,prior):
    config=b.load(root/'manifest.json'); ledger=b.load(root/'ledger.json')
    assert b.sha(b.load(prior/'ledger.json'))==config['prior_ledger_sha256']
    assert b.load(prior/'verified-report.json')['total_spend']==config['opening_spend']
    assert b.load(prior/'manifest.json')['total_budget']==config['total_budget']
    for name,digest in config['code_hashes'].items(): assert b.sha((b.REPO/name).read_bytes())==digest,name
    for info in config['binaries'].values(): assert b.sha(Path(info['path']).read_bytes())==info['sha256']
    assert b.sha(b.load(root/'fixture.json'))==config['fixture_sha256']==b.sha(s.fixture())
    assert config['jobs']==s.jobs() and config['facts']==s.FACTS
    assert all(row['state']=='finished' for row in ledger.values())
    calls={row['identity']:(key,row) for key,row in ledger.items()}; seen=set(); queries=0; searches=0
    for job in config['jobs']:
        result=b.load(root/'results'/f'{job["id"]}.json'); initial=b.load(root/'initial-messages'/f'{job["id"]}.json')
        guide=config['native_guidance_template'].replace('"SESSION_ID"',json.dumps('stress-'+job['id']))
        assert initial==s.messages_for(job,guide) and b.sha(initial)==result['initial_messages_sha256']
        value=s.FACTS[job['fact']]['value']
        if not job['visible']: assert value not in json.dumps(initial,ensure_ascii=False)
        trace=b.load(root/'traces'/f'{job["id"]}.json') if (root/'traces'/f'{job["id"]}.json').exists() else []
        messages=list(initial); offset=0; found=False; final=None; tokens=[]
        for turn in range(result['model_turns']):
            identity=job['id']+f'-turn{turn}'; key,row=calls[identity]; seen.add(identity)
            request=b.load(root/'requests'/f'{key}.json'); body=request['body']
            assert b.sha(request)==row['request_sha256']
            assert body['model']==config['model'] and body['thinking']=={'type':'disabled'} and body['max_tokens']==8192
            assert body.get('tool_choice') in (None,'auto')
            assert body['messages']==messages
            if turn==0: assert body['tools']==s.e.t.FUTURE_TOOLS and body['tool_choice']=='auto'
            payload=b.load(root/row['artifact']); tokens.append((payload.get('usage') or {}).get('prompt_tokens'))
            if payload.get('calls'):
                messages.append({'role':'assistant','content':payload['text'] or None,'tool_calls':payload['calls']})
                for call in payload['calls']:
                    item=trace[offset]; offset+=1; fn=call['function']
                    assert item['name']==fn['name'] and item['arguments']==fn.get('arguments')
                    if item['status']!='budget_denied': queries+=1
                    if item['status']=='ok':
                        data=json.loads(item['output']); arguments=json.loads(item['arguments'])
                        assert data['sessionId']=='stress-'+job['id']
                        if item['name']=='history_search': searches+=1; assert data['query']==arguments['query']
                        else: assert data['entryId']==arguments['entry_id']
                        evidence=[x.get('snippet','') for x in data.get('matches',[])]+[x.get('text','') for x in data.get('chunks',[])]
                        found |= any(value in text for text in evidence)
                    messages.append({'role':'tool','tool_call_id':call['id'],'content':item['output']})
            else:
                assert turn==result['model_turns']-1
                final=payload['text']
        assert offset==len(trace) and tokens==result['actual_prompt_tokens'] and final==result['answer']
        assert sum(item['status']!='budget_denied' for item in trace)==result['logical_queries']<=8
        if not job['absent']:
            assert found==result['hidden_value_retrieved']
            assert (value in final)==result['expected_value_in_answer']
            assert (found and value in final)==result['successful_auto_recall']
        proof=b.load(root/'database-proofs'/f'{job["id"]}.json')
        with sqlite3.connect(Path(proof['database']).as_uri()+'?mode=ro',uri=True) as db:
            state=db.execute('select status from legacy_imports where session_id=?',('stress-'+job['id'],)).fetchone()
            count=db.execute("select count(*) from entries where session_id=? and entry_type in ('user','assistant','tool')",('stress-'+job['id'],)).fetchone()[0]
        assert state==('imported',) and count==proof['expected']==len(s.fixture())
    assert seen==set(calls) and len(config['jobs'])==26
    result=s.report(root)
    assert result['total_spend']<=config['total_budget']<=300
    result.update(artifact_consistent=True,model_requests_verified=len(calls),native_databases_verified=26,
                  queries_verified=queries,searches_verified=searches,forced_tool_choice=False,
                  scope='synthetic held-out facts, about 12K long-context tokens; descriptive cell counts, not production reliability')
    b.save(root/'verified-report.json',result)
    return result


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--output',type=Path,required=True); parser.add_argument('--prior',type=Path,required=True)
    args=parser.parse_args(); print(json.dumps(verify(args.output,args.prior),ensure_ascii=False,indent=2))
