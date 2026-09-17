#!/usr/bin/env python3
"""Paired closed-answer -> required retrieval -> revised-answer experiment.

Pending candidates come ONLY from the public question and projection, never
from gold labels or previous mistakes. Same interface backends as interface-v1.
"""
import argparse
import json
import os
from pathlib import Path
import random
import re
import subprocess
import sys
import time

import interface_open_exam as e
b=e.b; f=e.f; t=e.t
QUERY_LIMIT=64
BYTE_STOP=262144


class PairedCalls(f.Calls):
    def model_call(self,identity,messages,tools=None,require_tool=False):
        body={'model':self.model,'messages':messages,'max_tokens':8192,'stream':True,
              'stream_options':{'include_usage':True},'thinking':{'type':'disabled'}}
        if tools:
            body['tools']=tools
            if require_tool: body['tool_choice']='required'
        request={'model':self.model,'body':body}
        reserve=(len(json.dumps(body).encode())*5+8192*20)/1e6
        def parse(stdout):
            payload=b.parse_sse(stdout)
            if payload.get('error'): raise RuntimeError(str(payload['error']))
            return payload,(payload.get('usage') or {}).get('credit_cost')
        return self.execute(identity,request,[str(self.bridge)],reserve,parse,json.dumps(request))


def public_candidates(prompt):
    marker='\nValues:\n'
    if marker not in prompt: raise ValueError('public candidate list missing')
    candidates=[line[2:] for line in prompt.split(marker,1)[1].splitlines() if line.startswith('- ')]
    if not candidates or len(candidates)!=len(set(candidates)): raise ValueError('invalid public candidate list')
    return candidates


def pending_candidates(prompt,projection):
    return [value for value in public_candidates(prompt) if value not in projection]


def checked_candidates(name,args,output,pending):
    """An attempt is not proof of absence. Query echo alone is never evidence."""
    checked=set(); evidence=[]
    if name in ('history_search','history_search_contents'):
        data=json.loads(output)
        if 'error' in data: return checked
        for value in pending:
            if args.get('query')==value: checked.add(value)
        if name=='history_search': evidence=[x.get('snippet','') for x in data.get('matches',[])]
        else: evidence=[x.get('truncated_content','') for x in data.get('items',[])]
    elif name=='history_get':
        data=json.loads(output); evidence=[x.get('text','') for x in data.get('chunks',[])]
    elif name=='history_read_item': evidence=[json.loads(output).get('content','')]
    elif name=='history_list_items': evidence=[x.get('truncated_content','') for x in json.loads(output).get('items',[])]
    elif name=='grep':
        if 'regex parse error' in output or output.startswith('error:'): return checked
        for value in pending:
            # Fixed candidate-verification idiom; no inference from arbitrary
            # regexes or from query-string echoes in errors.
            if args.get('pattern')==re.escape(value): checked.add(value)
        if output!='No files found': evidence=[output]
    elif name in ('read','glob'):
        if not output.startswith(('File observation not available:','No files found')): evidence=[output]
    checked.update(value for value in pending if any(value in text for text in evidence))
    return checked


def paired_delta(before,after,question):
    first=set(before.get('appeared',[])); second=set(after.get('appeared',[]))
    positives=set(question['present']); decoys=set(question['decoys'])
    gained=(second-first)&positives; lost=(first-second)&positives
    return {'gained_values':sorted(gained),'lost_values':sorted(lost),'gained':len(gained),'lost':len(lost),
            'net_gain':len(gained)-len(lost),'new_false_positives':len((second-first)&decoys),
            'corrected_false_positives':len((first-second)&decoys)}


def augment(calls,identity,projection,question,baseline,tools,guide,dispatch,lookup,root):
    candidates=public_candidates(question['prompt'])
    # Label checks are assertions only; neither labels nor scores enter messages.
    assert set(candidates)==set(question['present'])|set(question['decoys'])
    initial=pending_candidates(question['prompt'],projection['text']); pending=set(initial)
    b.immutable(root/'plans'/f'{identity}.json',{'public_candidates':candidates,'pending':initial,
                'baseline_answer':baseline['answer'],'baseline_sha256':b.sha(baseline)})
    directive=('This is stage 2 of a PAIRED closed-book plus retrieval experiment. '
        'The previous answer is a draft, not an answer key. Revise it using the projection and actual retrieval evidence. '
        'You MUST attempt verification of the listed unresolved candidates before finalizing. '
        'The list is mechanically derived from the public candidates absent literally from the projection; it contains no gold labels. '
        'Do not reload candidates already established in the projection. Do not discard earlier supported facts merely because a lookup is empty. '
        'For history search, query a pending candidate exactly; do not use regex OR syntax. For file grep, use its regex-escaped literal and omit path filters initially. '
        'For presence checks, start with limit=1 on history searches; increase or read only if needed. '
        'You choose query order and may read returned item/entry IDs and follow pagination for context. '
        'A query attempt is not a finding: an empty search in incomplete files or one window does not prove absence from the original conversation. '
        'Query echoes and fixture IDs are not historical evidence. '
        f'Allowance: {QUERY_LIMIT} logical tool calls; stop initiating calls after {BYTE_STOP} UTF-8 returned bytes. '
        'Finally return only the revised {"appeared":[...]} JSON.')
    messages=[{'role':'system','content':b.SYSTEM+'\n'+guide+'\n'+directive},
              {'role':'user','content':projection['text']+'\n\n'+question['prompt']},
              {'role':'assistant','content':json.dumps(baseline['answer'],ensure_ascii=False)},
              {'role':'user','content':'Now perform stage 2. Unresolved public candidates requiring archive checks: '+json.dumps(initial,ensure_ascii=False)}]
    trace=[]; total=0; queries=0; turns=0; early_answers=0; final=None
    for turn in range(QUERY_LIMIT+3):
        available=tools if queries<QUERY_LIMIT and total<BYTE_STOP else None
        required=bool(pending) and available is not None
        if available is None:
            messages.append({'role':'user','content':'The lookup allowance is exhausted. Return the revised JSON from available evidence; unresolved items must not be called disproven merely because the budget ended.'})
        payload=calls.model_call(f'{identity}-turn{turn}',messages,tools=available,require_tool=required); turns+=1
        if not payload.get('calls'):
            if required:
                early_answers+=1
                if early_answers>=2: raise RuntimeError('provider/model bypassed required retrieval twice; stop, do not accept as measured retrieval')
                messages.extend([{'role':'assistant','content':payload['text']},
                    {'role':'user','content':'Retrieval is required before the final answer. Pending: '+json.dumps(sorted(pending),ensure_ascii=False)}])
                continue
            final=b.parse_answer(payload['text']); break
        messages.append({'role':'assistant','content':payload['text'] or None,'tool_calls':payload['calls']})
        for call in payload['calls']:
            fn=call['function']; started=time.monotonic(); status='ok'; checked=set()
            try:
                args=json.loads(fn.get('arguments') or '{}')
                if not isinstance(args,dict): raise ValueError('arguments must be an object')
                if queries>=QUERY_LIMIT or total>=BYTE_STOP: status='budget_denied'; output='LOOKUP_BUDGET_EXHAUSTED'
                else:
                    queries+=1; output=dispatch(fn['name'],args)
                    checked=checked_candidates(fn['name'],args,output,pending); pending-=checked
            except (RuntimeError,ValueError) as error: status='tool_error'; output=str(error)
            size=len(output.encode()); total+=size
            trace.append({'name':fn['name'],'arguments':fn.get('arguments'),'output':output,'bytes':size,'status':status,
                'checked_candidates':sorted(checked),'pending_after':sorted(pending),'seconds':time.monotonic()-started})
            b.save(root/'traces'/f'{identity}.json',trace)
            messages.append({'role':'tool','tool_call_id':call['id'],'content':output})
        if pending and available:
            messages.append({'role':'user','content':'Still requiring a query attempt (not gold labels): '+json.dumps(sorted(pending),ensure_ascii=False)+'. Prefer exact-candidate searches; already returned evidence can support your revision.'})
    if final is None: raise RuntimeError('no valid post-retrieval answer; retain all calls and stop')
    score=b.exam.score(final,dict.fromkeys(question['present']),dict.fromkeys(question['decoys']))
    reachable={v for v in question['present'] if v in projection['text'] or any(v in x for x in lookup)}
    return dict(score,answer=final,baseline_answer=baseline['answer'],baseline_hits=baseline['hits'],
        baseline_false_positives=baseline['false_positives'],baseline_sha256=b.sha(baseline),
        **paired_delta(baseline['answer'],final,question),projection_sha256=b.sha(projection['text']),question_sha256=b.sha(question),
        pending_initial=len(initial),pending_remaining=sorted(pending),verification_attempts_complete=not pending,
        logical_queries=queries,model_turns=turns,returned_bytes=total,tool_errors=sum(x['status']=='tool_error' for x in trace),
        reachable=len(reachable),reachable_hits=len(set(final['appeared'])&reachable),early_answers_rejected=early_answers)


def report(root):
    config=b.load(root/'manifest.json'); rows=[b.load(p) for p in (root/'scores').glob('*.json')]
    expected={(c,s,a) for c in config['chains'] for s in range(3) for a in b.ARMS}
    actual={(r['chain'],r['stage'],r['arm']) for r in rows}; assert actual<=expected and len(actual)==len(rows)
    result={'complete':actual==expected,'scored':len(rows),'expected':len(expected),'arms':{},'by_chain':{}}
    fields=('baseline_hits','hits','of_present','gained','lost','net_gain','false_positives','new_false_positives',
        'corrected_false_positives','logical_queries','model_turns','returned_bytes','tool_errors','reachable','reachable_hits','pending_initial')
    for arm in b.ARMS:
        subset=[r for r in rows if r['arm']==arm]
        result['arms'][arm]={key:sum(r[key] for r in subset) for key in fields}
        result['arms'][arm].update(fully_attempted_cases=sum(r['verification_attempts_complete'] for r in subset),
            no_lookup_cases=sum(r['logical_queries']==0 for r in subset),unattempted_candidates=sum(len(r['pending_remaining']) for r in subset))
        for chain in config['chains']:
            cases=[r for r in subset if r['chain']==chain]
            result['by_chain'].setdefault(chain,{})[arm]={k:sum(r[k] for r in cases) for k in ('baseline_hits','hits','of_present','net_gain')}
    ledger=b.load(root/'ledger.json') if (root/'ledger.json').exists() else {}
    result['new_spent_or_reserved']=sum(r.get('charged',r['reserved']) for r in ledger.values())
    result['total_spent_or_reserved']=config['opening_spend']+result['new_spent_or_reserved']
    b.save(root/'report.json',result)
    print(json.dumps({'scored':len(rows),'complete':result['complete'],'total':result['total_spent_or_reserved']}),flush=True)
    return result


def main():
    parser=argparse.ArgumentParser()
    for key in ('closed','prior','output','future','dumper','bridge'): parser.add_argument('--'+key,type=Path,required=True)
    parser.add_argument('--prepare-only',action='store_true'); parser.add_argument('--detach',action='store_true')
    args=parser.parse_args()
    for key in ('closed','prior','output','future','dumper','bridge'): setattr(args,key,getattr(args,key).resolve())
    args.output.mkdir(parents=True,exist_ok=True); args.output.chmod(0o700)
    if args.detach:
        if (args.output/'runner.lock').exists(): raise RuntimeError('runner already locked')
        with (args.output/'runner.log').open('ab') as log:
            child=subprocess.Popen([sys.executable,'-u',str(Path(__file__).resolve()),*[a for a in sys.argv[1:] if a!='--detach']],cwd=b.REPO,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        print(json.dumps({'pid':child.pid})); return
    lock=args.output/'runner.lock'; fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600); os.write(fd,str(os.getpid()).encode()); os.close(fd)
    try:
        frozen=b.load(args.closed/'fidelity-manifest.json'); previous=b.load(args.prior/'verified-report.json')
        prior_ledger=b.load(args.prior/'ledger.json')
        assert previous['complete'] and previous['artifact_consistent'] and all(r['state']=='finished' for r in prior_ledger.values())
        assert b.load(args.closed/'verified-report.json')['verified']
        assert b.sha(args.bridge.read_bytes())==frozen['bridge_sha256'] and b.sha(args.dumper.read_bytes())==frozen['driver_sha256']
        config={'version':1,'design':'paired fixed closed answer -> required candidate checks -> revised answer',
            'model':frozen['model'],'chains':frozen['chains'],'budget':300,'opening_spend':previous['total_spent_or_reserved'],
            'prior_ledger_sha256':b.sha(prior_ledger),'closed_manifest_sha256':b.sha(frozen),
            'code_hashes':{str(p.relative_to(b.REPO)):b.sha(p.read_bytes()) for p in (Path(__file__),Path(e.__file__),Path(t.__file__),Path(f.__file__),Path(b.__file__),b.HERE/'native_stores.py',b.HERE/'native_codex.py')},
            'binaries':{k:b.sha(getattr(args,k).read_bytes()) for k in ('future','dumper','bridge')},
            'backend_manifest_sha256':b.sha(b.load(args.prior/'manifest.json')),
            'query_limit':QUERY_LIMIT,'byte_stop':BYTE_STOP,'no_new_closed_draw':True,
            'selection':'all public candidates absent literally from projection; no gold-based case/item selection',
            'interpretation':'descriptive pipeline uplift from retrieval plus revision, not isolated causal retrieval effect; same local replicas/file coverage limits as interface-v1'}
        b.immutable(args.output/'manifest.json',config)
        calls=PairedCalls(args.output,300,frozen['model'],args.bridge,b.sha(config),previous_spend=config['opening_spend'])
        schedule=b.load(args.closed/'schedule.json'); rng=random.Random(b.SEED+8)
        for chain in frozen['chains']:
            data=b.load(args.closed/'corpus'/f'{chain}.json')
            for stage,cut in enumerate(data['cuts']):
                records=data['records'][:cut]; question=b.load(args.closed/'questions'/f'{chain}-{stage}.json'); step=schedule[chain].index(cut)
                codex=t.CodexHistory(records,[x for x in schedule[chain] if x<=cut]); files=t.ObservedFiles(records,args.output/'file-fixtures'/f'{chain}-{stage}')
                messages=f.normalize(args.output,args.dumper,frozen['model'],records)
                arms=list(b.ARMS); rng.shuffle(arms)
                for arm in arms:
                    identity=f'{chain}-{stage}-{arm}-paired'; dest=args.output/'scores'/f'{identity}.json'
                    baseline=b.load(args.closed/'scores'/f'{chain}-{stage}-{arm}-closed.json')
                    projection=b.load(args.closed/'projections'/f'{chain}-{step}-{arm}-compact.json')
                    assert baseline['projection_sha256']==b.sha(projection['text']) and baseline['question_sha256']==b.sha(question)
                    b.immutable(args.output/'baseline'/f'{identity}.json',baseline)
                    b.immutable(args.output/'pending'/f'{identity}.json',pending_candidates(question['prompt'],projection['text']))
                    if args.prepare_only: continue
                    if dest.exists(): assert b.load(dest)['baseline_sha256']==b.sha(baseline); continue
                    if any(r['identity'].startswith(identity+'-turn') for r in calls.rows.values()): raise RuntimeError('partial case requires explicit recovery, not a blind retry')
                    engine=None
                    try:
                        if arm in ('C','C3'):
                            case=args.output/'cases'/identity
                            engine=e.NativeFuture(args.future,case/'workspace',case/'home',case/'control',messages,'paired-'+identity)
                            b.immutable(args.output/'database-proofs'/f'{identity}.json',e.database_proof(engine,messages))
                            tools=t.FUTURE_TOOLS; lookup=[t.body(r) for r in records]; dispatch=lambda name,args:e.future_call(engine,name,args)
                            guide='history_search/history_get call the real Future CLI and original journal. Search is literal and ASCII case-insensitive. Scope is this session only; offsets for get are UTF-8 bytes.'
                        elif arm=='codex':
                            tools=t.CODEX_TOOLS; lookup=[r['content'] for r in codex.flat]; dispatch=codex.call
                            guide='Use the LOCAL REPRODUCTION of Codex dedicated history interfaces. Search uses case-sensitive literal substrings. Use global search first; short head excerpts may omit a matching value, so use item/window IDs for reads when needed. Missing namespace metadata is unknown; omit that filter. Read offsets are Unicode characters.'
                        else:
                            tools=t.FILE_TOOLS; lookup=list(files.files)+list(files.files.values()); dispatch=files.call
                            guide='Use the LOCAL OpenCode file-tool reproduction. Virtual root is / and relative names resolve under /workspace. These are last observed read fragments, not a complete working tree. Grep literal candidates using escaped regex patterns. Missing results do not prove absence from the original conversation.'
                        result=augment(calls,identity,projection,question,baseline,tools,guide,dispatch,lookup,args.output)
                        assert result['hits']-baseline['hits']==result['net_gain']
                        b.immutable(dest,dict(result,chain=chain,stage=stage,arm=arm))
                    finally:
                        if engine: engine.close()
                    report(args.output)
        if not args.prepare_only: report(args.output)
    finally: lock.unlink()


if __name__=='__main__': main()
