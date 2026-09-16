#!/usr/bin/env python3
"""Offline v3 verification: recompute policy plans, requests, scores and costs."""
import argparse
from collections import Counter
import json
from pathlib import Path

import fidelity_rerun as f
b=f.b


def verify(root,previous,sdk,sources):
    config=b.load(root/'fidelity-manifest.json')
    assert b.load(root/'report.json')['complete'], 'run incomplete'
    ledger=b.load(root/'ledger.json'); old=b.load(previous/'ledger.json')
    assert b.sha(old)==config['prior_ledger_sha256']
    assert all(r['state']=='finished' for r in ledger.values())
    assert not any('-open' in r['identity'] for r in ledger.values()), 'unauthorized open-book call'
    for path,expected in config['code_hashes'].items(): assert b.sha((b.REPO/path).read_bytes())==expected,path
    assert b.sha(b.load(sdk/'package-lock.json'))==config['sdk_lock_sha256']
    for name,info in config['sources'].items(): assert b.sha((sources/name).read_bytes())==info['sha256']
    calls={r['identity']:(key,r) for key,r in ledger.items()}
    assert len(calls)==len(ledger)
    schedule=b.load(root/'schedule.json'); rows=[]; algorithms=Counter(); finishes=Counter(); total_tokens=Counter()
    for chain in config['chains']:
        data=b.load(root/'corpus'/f'{chain}.json')
        original=b.load(previous/'corpus'/f'{chain}.json')
        expected,ends=f.safe_plan(original)
        assert data==expected and ends==schedule[chain]
        prior={a:None for a in b.ARMS}; start=0
        for step,end in enumerate(ends):
            raw=data['records'][start:end]
            fresh=b.load(root/'normalized'/f'{b.sha(raw)}.json')
            f.to_chat(fresh)
            for arm in b.ARMS:
                identity=f'{chain}-{step}-{arm}-compact'
                p=b.load(root/'projections'/f'{identity}.json')
                key,row=calls[identity]; response=b.load(root/row['artifact']); request=b.load(root/'requests'/f'{key}.json')
                assert b.sha(request)==row['request_sha256']
                assert p['input_sha256']==b.sha(data['records'][:end])
                assert p['previous_sha256']==b.sha(prior[arm])
                if arm in ('C3','C'):
                    assert p['text']==response['text']
                    assert p['thinking_level']=='off'
                    for logical in p['logical_requests']:
                        assert logical['system_prompt']==(sources/'codex-base.md').read_text()
                        assert 0<logical['max_output_tokens']<=8192
                        f.to_chat(logical['messages'])
                    if arm=='C': assert p['model_requests']==0
                    else: algorithms[(p.get('checkpoint') or {}).get('algorithm_version','unchanged')]+=1
                else:
                    live=(prior[arm]['live'] if prior[arm] else [])+fresh
                    assert p['summary']==response['text'].strip()
                    body=request['body']; assert body['thinking']=={'type':'disabled'}
                    finishes[(arm,response.get('finish'))]+=1
                    total_tokens[arm]+=(response.get('usage') or {}).get('completion_tokens',0)
                    if arm=='codex':
                        expected_messages=[{'role':'system','content':(sources/'codex-base.md').read_text()}]+f.to_chat(live)+[
                            {'role':'user','content':(sources/'codex-compact.md').read_text()}]
                        assert body['messages']==expected_messages
                        assert body['max_tokens']==config['metadata']['maxTokens']
                        kept=f.codex_keep(live,p['summary'])
                        assert p['live']==kept and p['text']==f.render(kept)
                    else:
                        cap=min(config['metadata']['maxTokens'],32000)
                        selected=f.sdk_select(live,sdk,cap)
                        planned=b.load(root/'sdk-plans'/f'{identity}.json')
                        assert planned=={'input_sha256':b.sha(live),'selection':selected}
                        assert body['messages']==[
                            {'role':'system','content':(sources/'opencode-system.txt').read_text()},
                            {'role':'user','content':b.ext.opencode_summary_request(selected['head'],prior[arm].get('summary') if prior[arm] else None)}]
                        assert body['max_tokens']==cap
                        assert p['live']==selected['tailMessages']
                        assert p['text']=='[Context compaction summary]: '+p['summary']+'\n\n'+f.render(p['live'])
                prior[arm]=p
            start=end
            if end not in data['cuts']: continue
            stage=data['cuts'].index(end); q=b.load(root/'questions'/f'{chain}-{stage}.json')
            assert q==b.questionnaire(data['records'][:end],stage)
            for arm in b.ARMS:
                identity=f'{chain}-{stage}-{arm}-closed'; key,call=calls[identity]
                row=b.load(root/'scores'/f'{identity}.json'); payload=b.load(root/call['artifact'])
                request=b.load(root/'requests'/f'{key}.json')
                assert request['body']['thinking']=={'type':'disabled'}
                assert request['body']['messages']==[{'role':'system','content':b.SYSTEM},
                    {'role':'user','content':prior[arm]['text']+'\n\n'+q['prompt']}]
                answer=b.parse_answer(payload['text']); assert row['answer']==answer
                scores=b.exam.score(answer,dict.fromkeys(q['present']),dict.fromkeys(q['decoys']))
                assert all(row[k]==v for k,v in scores.items())
                assert row['projection_sha256']==b.sha(prior[arm]['text']) and row['question_sha256']==b.sha(q)
                assert row['contained']==sum(v in prior[arm]['text'] for v in q['present'])
                rows.append(row)
    assert len(rows)==len(list((root/'scores').glob('*.json')))==72
    assert {(r['chain'],r['stage'],r['arm']) for r in rows}=={(c,s,a) for c in config['chains'] for s in range(3) for a in b.ARMS}
    result=f.report(root)
    result.update(verified=True,prior_ledger_unchanged=True,open_model_calls=0,
                  C3_algorithms=dict(algorithms),external_finish_counts={f'{a}/{k}':v for (a,k),v in finishes.items()},
                  external_output_tokens=dict(total_tokens))
    assert result['total_spent_or_reserved']<=config['budget']
    b.save(root/'verified-report.json',result)
    return result


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for name in ('output','previous','sdk-root','sources'): parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args()
    print(json.dumps(verify(args.output.resolve(),args.previous.resolve(),args.sdk_root.resolve(),args.sources.resolve()),indent=2))
