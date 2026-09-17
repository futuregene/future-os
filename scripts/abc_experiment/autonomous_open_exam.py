#!/usr/bin/env python3
"""Standard open book: replay the exact frozen closed request, adding tools only.

No closed answer, generated candidate checklist, forced query, or revision turn
is sent to the model. Gold labels and closed scores are consulted after answering.
"""
import argparse
import copy
import json
import os
from pathlib import Path
import random
import subprocess
import sys
import time

import interface_open_exam as e
b=e.b; f=e.f; t=e.t
QUERY_LIMIT=64
BYTE_STOP=262144


def schemas(arm):
    return t.FUTURE_TOOLS if arm in ('C','C3') else t.CODEX_TOOLS if arm=='codex' else t.FILE_TOOLS


def open_body(closed_body,tools):
    if 'tools' in closed_body or 'tool_choice' in closed_body:
        raise ValueError('baseline is not a closed-book request')
    result=copy.deepcopy(closed_body)
    result['tools']=copy.deepcopy(tools)
    result['tool_choice']='auto'
    return result


def assert_initial_parity(closed_body,body,tools):
    assert body.get('tool_choice')=='auto' and body.get('tools')==tools
    assert {k:v for k,v in body.items() if k not in ('tools','tool_choice')}==closed_body
    assert [m['role'] for m in body['messages']]==['system','user']


class Calls(f.Calls):
    def request(self,identity,body):
        if (self.root/'STOP').exists(): raise RuntimeError('operator stop before next paid request')
        request={'model':self.model,'body':body}
        reserve=(len(json.dumps(body).encode())*5+body['max_tokens']*20)/1e6
        def parse(stdout):
            result=b.parse_sse(stdout)
            if result.get('error'): raise RuntimeError(str(result['error']))
            return result,(result.get('usage') or {}).get('credit_cost')
        return self.execute(identity,request,[str(self.bridge)],reserve,parse,json.dumps(request))


def answer(calls,identity,closed_body,tools,dispatch,root):
    """Only the baseline REQUEST enters this function, never its answer/labels."""
    initial=open_body(closed_body,tools)
    assert_initial_parity(closed_body,initial,tools)
    messages=copy.deepcopy(initial['messages'])
    trace=[]; queries=0; delivered=0; turns=0; final=None; finish=None
    for turn in range(QUERY_LIMIT+1):
        body=copy.deepcopy(initial); body['messages']=messages
        if queries>=QUERY_LIMIT or delivered>=BYTE_STOP:
            body.pop('tools'); body.pop('tool_choice')
            # A post-tool budget notice only, never part of the initial request.
            messages.append({'role':'user','content':'The lookup allowance is exhausted. Answer the original question using the evidence already available.'})
        payload=calls.request(f'{identity}-turn{turn}',body); turns+=1; finish=payload.get('finish')
        if not payload.get('calls'):
            final=b.parse_answer(payload['text']); break
        messages.append({'role':'assistant','content':payload['text'] or None,'tool_calls':payload['calls']})
        for call in payload['calls']:
            fn=call['function']; status='ok'; began=time.monotonic()
            if queries>=QUERY_LIMIT or delivered>=BYTE_STOP:
                output='LOOKUP_BUDGET_EXHAUSTED'; status='budget_denied'
            else:
                queries+=1
                try:
                    args=json.loads(fn.get('arguments') or '{}')
                    if not isinstance(args,dict): raise ValueError('arguments must be an object')
                    output=dispatch(fn['name'],args)
                except (ValueError,RuntimeError) as error:
                    status='tool_error'; output=str(error)
            size=len(output.encode()); delivered+=size
            trace.append({'name':fn['name'],'arguments':fn.get('arguments'),'output':output,'bytes':size,
                          'status':status,'seconds':time.monotonic()-began})
            b.save(root/'traces'/f'{identity}.json',trace)
            messages.append({'role':'tool','tool_call_id':call['id'],'content':output})
    return {'answer':final,'valid_answer':final is not None,'finish':finish,'logical_queries':queries,
            'returned_bytes':delivered,'model_turns':turns,'lookup_used':queries>0,
            'tool_errors':sum(item['status']=='tool_error' for item in trace)}


def evaluate(result,question,baseline,projection,lookup):
    # Evaluation only: none of these labels or baseline outputs reach answer().
    final=result['answer']; previous=set(baseline['answer']['appeared'])
    reported=set(final.get('appeared',[])) if isinstance(final,dict) else set()
    positives=set(question['present']); decoys=set(question['decoys'])
    reachable={v for v in positives if v in projection or any(v in content for content in lookup)}
    gained=(reported-previous)&positives; lost=(previous-reported)&positives
    score=b.exam.score(final,dict.fromkeys(question['present']),dict.fromkeys(question['decoys']))
    return dict(result,**score,baseline_hits=baseline['hits'],baseline_false_positives=baseline['false_positives'],
        gained=len(gained),lost=len(lost),net_gain=len(gained)-len(lost),gained_values=sorted(gained),lost_values=sorted(lost),
        new_false_positives=len((reported-previous)&decoys),corrected_false_positives=len((previous-reported)&decoys),
        reachable=len(reachable),reachable_hits=len(reported&reachable),
        correct_outside_literal_containment=len((reported&positives)-reachable))


def report(root):
    config=b.load(root/'manifest.json'); rows=[b.load(path) for path in (root/'scores').glob('*.json')]
    expected={job['identity'] for job in config['jobs']}; actual={row['identity'] for row in rows}
    assert actual<=expected and len(actual)==len(rows)
    result={'complete':actual==expected,'scored':len(rows),'expected':len(expected),'arms':{},'by_chain':{}}
    fields=('baseline_hits','hits','of_present','false_positives','gained','lost','net_gain','logical_queries',
            'model_turns','returned_bytes','tool_errors','reachable','reachable_hits','correct_outside_literal_containment')
    for arm in b.ARMS:
        selected=[row for row in rows if row['arm']==arm]
        result['arms'][arm]={key:sum(row[key] for row in selected) for key in fields}
        result['arms'][arm].update(lookup_cases=sum(row['lookup_used'] for row in selected),
            no_lookup_cases=sum(not row['lookup_used'] for row in selected),invalid_answers=sum(not row['valid_answer'] for row in selected))
        for chain in config['chains']:
            subset=[row for row in selected if row['chain']==chain]
            result['by_chain'].setdefault(chain,{})[arm]={key:sum(row[key] for row in subset) for key in ('baseline_hits','hits','of_present','net_gain')}
    ledger=b.load(root/'ledger.json') if (root/'ledger.json').exists() else {}
    result['new_spent_or_reserved']=sum(row.get('charged',row['reserved']) for row in ledger.values())
    result['total_spent_or_reserved']=config['opening_spend']+result['new_spent_or_reserved']
    b.save(root/'report.json',result)
    print(json.dumps({'scored':len(rows),'complete':result['complete'],'spent':result['total_spent_or_reserved']}),flush=True)
    return result


def main():
    parser=argparse.ArgumentParser()
    for key in ('closed','prior','output','future','dumper','bridge'): parser.add_argument('--'+key,type=Path,required=True)
    parser.add_argument('--prepare-only',action='store_true'); parser.add_argument('--detach',action='store_true')
    args=parser.parse_args()
    for key in ('closed','prior','output','future','dumper','bridge'): setattr(args,key,getattr(args,key).resolve())
    args.output.mkdir(parents=True,exist_ok=True); args.output.chmod(0o700)
    if args.detach:
        if (args.output/'runner.lock').exists(): raise RuntimeError('runner already active')
        with (args.output/'runner.log').open('ab') as log:
            child=subprocess.Popen([sys.executable,'-u',str(Path(__file__).resolve()),*[x for x in sys.argv[1:] if x!='--detach']],cwd=b.REPO,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        print(json.dumps({'pid':child.pid})); return
    lock=args.output/'runner.lock'; fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600); os.write(fd,str(os.getpid()).encode()); os.close(fd)
    try:
        frozen=b.load(args.closed/'fidelity-manifest.json'); prior=b.load(args.prior/'verified-report.json')
        prior_ledger=b.load(args.prior/'ledger.json'); closed_ledger=b.load(args.closed/'ledger.json')
        assert prior['complete'] and prior['artifact_consistent'] and all(row['state']=='finished' for row in prior_ledger.values())
        assert b.load(args.closed/'verified-report.json')['verified']
        assert b.sha(args.bridge.read_bytes())==frozen['bridge_sha256'] and b.sha(args.dumper.read_bytes())==frozen['driver_sha256']
        index={row['identity']:key for key,row in closed_ledger.items()}
        schedule=b.load(args.closed/'schedule.json'); jobs=[]; rng=random.Random(b.SEED+9)
        for chain in frozen['chains']:
            data=b.load(args.closed/'corpus'/f'{chain}.json')
            for stage,cut in enumerate(data['cuts']):
                arms=list(b.ARMS); rng.shuffle(arms)
                for arm in arms:
                    identity=f'{chain}-{stage}-{arm}-autonomous'; old_id=f'{chain}-{stage}-{arm}-closed'; request_key=index[old_id]
                    request=b.load(args.closed/'requests'/f'{request_key}.json'); body=request['body']
                    projection=b.load(args.closed/'projections'/f'{chain}-{schedule[chain].index(cut)}-{arm}-compact.json')
                    q=b.load(args.closed/'questions'/f'{chain}-{stage}.json')
                    assert body['model']==frozen['model'] and body['max_tokens']==8192 and body['thinking']=={'type':'disabled'}
                    assert body['messages']==[{'role':'system','content':b.SYSTEM},{'role':'user','content':projection['text']+'\n\n'+q['prompt']}]
                    proposed=open_body(body,schemas(arm)); assert_initial_parity(body,proposed,schemas(arm))
                    b.immutable(args.output/'closed-requests'/f'{identity}.json',request)
                    b.immutable(args.output/'initial-open-bodies'/f'{identity}.json',proposed)
                    jobs.append({'identity':identity,'chain':chain,'stage':stage,'arm':arm,'cut':cut,'step':schedule[chain].index(cut),
                        'closed_request_sha256':b.sha(request),'closed_request_key':request_key,'initial_open_body_sha256':b.sha(proposed),
                        'projection_sha256':b.sha(projection['text']),'question_sha256':b.sha(q)})
        config={'version':1,'design':'same frozen closed request + optional tools, no prior answer/checklist/forced query',
            'model':frozen['model'],'chains':frozen['chains'],'jobs':jobs,'budget':300,'opening_spend':prior['total_spent_or_reserved'],
            'closed_manifest_sha256':b.sha(frozen),'closed_ledger_sha256':b.sha(closed_ledger),'prior_ledger_sha256':b.sha(prior_ledger),
            'code_hashes':{str(path.relative_to(b.REPO)):b.sha(path.read_bytes()) for path in (Path(__file__),Path(e.__file__),Path(t.__file__),Path(f.__file__),Path(b.__file__),b.HERE/'native_stores.py',b.HERE/'native_codex.py')},
            'binaries':{key:{'path':str(getattr(args,key)),'sha256':b.sha(getattr(args,key).read_bytes())} for key in ('future','dumper','bridge')},
            'tool_schemas':{arm:schemas(arm) for arm in b.ARMS},'logical_query_limit':QUERY_LIMIT,'byte_stop':BYTE_STOP,
            'initial_request_diff_allowlist':['tools','tool_choice'],'backend_policy':'unchanged interface-v1 local replicas/observed file fragments; no export or shell substitutes',
            'measurement_limit':'frozen lexical scoring (including known numeric-substring ambiguity), single draw, only three independent real sessions; no minimum tool-use requirement'}
        b.immutable(args.output/'manifest.json',config)
        b.immutable(args.output/'request-parity.json',{'cases':len(jobs),'all_passed':True,'only_differences':['tools','tool_choice'],
            'prior_answer_injected':False,'pending_list_injected':False,'forced_tool_choice':False})
        if args.prepare_only: return
        calls=Calls(args.output,300,frozen['model'],args.bridge,b.sha(config),previous_spend=config['opening_spend'])
        for job in jobs:
            identity=job['identity']; chain=job['chain']; stage=job['stage']; arm=job['arm']; dest=args.output/'scores'/f'{identity}.json'
            if dest.exists(): assert b.load(dest)['closed_request_sha256']==job['closed_request_sha256']; continue
            if any(row['identity'].startswith(identity+'-turn') for row in calls.rows.values()): raise RuntimeError('partial case: do not silently retry')
            data=b.load(args.closed/'corpus'/f'{chain}.json'); records=data['records'][:job['cut']]
            body=b.load(args.output/'closed-requests'/f'{identity}.json')['body']; engine=None
            try:
                if arm in ('C','C3'):
                    messages=f.normalize(args.output,args.dumper,frozen['model'],records); case=args.output/'cases'/identity
                    engine=e.NativeFuture(args.future,case/'workspace',case/'home',case/'control',messages,'autonomous-'+identity)
                    b.immutable(args.output/'database-proofs'/f'{identity}.json',e.database_proof(engine,messages))
                    dispatch=lambda name,arguments:e.future_call(engine,name,arguments); lookup=[t.body(row) for row in records]
                elif arm=='codex':
                    backend=t.CodexHistory(records,[end for end in schedule[chain] if end<=job['cut']])
                    dispatch=backend.call; lookup=[row['content'] for row in backend.flat]
                else:
                    backend=t.ObservedFiles(records,args.output/'file-fixtures'/f'{chain}-{stage}')
                    dispatch=backend.call; lookup=list(backend.files)+list(backend.files.values())
                    b.immutable(args.output/'file-visibility'/f'{identity}.json',{'paths':list(backend.files),
                        'invalidated':backend.invalidated,'unmapped_results':backend.unmapped_results,
                        'content_sha256':b.sha(backend.files)})
                result=answer(calls,identity,body,schemas(arm),dispatch,args.output)
                # Closed answer and gold labels are only read now, after generation.
                baseline=b.load(args.closed/'scores'/f'{chain}-{stage}-{arm}-closed.json')
                question=b.load(args.closed/'questions'/f'{chain}-{stage}.json')
                projection=b.load(args.closed/'projections'/f'{chain}-{job["step"]}-{arm}-compact.json')['text']
                scored=evaluate(result,question,baseline,projection,lookup)
                b.immutable(dest,dict(scored,**job,baseline_sha256=b.sha(baseline)))
            finally:
                if engine: engine.close()
            report(args.output)
        report(args.output)
    finally: lock.unlink()


if __name__=='__main__': main()
