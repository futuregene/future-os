#!/usr/bin/env python3
"""Closed-book v3: pinned external policies, SDK lowering, matched generation.

Old v2 artifacts and scientific sources are never overwritten. All four arms are
rerun because safe boundaries and C3's generation/base instructions change.
"""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import random
import subprocess
import sys

import four_arm_rerun as b


def codex_tokens(text):
    return (len(text.encode('utf-8')) + 3) // 4


def codex_truncate(text, tokens):
    raw = text.encode('utf-8')
    budget = tokens * 4
    if len(raw) <= budget:
        return text
    left = budget // 2
    right = budget - left
    prefix = raw[:left].decode('utf-8', errors='ignore')
    suffix = raw[len(raw)-right:].decode('utf-8', errors='ignore') if right else ''
    removed = (len(raw)-budget+3)//4
    return prefix + f'…{removed} tokens truncated…' + suffix


def text(message):
    return '\n'.join(c['text'] for c in message['content'] if c['type']=='text')


def codex_keep(messages, summary):
    selected, remaining = [], 20000
    for m in reversed(messages):
        if m['role'] != 'user' or m.get('is_summary'):
            continue
        if not remaining:
            break
        body = text(m)
        cost = codex_tokens(body)
        selected.append({'role':'user','content':[{'type':'text','text':body if cost<=remaining else codex_truncate(body,remaining)}]})
        if cost > remaining:
            break
        remaining -= cost
    selected.reverse()
    selected.append({'role':'user','is_summary':True,'content':[{'type':'text','text':b.ext.CODEX_SUMMARY_PREFIX+'\n'+summary}]})
    return selected


def to_chat(messages):
    """Chat-Completions transport for actual text/call/result message roles."""
    out, pending = [], set()
    for m in messages:
        if m['role']=='tool':
            for block in m['content']:
                assert block['type']=='tool_result'
                ident = block['tool_call_id']
                if ident not in pending:
                    raise ValueError('orphan tool result')
                pending.remove(ident)
                out.append({'role':'tool','tool_call_id':ident,'content':block['content']})
            continue
        if pending:
            raise ValueError('tool results not immediately after their assistant')
        calls = []
        for block in m['content']:
            if block['type']=='tool_call':
                ident = block['id']
                if ident in pending:
                    raise ValueError('duplicate call')
                pending.add(ident)
                calls.append({'id':ident,'type':'function','function':{'name':block['name'],
                    'arguments':json.dumps(block['args'],ensure_ascii=False,separators=(',',':'))}})
            elif block['type']!='text':
                raise ValueError('unsupported reduced-corpus block')
        row = {'role':m['role'],'content':text(m) or (None if calls else '')}
        if calls:
            row['tool_calls']=calls
        out.append(row)
    if pending:
        raise ValueError('dangling tool call')
    return out


def render(messages):
    parts=[]
    for m in messages:
        for c in m['content']:
            if c['type']=='text':
                parts.append(c['text'])
            elif c['type']=='tool_call':
                parts.append(f"[Assistant tool call {c['id']}]: {c['name']}({json.dumps(c['args'],ensure_ascii=False)})")
            elif c['type']=='tool_result':
                parts.append(f"[Tool {'error' if c.get('is_error') else 'result'} {c['tool_call_id']}]: {c['content']}")
    return '\n\n'.join(parts)


def safe_plan(data):
    records=data['records']
    calls=[r['call'] for r in records if r['kind']=='tool_call']
    results=[r['call'] for r in records if r['kind']=='tool_result']
    if len(calls)!=len(set(calls)) or len(results)!=len(set(results)) or set(calls)!=set(results):
        raise ValueError('ambiguous/incomplete original tool pairs: cannot faithfully replay')
    pending, safe=set(), []
    for i,r in enumerate(records):
        if r['kind']=='tool_call': pending.add(r['call'])
        if r['kind']=='tool_result': pending.remove(r['call'])
        end=i+1
        complete=end==len(records) or r.get('position',r.get('order'))!=records[end].get('position',records[end].get('order'))
        if complete and not pending: safe.append(end)
    cuts=[max(s for s in safe if s<=cut) for cut in data['cuts']]
    if len(set(cuts))!=3: raise ValueError('probe boundaries collapsed')
    ends, used=[],0
    safe_set=set(safe)
    for i,r in enumerate(records):
        used+=b.tokens(b.ext.plain([r]))+12
        end=i+1
        if end in cuts or (used>=b.CHUNK and end in safe_set):
            ends.append(end); used=0
    assert ends[-1]==len(records)
    return dict(data,cuts=cuts), ends


class Calls(b.Calls):
    def __init__(self,*args,previous_spend=0,**kwargs):
        self.previous_spend=previous_spend
        super().__init__(*args,**kwargs)
    def spent(self):
        return self.previous_spend+super().spent()


def sdk_select(messages,sdk_root,max_output):
    request={'messages':messages,'sdkRoot':str(sdk_root),'window':b.WINDOW,'maxOutput':max_output}
    raw=subprocess.check_output(['node',str(b.HERE/'opencode_fidelity.mjs')],input=json.dumps(request),text=True)
    return json.loads(raw)


def normalize(root,driver,model,records):
    key=b.sha(records)
    source=root/'inputs'/f'{key}.json'
    dest=root/'normalized'/f'{key}.json'
    b.immutable(source,records)
    if dest.exists(): return b.load(dest)
    raw=subprocess.check_output([str(driver),'--records',str(source),'--model',model,'--dump-messages'],text=True)
    messages=json.loads(raw)
    # The shared Rust normalizer must not silently drop any text/tool record.
    counts=Counter(c['type'] for m in messages for c in m['content'])
    expected=Counter(r['kind'] for r in records)
    assert counts['tool_call']==expected['tool_call']
    assert counts['tool_result']==expected['tool_result']
    for record in records:
        if record['kind']=='text': assert record['text'] in render(messages)
    to_chat(messages)  # validate sequencing before any paid request
    b.immutable(dest,messages)
    return messages


def project(calls,driver,sdk,sources,max_output,arm,records,fresh,previous,identity):
    if arm in ('C','C3'):
        input_path=calls.root/'inputs'/f'{b.sha(records)}.json'
        b.immutable(input_path,records)
        previous_cp=previous.get('checkpoint') if previous else None
        argv=[str(driver),'--records',str(input_path),'--model',calls.model,'--window',str(b.WINDOW),
              '--thinking-level','off','--system-prompt-file',str(sources/'codex-base.md')]
        if previous_cp: argv+=['--previous',json.dumps(previous_cp)]
        if arm=='C': argv+=['--no-summary']
        def parse(stdout):
            p=json.loads(stdout.strip().splitlines()[-1]); p['text']='\n\n'.join(x['text'] for x in p['projection'])
            assert p['thinking_level']=='off'
            for request in p['logical_requests']:
                assert request['system_prompt']==(sources/'codex-base.md').read_text()
                assert 0<request['max_output_tokens']<=8192
                to_chat(request['messages'])
            usage=p['usage']
            cost=usage['cost'] if usage['input_tokens'] or p['model_requests']==0 else None
            return p,cost
        request={'arm':arm,'input':b.sha(records),'previous':previous_cp,'thinking':'off','base':b.sha((sources/'codex-base.md').read_bytes())}
        reserve=0 if arm=='C' else 3*(b.WINDOW*5+b.OUTPUT*20)/1e6
        return calls.execute(identity,request,argv,reserve,parse)
    live=(previous['live'] if previous else [])+fresh
    if arm=='codex':
        messages=[{'role':'system','content':(sources/'codex-base.md').read_text()}]+to_chat(live)+[
            {'role':'user','content':(sources/'codex-compact.md').read_text()}]
        p=calls.model_call(identity,messages,cap=max_output)
        summary=p['text'].strip()
        if not summary: raise RuntimeError('empty Codex summary')
        kept=codex_keep(live,summary)
        return {'text':render(kept),'live':kept,'summary':summary,'finish':p.get('finish'),'usage':p.get('usage'),
                'max_output':max_output,'input_messages':len(messages)}
    cap=min(max_output,32000) or 32000
    planned=b.load(calls.root/'sdk-plans'/f'{identity}.json')
    assert planned['input_sha256']==b.sha(live), 'SDK plan/input mismatch'
    selected=planned['selection']
    prompt=b.ext.opencode_summary_request(selected['head'],previous.get('summary') if previous else None)
    p=calls.model_call(identity,[{'role':'system','content':(sources/'opencode-system.txt').read_text()},
                                {'role':'user','content':prompt}],cap=cap)
    summary=p['text'].strip()
    if not summary: raise RuntimeError('empty OpenCode summary; stop rather than silently skip')
    tail=selected.pop('tailMessages')
    return {'text':'[Context compaction summary]: '+summary+'\n\n'+render(tail),
            'live':tail,'summary':summary,'finish':p.get('finish'),'usage':p.get('usage'),
            'max_output':cap,'selection':selected}


def report(root):
    config=b.load(root/'fidelity-manifest.json')
    rows=[b.load(p) for p in (root/'scores').glob('*.json')]
    expected={(c,s,a) for c in config['chains'] for s in range(3) for a in b.ARMS}
    actual={(r['chain'],r['stage'],r['arm']) for r in rows}
    assert actual<=expected and len(actual)==len(rows)
    result={'complete':actual==expected,'scored':len(rows),'expected':len(expected),'arms':{},'by_chain':{}}
    for arm in b.ARMS:
        xs=[r for r in rows if r['arm']==arm]
        result['arms'][arm]={k:sum(r[k] for r in xs) for k in ('hits','of_present','false_positives','contained')}
        result['arms'][arm]['invalid']=sum(not r['valid_answer'] for r in xs)
        for chain in config['chains']:
            ys=[r for r in xs if r['chain']==chain]
            result['by_chain'].setdefault(chain,{})[arm]={k:sum(r[k] for r in ys) for k in ('hits','of_present')}
    rows=b.load(root/'ledger.json') if (root/'ledger.json').exists() else {}
    result['new_spent_or_reserved']=sum(x.get('charged',x['reserved']) for x in rows.values())
    result['total_spent_or_reserved']=config['prior_spend']+result['new_spent_or_reserved']
    result['open_calls']=0
    b.save(root/'report.json',result)
    print(json.dumps(result,ensure_ascii=False),flush=True)
    return result


def main():
    ap=argparse.ArgumentParser()
    ap.add_argument('--previous',type=Path,required=True)
    ap.add_argument('--output',type=Path,required=True)
    ap.add_argument('--driver',type=Path,required=True)
    ap.add_argument('--bridge',type=Path,required=True)
    ap.add_argument('--sdk-root',type=Path,required=True)
    ap.add_argument('--sources',type=Path,required=True)
    ap.add_argument('--prepare-only',action='store_true')
    ap.add_argument('--report-only',action='store_true')
    ap.add_argument('--detach',action='store_true')
    args=ap.parse_args()
    for key in ('previous','output','driver','bridge','sdk_root','sources'): setattr(args,key,getattr(args,key).resolve())
    args.output.mkdir(parents=True,exist_ok=True)
    if args.report_only: report(args.output); return
    if args.detach:
        if (args.output/'runner.lock').exists(): raise RuntimeError('runner present')
        with (args.output/'runner.log').open('ab') as log:
            p=subprocess.Popen([sys.executable,'-u',str(Path(__file__).resolve()),*[x for x in sys.argv[1:] if x!='--detach']],
                cwd=b.REPO,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        print(json.dumps({'pid':p.pid,'log':str(args.output/'runner.log')})); return
    lock=args.output/'runner.lock'
    fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600); os.write(fd,str(os.getpid()).encode()); os.close(fd)
    try:
        prior=b.load(args.previous/'manifest.json'); old_ledger=b.load(args.previous/'ledger.json')
        assert not (args.previous/'runner.lock').exists()
        assert all(r['state']=='finished' for r in old_ledger.values())
        prior_spend=sum(r.get('charged',r['reserved']) for r in old_ledger.values())
        auth=b.load(args.previous/'budget-amendment-001.json'); assert auth['authorized_total_budget']==300
        model=prior['model']
        metadata=json.loads(subprocess.check_output([str(args.bridge)],input=json.dumps({'model':model,'mode':'metadata'}),text=True))
        max_output=metadata['maxTokens']; assert 0<max_output<=384000
        sources=b.load(args.sources/'manifest.json')
        for name,info in sources.items(): assert b.sha((args.sources/name).read_bytes())==info['sha256']
        assert (args.sources/'codex-compact.md').read_text().strip()==b.ext.CODEX_SUMMARIZATION_PROMPT.strip()
        assert (args.sources/'opencode-system.txt').read_text().strip()==b.ext.OPENCODE_COMPACTION_SYSTEM_PROMPT.strip()
        sdklock=b.load(args.sdk_root/'package-lock.json'); assert b.load(args.sdk_root/'node_modules/ai/package.json')['version']=='6.0.168'
        code=[Path(__file__),b.HERE/'opencode_fidelity.mjs',Path(b.__file__),b.HERE/'realistic_exam.py',b.HERE.parent/'abc_external_strategies.py',b.HERE.parent/'abc_compaction_experiment.py']
        config={'version':3,'chains':prior['chains'],'model':model,'generation':'off','window':b.WINDOW,'chunk':b.CHUNK,
                'metadata':metadata,'budget':300,'prior_spend':prior_spend,'prior_ledger_sha256':b.sha(old_ledger),
                'authorization_sha256':b.sha(auth),'sources':sources,'sdk_lock_sha256':b.sha(sdklock),
                'driver_sha256':b.sha(args.driver.read_bytes()),'bridge_sha256':b.sha(args.bridge.read_bytes()),
                'code_hashes':{str(p.relative_to(b.REPO)):b.sha(p.read_bytes()) for p in code},
                'question_policy':'same seeded exact-value instrument; not task continuation',
                'scope':'text/tool policy replay; common safe forced schedule; no open-book authorization'}
        b.immutable(args.output/'fidelity-manifest.json',config)
        calls=Calls(args.output,300,model,args.bridge,b.sha(config),previous_spend=prior_spend)
        plans={}; corpus={}
        for chain in prior['chains']:
            original=b.load(args.previous/'corpus'/f'{chain}.json'); assert b.sha(original)==prior['input_hashes'][chain]
            data,ends=safe_plan(original); corpus[chain]=data; plans[chain]=ends
            b.immutable(args.output/'corpus'/f'{chain}.json',data)
            for s,cut in enumerate(data['cuts']): b.immutable(args.output/'questions'/f'{chain}-{s}.json',b.questionnaire(data['records'][:cut],s))
            start=0
            open_tail=[]
            for step,end in enumerate(ends):
                normalized=normalize(args.output,args.driver,model,data['records'][start:end])
                open_live=open_tail+normalized
                selection=sdk_select(open_live,args.sdk_root,min(max_output,32000))
                b.immutable(args.output/'sdk-plans'/f'{chain}-{step}-opencode-compact.json',
                            {'input_sha256':b.sha(open_live),'selection':selection})
                open_tail=selection['tailMessages']
                start=end
        b.immutable(args.output/'schedule.json',plans)
        print(json.dumps({'safe_stages':{c:len(e) for c,e in plans.items()},'prior_spend':prior_spend,'budget':300}),flush=True)
        if args.prepare_only: return
        rng=random.Random(b.SEED)
        for chain,data in corpus.items():
            states={a:None for a in b.ARMS}; start=0
            for step,end in enumerate(plans[chain]):
                fresh=normalize(args.output,args.driver,model,data['records'][start:end]); arms=list(b.ARMS); rng.shuffle(arms)
                for arm in arms:
                    identity=f'{chain}-{step}-{arm}-compact'; path=args.output/'projections'/f'{identity}.json'
                    if path.exists(): p=b.load(path)
                    else:
                        p=project(calls,args.driver,args.sdk_root,args.sources,max_output,arm,data['records'][:end],fresh,states[arm],identity)
                        p.update(input_sha256=b.sha(data['records'][:end]),previous_sha256=b.sha(states[arm]),arm=arm,chain=chain,end=end)
                        b.immutable(path,p)
                    assert p['input_sha256']==b.sha(data['records'][:end]) and p['previous_sha256']==b.sha(states[arm])
                    states[arm]=p
                start=end
                if end in data['cuts']:
                    s=data['cuts'].index(end); q=b.load(args.output/'questions'/f'{chain}-{s}.json'); rng.shuffle(arms)
                    for arm in arms:
                        identity=f'{chain}-{s}-{arm}-closed'; path=args.output/'scores'/f'{identity}.json'
                        if not path.exists(): b.immutable(path,dict(b.probe(calls,identity,states[arm],q),chain=chain,stage=s,arm=arm))
                        else: assert b.load(path)['projection_sha256']==b.sha(states[arm]['text'])
                    report(args.output)
        report(args.output)
    finally:
        lock.unlink()


if __name__=='__main__': main()
