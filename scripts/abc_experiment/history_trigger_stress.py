#!/usr/bin/env python3
"""Held-out 2x2x2 history-trigger calibration, plus absent-history controls.

Three new facts x short/long x source hint/policy x hidden/visible. Fixed 26
cells, no required tools, no full benchmark rerun and no outcome-based retries.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import random
import subprocess
import sys

import history_trigger_calibration as c
b=c.b; e=c.e; a=c.a

BASE=c.BASE
FACTS=[
    {'topic':'Aspen-release','field':'release_tag','value':'aspen-canary-'+hashlib.sha256(b'trigger-stress-release').hexdigest()[:12]},
    {'topic':'Reef-transfer','field':'handoff_timeout_ms','value':'2917 ms'},
    {'topic':'Juniper-manifest','field':'manifest_path','value':'build/juniper/'+hashlib.sha256(b'trigger-stress-path').hexdigest()[:12]+'/manifest.json'},
]
for fact in FACTS: fact['text']=f'{fact["topic"]} 最终确认：{fact["field"]} = {fact["value"]}。此记录为当前约定，早期试验值不再采用。'


def distractors(target_chars=60000):
    paragraphs=[]; size=0
    areas=['rendering','transport','schema','navigation','storage','packaging','telemetry']
    while size<target_chars:
        i=len(paragraphs); area=areas[i%len(areas)]; marker=hashlib.sha256(f'unrelated-{i}'.encode()).hexdigest()[:10]
        text=(f'Archive note {i:04d}, unrelated {area} maintenance. The team reviewed test group {marker}, '
              f'kept compatibility variant {i%9}, and compared {3+i%5} local fixtures. '
              'This note records routine implementation details, not deployment parameters for the current question. '
              'The observed checks passed in the local fixture; external environments still need independent validation. '
              'Earlier estimates were provisional and are not a substitute for exact configuration values. '
              f'Next action for this work item: review module note-{i:04d}.md and retain the existing evidence links.')
        paragraphs.append(text); size+=len(text)+2
    return paragraphs


def fixture():
    return [{'role':'user','content':[{'type':'text','text':'记录各项目确认参数，后续回答必须按具体项目区分。'}]},
            *[{'role':'assistant','content':[{'type':'text','text':fact['text']}]} for fact in FACTS],
            *[{'role':'assistant','content':[{'type':'text','text':text}]} for text in distractors()]]


def jobs():
    result=[]
    for index in range(3):
        for length in ('short','long'):
            for policy in (False,True):
                for visible in (False,True):
                    result.append({'id':f'{index}-{length}-p{int(policy)}-v{int(visible)}','fact':index,'length':length,'policy':policy,'visible':visible,'absent':False})
    for policy in (False,True): result.append({'id':f'absent-long-p{int(policy)}','fact':0,'length':'long','policy':policy,'visible':False,'absent':True})
    random.Random(19117).shuffle(result)
    return result


def messages_for(job,guide):
    fact=FACTS[job['fact']]
    system=BASE+'\n\n'+guide+('\n\n'+c.POLICY if job['policy'] else '')
    status='当前可见内容是压缩后的投影；早期项目参数保留在原始会话历史中。'
    if job['visible']: status+='\n当前可见的已确认记录：'+fact['text']
    filler='\n\n'.join(distractors()) if job['length']=='long' else distractors()[0]
    topic,field=('Willow-check','source_fingerprint') if job['absent'] else (fact['topic'],fact['field'])
    question=f'我们之前给 {topic} 确定的 {field} 是什么？请给出原记录中的准确值，不要猜测。'
    messages=[{'role':'system','content':system},{'role':'user','content':status+'\n\n'+filler+'\n\n'+question}]
    if not job['visible']: assert fact['value'] not in json.dumps(messages,ensure_ascii=False)
    assert all(old['value'] not in json.dumps(messages,ensure_ascii=False) for old in c.FACTS)
    return messages


def run_case(calls,job,engine,guide,root):
    messages=messages_for(job,guide); initial=copy.deepcopy(messages); trace=[]; logical=0; final=''; turns=0; prompt_tokens=[]
    for turn in range(9):
        body={'model':calls.model,'messages':messages,'thinking':{'type':'disabled'},'max_tokens':8192,
              'stream':True,'stream_options':{'include_usage':True}}
        if logical<8: body.update(tools=e.t.FUTURE_TOOLS,tool_choice='auto')
        payload=calls.request(job['id']+f'-turn{turn}',body); turns+=1
        prompt_tokens.append((payload.get('usage') or {}).get('prompt_tokens'))
        if not payload.get('calls'): final=payload['text']; break
        messages.append({'role':'assistant','content':payload['text'] or None,'tool_calls':payload['calls']})
        for call in payload['calls']:
            fn=call['function']; status='ok'
            if logical>=8: output='CALIBRATION_LOOKUP_LIMIT'; status='budget_denied'
            else:
                logical+=1
                try: output=e.future_call(engine,fn['name'],json.loads(fn.get('arguments') or '{}'))
                except (ValueError,RuntimeError) as error: output=str(error); status='tool_error'
            trace.append({'name':fn['name'],'arguments':fn.get('arguments'),'output':output,'status':status})
            b.save(root/'traces'/f'{job["id"]}.json',trace)
            messages.append({'role':'tool','tool_call_id':call['id'],'content':output})
    expected=FACTS[job['fact']]['value']; found=False
    for item in trace:
        if item['status']!='ok': continue
        data=json.loads(item['output'])
        texts=[x.get('snippet','') for x in data.get('matches',[])]+[x.get('text','') for x in data.get('chunks',[])]
        found |= any(expected in text for text in texts)
    return dict(job,answer=final,logical_queries=logical,model_turns=turns,initial_messages_sha256=b.sha(initial),
        actual_prompt_tokens=prompt_tokens,hidden_value_retrieved=found if not job['absent'] else None,
        expected_value_in_answer=expected in final if not job['absent'] else None,
        successful_auto_recall=(found and expected in final) if not job['absent'] else None,
        tool_errors=sum(item['status']=='tool_error' for item in trace))


def report(root):
    config=b.load(root/'manifest.json'); results=[b.load(path) for path in (root/'results').glob('*.json')]
    ledger=b.load(root/'ledger.json') if (root/'ledger.json').exists() else {}
    groups={}
    for length in ('short','long'):
        for policy in (False,True):
            for visible in (False,True):
                selected=[x for x in results if not x['absent'] and x['length']==length and x['policy']==policy and x['visible']==visible]
                groups[f'{length}-p{int(policy)}-v{int(visible)}']={'cases':len(selected),
                    'used_tools':sum(x['logical_queries']>0 for x in selected),'correct':sum(x['expected_value_in_answer'] for x in selected),
                    'retrieved_and_correct':sum(x['successful_auto_recall'] for x in selected),'queries':sum(x['logical_queries'] for x in selected),
                    'initial_prompt_tokens':[x['actual_prompt_tokens'][0] for x in selected]}
    cost=sum(row.get('charged',row['reserved']) for row in ledger.values())
    result={'complete':len(results)==len(config['jobs']),'scored':len(results),'groups':groups,
            'controls':[x for x in results if x['absent']],'new_spend':cost,'total_spend':config['opening_spend']+cost}
    b.save(root/'report.json',result); print(json.dumps({'scored':len(results),'complete':result['complete'],'spent':result['total_spend']}),flush=True)
    return result


def main():
    parser=argparse.ArgumentParser()
    for key in ('prior','output','future','bridge'): parser.add_argument('--'+key,type=Path,required=True)
    parser.add_argument('--prepare-only',action='store_true'); parser.add_argument('--detach',action='store_true')
    args=parser.parse_args()
    for key in ('prior','output','future','bridge'): setattr(args,key,getattr(args,key).resolve())
    args.output.mkdir(parents=True,exist_ok=True); args.output.chmod(0o700)
    if args.detach:
        if (args.output/'runner.lock').exists(): raise RuntimeError('runner already active')
        with (args.output/'runner.log').open('ab') as log:
            child=subprocess.Popen([sys.executable,'-u',str(Path(__file__).resolve()),*[x for x in sys.argv[1:] if x!='--detach']],stdout=log,stderr=subprocess.STDOUT,cwd=b.REPO,start_new_session=True)
        print(json.dumps({'pid':child.pid})); return
    lock=args.output/'runner.lock'; fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600); os.write(fd,str(os.getpid()).encode()); os.close(fd)
    try:
        prior=b.load(args.prior/'verified-report.json'); previous=b.load(args.prior/'manifest.json'); ledger=b.load(args.prior/'ledger.json')
        assert prior['complete'] and prior['artifact_consistent'] and all(row['state']=='finished' for row in ledger.values())
        # Share the previous calibration's original CNY2 cap; do not reset it.
        ceiling=previous['total_budget']; assert ceiling<=300 and prior['total_spend']<ceiling
        config={'version':1,'kind':'held-out length x policy x visibility history-trigger calibration','model':previous['model'],
            'jobs':jobs(),'facts':FACTS,'fixture_sha256':b.sha(fixture()),'opening_spend':prior['total_spend'],'total_budget':ceiling,
            'prior_ledger_sha256':b.sha(ledger),'native_guidance_template':previous['native_guidance_template'],
            'policy':c.POLICY,'base_system':BASE,'allocation_seed':19117,'long_context_min_chars':60000,
            'code_hashes':{str(path.relative_to(b.REPO)):b.sha(path.read_bytes()) for path in (Path(__file__),Path(c.__file__),Path(a.__file__),Path(e.__file__),Path(e.t.__file__),Path(b.__file__),Path(e.f.__file__),b.HERE/'native_stores.py',b.HERE/'native_codex.py')},
            'binaries':{key:{'path':str(getattr(args,key)),'sha256':b.sha(getattr(args,key).read_bytes())} for key in ('future','bridge')},
            'scope':'synthetic long-context transfer; same natural question, archive and base within fact; no required/checklist/prior answer; no production change implied'}
        b.immutable(args.output/'manifest.json',config); b.immutable(args.output/'fixture.json',fixture())
        for job in config['jobs']:
            guide=config['native_guidance_template'].replace('"SESSION_ID"',json.dumps('stress-'+job['id']))
            b.immutable(args.output/'initial-messages'/f'{job["id"]}.json',messages_for(job,guide))
        if args.prepare_only: return
        calls=a.Calls(args.output,ceiling,config['model'],args.bridge,b.sha(config),previous_spend=config['opening_spend'])
        for job in config['jobs']:
            dest=args.output/'results'/f'{job["id"]}.json'
            if dest.exists(): continue
            if any(row['identity'].startswith(job['id']+'-turn') for row in calls.rows.values()): raise RuntimeError('partial cell: no silent retry')
            case=args.output/'cases'/job['id']; sid='stress-'+job['id']
            engine=e.NativeFuture(args.future,case/'workspace',case/'home',case/'control',fixture(),sid)
            try:
                b.immutable(args.output/'database-proofs'/f'{job["id"]}.json',e.database_proof(engine,fixture()))
                guide=config['native_guidance_template'].replace('"SESSION_ID"',json.dumps(sid))
                b.immutable(dest,run_case(calls,job,engine,guide,args.output))
            finally: engine.close()
            report(args.output)
        report(args.output)
    finally: lock.unlink()


if __name__=='__main__': main()
