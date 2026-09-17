"""Offline verification of complete-literal-recall prompt iterations."""
import argparse
import json
from pathlib import Path
import sqlite3

import c3_verbatim_recall as x
b=x.b


def verify(root,prior,reference,source):
    m=b.load(root/'manifest.json'); ledger=b.load(root/'ledger.json'); ref=b.load(reference/'manifest.json')
    assert b.sha(b.load(prior/'ledger.json'))==m['prior_ledger_sha256']
    assert b.load(prior/'verified-report.json')['total_spend']==m['opening_spend']
    assert b.sha(ref)==m['reference_manifest_sha256']
    for name,digest in m['code_hashes'].items(): assert b.sha((b.REPO/name).read_bytes())==digest,name
    for info in m['binaries'].values(): assert b.sha(Path(info['path']).read_bytes())==info['sha256']
    assert all(row['state']=='finished' for row in ledger.values())
    calls={row['identity']:(key,row) for key,row in ledger.items()}; seen=set(); queries=0; missing={}; extra={}
    returned_evidence_hits=0; projection_only={}
    for job in m['jobs']:
        old=next(row for row in ref['jobs'] if row['identity']==job['reference_identity'])
        original=b.load(reference/'initial-bodies'/f'{old["identity"]}.json')
        initial=x.make_body(original,old['session_id'],job['session_id'],m['variant'])
        assert initial==b.load(root/'initial-bodies'/f'{job["id"]}.json') and b.sha(initial)==job['initial_body_sha256']
        assert initial['messages'][1]==original['messages'][1]
        assert not any(message['role']=='assistant' for message in initial['messages'])
        question=b.load(reference/'questions'/f'{job["chain"]}-{job["stage"]}.json')
        projection=b.load(source/'projections'/f'{job["chain"]}-{job["step"]}-C3-compact.json')['text']
        assert b.sha(question)==job['question_sha256'] and b.sha(projection)==job['projection_sha256']
        result=b.load(root/'results'/f'{job["id"]}.json')
        for key,value in job.items(): assert result[key]==value
        trace=b.load(root/'traces'/f'{job["id"]}.json') if (root/'traces'/f'{job["id"]}.json').exists() else []
        messages=list(initial['messages']); offset=0; used=0; size=0; answer=None; evidence_texts=[]
        for turn in range(result['model_turns']):
            identity=job['id']+f'-turn{turn}'; key,row=calls[identity]; seen.add(identity)
            request=b.load(root/'requests'/f'{key}.json'); assert b.sha(request)==row['request_sha256']
            expected=dict(initial)
            if used>=m['logical_query_limit'] or size>=m['byte_stop']:
                expected.pop('tools'); expected.pop('tool_choice')
                messages.append({'role':'user','content':'The lookup allowance is exhausted. Return the requested JSON using established original evidence; do not guess.'})
            expected['messages']=messages
            assert request['body']==expected and expected.get('tool_choice') in (None,'auto')
            payload=b.load(root/row['artifact'])
            if payload.get('calls'):
                messages.append({'role':'assistant','content':payload['text'] or None,'tool_calls':payload['calls']})
                for call in payload['calls']:
                    item=trace[offset]; offset+=1; fn=call['function']
                    assert item['name']==fn['name'] and item['arguments']==fn.get('arguments')
                    assert item['bytes']==len(item['output'].encode()); size+=item['bytes']
                    if item['status']!='budget_denied': used+=1; queries+=1
                    if item['status']=='ok':
                        data=json.loads(item['output']); args=json.loads(item['arguments'])
                        assert data['sessionId']==job['session_id']
                        if item['name']=='history_search':
                            assert data['query']==args['query']
                            evidence_texts.extend(match.get('snippet','') for match in data.get('matches',[]))
                        else:
                            assert data['entryId']==args['entry_id']
                            evidence_texts.extend(chunk.get('text','') for chunk in data.get('chunks',[]))
                            if isinstance(data.get('text'),str): evidence_texts.append(data['text'])
                    messages.append({'role':'tool','tool_call_id':call['id'],'content':item['output']})
            else:
                assert turn==result['model_turns']-1
                answer=b.parse_answer(payload['text'])
        assert offset==len(trace) and used==result['logical_queries'] and size==result['returned_bytes']
        assert answer==result['answer'] and bool(used)==result['lookup_used']
        score=b.exam.score(answer,dict.fromkeys(question['present']),dict.fromkeys(question['decoys']))
        assert all(result[key]==value for key,value in score.items())
        reported=set((answer or {}).get('appeared',[])); lost=set(question['present'])-reported; false=reported&set(question['decoys'])
        if lost: missing[job['id']]=sorted(lost)
        if false: extra[job['id']]=sorted(false)
        from_tools={value for value in reported if any(value in text for text in evidence_texts)}
        returned_evidence_hits+=len(from_tools)
        only_projection=reported-from_tools
        assert all(value in projection for value in only_projection),'final value lacks literal visible/returned evidence'
        if only_projection: projection_only[job['id']]=sorted(only_projection)
        proof=b.load(root/'database-proofs'/f'{job["id"]}.json')
        with sqlite3.connect(Path(proof['database']).as_uri()+'?mode=ro',uri=True) as db:
            status=db.execute('select status from legacy_imports where session_id=?',(job['session_id'],)).fetchone()
            count=db.execute("select count(*) from entries where session_id=? and entry_type in ('user','assistant','tool')",(job['session_id'],)).fetchone()[0]
        assert status==('imported',) and count==proof['expected']
    assert len(m['jobs'])==18 and seen==set(calls)
    report=x.report(root); assert report['complete'] and report['total_spend']<=m['total_budget']<=300
    report.update(artifact_consistent=True,model_requests_verified=len(calls),native_databases_verified=18,
                  queries_verified=queries,forced_tool_choice=False,missed_values=missing,false_positive_values=extra,
                  returned_original_text_supported_items=returned_evidence_hits,projection_only_supported_items=projection_only,
                  target_verified=report['target_met_observed'] and not missing and not extra,
                  scope='prompt-directed literal archive audit on tuning cohort; no guarantee of production/generalization reliability')
    b.save(root/'verified-report.json',report)
    return report


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for key in ('output','prior','reference','source'): parser.add_argument('--'+key,type=Path,required=True)
    args=parser.parse_args(); print(json.dumps(verify(args.output,args.prior,args.reference,args.source),ensure_ascii=False,indent=2))
