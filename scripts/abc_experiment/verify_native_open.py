"""Artifact/score verification only; does not certify fairness of scope policy."""
import argparse
from collections import Counter
import json
from pathlib import Path
import native_open_exam as n
b=n.b


def verify(root,closed,prior):
    config=b.load(root/'native-open-manifest.json')
    ledger=b.load(root/'ledger.json'); old=b.load(prior/'ledger.json')
    assert b.sha(old)==config['prior_native_ledger_sha256']
    assert b.sha(b.load(closed/'ledger.json'))==config['closed_ledger_sha256']
    assert b.sha(b.load(closed/'fidelity-manifest.json'))==config['closed_manifest_sha256']
    for name,value in config['code_hashes'].items(): assert b.sha((b.REPO/name).read_bytes())==value,name
    assert all(row['state']=='finished' for row in ledger.values())
    calls={row['identity']:(key,row) for key,row in ledger.items()}; assert len(calls)==len(ledger)
    rows=[]; statuses=Counter(); native_open_matches=0; native_codex_matches=0
    for chain in config['chains']:
        for stage in range(3):
            question=b.load(closed/'questions'/f'{chain}-{stage}.json')
            for arm in b.ARMS:
                ident=f'{chain}-{stage}-{arm}-native-open'
                row=b.load(root/'scores'/f'{ident}.json'); rows.append(row)
                baseline=b.load(closed/'scores'/f'{chain}-{stage}-{arm}-closed.json')
                assert row['projection_sha256']==baseline['projection_sha256']
                assert row['question_sha256']==b.sha(question)
                last_key,last=calls[ident+f'-turn{row["model_turns"]-1}']
                answer=b.parse_answer(b.load(root/last['artifact'])['text'])
                assert row['answer']==answer
                score=b.exam.score(answer,dict.fromkeys(question['present']),dict.fromkeys(question['decoys']))
                assert all(row[k]==v for k,v in score.items())
                trace=b.load(root/'traces'/f'{ident}.json')
                assert len(trace)==row['tool_calls']
                assert sum(x['bytes'] for x in trace)==row['returned_bytes']
                assert all(x['bytes']==len(x['output'].encode()) for x in trace)
                assert sum(x['status'] in ('native','native_error') for x in trace)==row['native_calls']
                assert sum(x['status']=='scope_denied' for x in trace)==row['scope_denied']
                statuses.update(x['status'] for x in trace)
                for turn in range(row['model_turns']):
                    key,call=calls[ident+f'-turn{turn}']
                    request=b.load(root/'requests'/f'{key}.json')
                    assert b.sha(request)==call['request_sha256']
                    assert request['body']['thinking']=={'type':'disabled'}
                    if turn==0: assert request['body']['tool_choice']=='required'
                control=root/'control'/ident
                if arm=='opencode':
                    native=[]
                    for file in control.glob('cli-*.stdout'):
                        try: data=json.loads(file.read_text())
                        except (json.JSONDecodeError,UnicodeError): continue
                        if isinstance(data,dict) and 'tool' in data: native.append(data)
                    for item in trace:
                        if item['status']=='native':
                            assert any(x['tool']==item['name'] and x['result']['output']==item['output'] for x in native), 'native OpenCode output mismatch'
                            native_open_matches+=1
                if arm=='codex':
                    native=b.load(control/'native-requests.json')
                    outputs=[x['output'] for req in native for x in req.get('input',[]) if x.get('type') in ('function_call_output','custom_tool_call_output')]
                    for item in trace:
                        if item['status']=='native':
                            assert item['output'] in outputs,'native Codex output mismatch'
                            native_codex_matches+=1
    assert len(rows)==len(list((root/'scores').glob('*.json')))==72
    result=n.report(root)
    result.update(artifact_consistent=True,native_output_matches={'codex':native_codex_matches,'opencode':native_open_matches},
                  tool_statuses=dict(statuses),fairness_certified=False,
                  caveat='Study guard still rejects some native read-only idioms/temporary paths. Native engines ran, but constraints and budget differ in effective impact; no universal native-product ranking is certified.')
    assert result['total_spent_or_reserved']<=config['budget']
    b.save(root/'verified-artifacts.json',result)
    return result


if __name__=='__main__':
    ap=argparse.ArgumentParser()
    for key in ('output','closed','prior'): ap.add_argument('--'+key,type=Path,required=True)
    args=ap.parse_args()
    print(json.dumps(verify(args.output,args.closed,args.prior),indent=2))
