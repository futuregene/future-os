"""Offline paired verification: fixed baseline, actual tool flow and deltas."""
import argparse
import json
from pathlib import Path
import sqlite3
import tempfile

import paired_retrieval_exam as p
b=p.b; t=p.t


def verify(root,closed,prior,superseded):
    config=b.load(root/'manifest.json'); ledger=b.load(root/'ledger.json')
    assert b.sha(b.load(prior/'ledger.json'))==config['prior_ledger_sha256']
    assert b.sha(b.load(superseded/'ledger.json'))==config['superseded_attempt_ledger_sha256']
    assert b.sha(b.load(closed/'fidelity-manifest.json'))==config['closed_manifest_sha256']
    for name,digest in config['code_hashes'].items(): assert b.sha((b.REPO/name).read_bytes())==digest,name
    assert all(row['state']=='finished' for row in ledger.values())
    calls={row['identity']:(key,row) for key,row in ledger.items()}; assert len(calls)==len(ledger)
    schedule=b.load(closed/'schedule.json'); cases=0; db_count=0; replay_count=0; seen_calls=set()
    with tempfile.TemporaryDirectory(prefix='paired-verify-',dir=root) as temporary:
        for chain in config['chains']:
            data=b.load(closed/'corpus'/f'{chain}.json')
            for stage,cut in enumerate(data['cuts']):
                records=data['records'][:cut]; q=b.load(closed/'questions'/f'{chain}-{stage}.json')
                codex=t.CodexHistory(records,[end for end in schedule[chain] if end<=cut])
                files=t.ObservedFiles(records,Path(temporary)/f'{chain}-{stage}')
                for arm in b.ARMS:
                    identity=f'{chain}-{stage}-{arm}-paired'; row=b.load(root/'scores'/f'{identity}.json'); cases+=1
                    before=b.load(closed/'scores'/f'{chain}-{stage}-{arm}-closed.json')
                    projection=b.load(closed/'projections'/f'{chain}-{schedule[chain].index(cut)}-{arm}-compact.json')
                    assert before==b.load(root/'baseline'/f'{identity}.json')
                    assert row['baseline_sha256']==b.sha(before) and row['baseline_answer']==before['answer']
                    assert row['projection_sha256']==b.sha(projection['text'])==before['projection_sha256']
                    assert row['question_sha256']==b.sha(q)==before['question_sha256']
                    plan=b.load(root/'plans'/f'{identity}.json')
                    pending=set(p.pending_candidates(q['prompt'],projection['text']))
                    assert plan['pending']==p.pending_candidates(q['prompt'],projection['text'])
                    assert plan['public_candidates']==p.public_candidates(q['prompt'])
                    trace=b.load(root/'traces'/f'{identity}.json') if (root/'traces'/f'{identity}.json').exists() else []
                    schemas=t.FUTURE_TOOLS if arm in ('C','C3') else t.CODEX_TOOLS if arm=='codex' else t.FILE_TOOLS
                    offset=0; used=0; query_count=0; final=None
                    for turn in range(row['model_turns']):
                        call_id=f'{identity}-turn{turn}'; key,record=calls[call_id]; seen_calls.add(call_id)
                        request=b.load(root/'requests'/f'{key}.json'); body=request['body']
                        assert b.sha(request)==record['request_sha256']
                        assert body['model']==config['model'] and body['max_tokens']==8192 and body['thinking']=={'type':'disabled'}
                        if turn==0:
                            assert body['messages'][1]['content']==projection['text']+'\n\n'+q['prompt']
                            assert json.loads(body['messages'][2]['content'])==before['answer']
                        allowed=query_count<config['query_limit'] and used<config['byte_stop']
                        if allowed: assert body['tools']==schemas
                        else: assert 'tools' not in body
                        assert (body.get('tool_choice')=='required')==(bool(pending) and allowed)
                        actual_tools=[m['content'] for m in body['messages'] if m['role']=='tool']
                        assert actual_tools==[item['output'] for item in trace[:offset]]
                        payload=b.load(root/record['artifact'])
                        for call in payload.get('calls',[]):
                            item=trace[offset]; offset+=1; fn=call['function']
                            assert item['name']==fn['name'] and item['arguments']==fn.get('arguments')
                            assert item['bytes']==len(item['output'].encode()); used+=item['bytes']
                            if item['status']!='budget_denied': query_count+=1
                            if item['status']=='ok':
                                args=json.loads(item['arguments'])
                                checked=p.checked_candidates(item['name'],args,item['output'],pending)
                                assert item['checked_candidates']==sorted(checked); pending-=checked
                                if arm not in ('C','C3'):
                                    output=(codex.call if arm=='codex' else files.call)(item['name'],args)
                                    assert output==item['output']; replay_count+=1
                                else:
                                    native=json.loads(item['output'])
                                    assert native['sessionId']=='paired-'+identity
                                    if item['name']=='history_search': assert native['query']==args['query']
                                    else: assert native['entryId']==args['entry_id']
                            assert item['pending_after']==sorted(pending)
                        if not payload.get('calls'): final=b.parse_answer(payload['text'])
                    assert offset==len(trace) and query_count==row['logical_queries'] and used==row['returned_bytes']
                    assert sorted(pending)==row['pending_remaining'] and (not pending)==row['verification_attempts_complete']
                    assert final==row['answer']
                    score=b.exam.score(final,dict.fromkeys(q['present']),dict.fromkeys(q['decoys']))
                    assert all(row[k]==v for k,v in score.items())
                    delta=p.paired_delta(before['answer'],final,q); assert all(row[k]==v for k,v in delta.items())
                    assert row['net_gain']==row['hits']-before['hits']
                    if arm in ('C','C3'):
                        proof=b.load(root/'database-proofs'/f'{identity}.json')
                        with sqlite3.connect(Path(proof['database']).as_uri()+'?mode=ro',uri=True) as db:
                            state=db.execute('select status from legacy_imports where session_id=?',('paired-'+identity,)).fetchone()
                            count=db.execute("select count(*) from entries where session_id=? and entry_type in ('user','assistant','tool')",('paired-'+identity,)).fetchone()[0]
                        assert state==('imported',) and count==proof['expected']; db_count+=1
    assert cases==72 and len(list((root/'scores').glob('*.json')))==72 and seen_calls==set(calls)
    result=p.report(root)
    assert result['total_spent_or_reserved']<=300
    result.update(artifact_consistent=True,paired_baselines_verified=cases,model_requests_verified=len(calls),
                  local_queries_replayed=replay_count,native_databases_verified=db_count,
                  scope='paired retrieval+revision uplift; same limited local replicas/file substrate, not isolated causal effect or product ranking')
    b.save(root/'verified-report.json',result)
    return result


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for name in ('output','closed','prior','superseded'): parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args()
    print(json.dumps(verify(args.output,args.closed,args.prior,args.superseded),ensure_ascii=False,indent=2))
