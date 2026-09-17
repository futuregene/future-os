#!/usr/bin/env python3
"""Whole-cohort C3 prompt iterations toward explicitly verified complete recall.

No gold labels/old answers enter inference. Only prompts vary; real native
query adapters, original questions/projections, model and auto choice remain.
"""
import argparse
import copy
import json
import os
from pathlib import Path
import random
import subprocess
import sys

import autonomous_open_exam as a
import interface_open_exam as e
b=a.b; f=a.f

RULES=('## Original-archive literal occurrence audit\n'
    'This complete-history task is a literal STRING-occurrence audit, not a summary or a judgment about whether past claims were true. '
    'A value that was later corrected still occurred. A candidate can occur as an exact substring inside a longer path, identifier, UUID, color code or numeric text; '
    'this is not a claim that the larger number has the same numeric value. Preserve candidate spelling in the answer.\n'
    'The projection contains generated summaries and selected excerpts, so audit the candidate list against the ORIGINAL session using history_search. '
    'For each requested candidate, issue a literal history_search query with limit=1; independent queries can be sent together. '
    'Read history_get or request more matches only if a returned snippet does not suffice. Stay within the available allowance; do not invent IDs or repeat settled checks.\n'
    'CRITICAL evidence distinction: the top-level query field is your input echoed back even on failure or zero matches. '
    'It is NEVER evidence that the value occurred. Lookup response metadata (session/entry/call IDs) and the CURRENT lookup arguments are not occurrence evidence. '
    'Original historical tool-call arguments inside a retrieved original record do count as historical text. '
    'Use the original text in matches[].snippet, or the original entry text returned by history_get. '
    'Confirm exact candidate spelling in that original text. A nonempty matches array with different spelling is not enough.\n'
    'If a candidate has matches=[] and no other original record containing it has been read, do NOT put it in appeared. '
    'Do not count the candidate list in the current question, this instruction, or a generated handoff summary as original archival proof. '
    'If retrieval is unavailable or the allowance ends, report only candidates with adequate original evidence; never guess.\n'
    'After checking, return the COMPLETE requested JSON {"appeared":[...]} with all confirmed candidates, not only the last batch of findings. '
    'Check the final list against the positive original excerpts and remove any item supported only by a query echo or empty result. '
    'No explanation or intermediate checklist belongs in the final JSON.')

EXAMPLE=('\n\nIllustrative schema example ONLY, not session evidence and not an answer to the task:\n'
    'Candidate DEMO_ONLY: {"query":"DEMO_ONLY","matches":[]} => unconfirmed; do not include it.\n'
    'Candidate SAMPLE_TOKEN: {"query":"SAMPLE_TOKEN","matches":[{"snippet":"previous tag was SAMPLE_TOKEN"}]} '
    '=> original snippet confirms it. Use this distinction on the actual candidates; never output these demo strings.')


def make_body(original,old_sid,sid,variant):
    body=copy.deepcopy(original)
    body['messages'][0]['content']=body['messages'][0]['content'].replace(old_sid,sid)
    instruction=RULES+(EXAMPLE if variant=='example-tail' else '')
    if variant=='head': body['messages'][0]['content']+='\n\n'+instruction
    elif variant in ('tail','example-tail'): body['messages'].append({'role':'system','content':instruction})
    else: raise ValueError('unknown variant')
    assert body['messages'][1]==original['messages'][1]
    assert body['tools']==e.t.FUTURE_TOOLS and body['tool_choice']=='auto'
    return body


def answer(calls,identity,initial,engine,root):
    messages=copy.deepcopy(initial['messages']); trace=[]; used=0; size=0; final=None; finish=None; turns=0
    for turn in range(a.QUERY_LIMIT+1):
        body=copy.deepcopy(initial); body['messages']=messages
        if used>=a.QUERY_LIMIT or size>=a.BYTE_STOP:
            body.pop('tools'); body.pop('tool_choice')
            messages.append({'role':'user','content':'The lookup allowance is exhausted. Return the requested JSON using established original evidence; do not guess.'})
        payload=calls.request(identity+f'-turn{turn}',body); turns+=1; finish=payload.get('finish')
        if not payload.get('calls'): final=b.parse_answer(payload['text']); break
        messages.append({'role':'assistant','content':payload['text'] or None,'tool_calls':payload['calls']})
        for call in payload['calls']:
            fn=call['function']; status='ok'
            if used>=a.QUERY_LIMIT or size>=a.BYTE_STOP: status='budget_denied'; output='LOOKUP_BUDGET_EXHAUSTED'
            else:
                used+=1
                try: output=e.future_call(engine,fn['name'],json.loads(fn.get('arguments') or '{}'))
                except (ValueError,RuntimeError) as error: status='tool_error'; output=str(error)
            n=len(output.encode()); size+=n
            trace.append({'name':fn['name'],'arguments':fn.get('arguments'),'status':status,'output':output,'bytes':n})
            b.save(root/'traces'/f'{identity}.json',trace)
            messages.append({'role':'tool','tool_call_id':call['id'],'content':output})
    return {'answer':final,'valid_answer':final is not None,'finish':finish,'logical_queries':used,'returned_bytes':size,
            'model_turns':turns,'lookup_used':used>0,'tool_errors':sum(x['status']=='tool_error' for x in trace)}


def report(root):
    m=b.load(root/'manifest.json'); rows=[b.load(path) for path in (root/'results').glob('*.json')]
    ledger=b.load(root/'ledger.json') if (root/'ledger.json').exists() else {}
    result={'complete':len(rows)==18,'cases':len(rows),'variant':m['variant'],
        **{key:sum(row[key] for row in rows) for key in ('hits','of_present','false_positives','logical_queries','returned_bytes','model_turns','tool_errors')},
        'lookup_cases':sum(row['lookup_used'] for row in rows),'invalid':sum(not row['valid_answer'] for row in rows),
        'real_hits':sum(row['hits'] for row in rows if row['chain'].startswith('real-')),
        'no_lookup_cases':[row['id'] for row in rows if not row['lookup_used']]}
    result['target_met_observed']=(result['complete'] and result['lookup_cases']==18 and result['hits']==178 and result['false_positives']==0 and result['invalid']==0)
    result['new_spend']=sum(row.get('charged',row['reserved']) for row in ledger.values())
    result['total_spend']=m['opening_spend']+result['new_spend']
    b.save(root/'report.json',result); print(json.dumps(result,ensure_ascii=False),flush=True)
    return result


def main():
    parser=argparse.ArgumentParser()
    for key in ('prior','reference','source','output','future','dumper','bridge'): parser.add_argument('--'+key,type=Path,required=True)
    parser.add_argument('--variant',choices=('head','tail','example-tail'),required=True)
    parser.add_argument('--prepare-only',action='store_true'); parser.add_argument('--detach',action='store_true')
    args=parser.parse_args()
    for key in ('prior','reference','source','output','future','dumper','bridge'): setattr(args,key,getattr(args,key).resolve())
    args.output.mkdir(parents=True,exist_ok=True); args.output.chmod(0o700)
    if args.detach:
        if (args.output/'runner.lock').exists(): raise RuntimeError('runner already active')
        with (args.output/'runner.log').open('ab') as log:
            child=subprocess.Popen([sys.executable,'-u',str(Path(__file__).resolve()),*[v for v in sys.argv[1:] if v!='--detach']],stdout=log,stderr=subprocess.STDOUT,cwd=b.REPO,start_new_session=True)
        print(json.dumps({'pid':child.pid})); return
    lock=args.output/'runner.lock'; fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600); os.write(fd,str(os.getpid()).encode()); os.close(fd)
    try:
        previous=b.load(args.prior/'verified-report.json'); prior_ledger=b.load(args.prior/'ledger.json')
        reference=b.load(args.reference/'manifest.json')
        assert previous['complete'] and previous['artifact_consistent'] and all(row['state']=='finished' for row in prior_ledger.values())
        jobs=[]
        for old in reference['jobs']:
            if old['arm']!='C3' or old['mode']!='open': continue
            initial=b.load(args.reference/'initial-bodies'/f'{old["identity"]}.json'); assert b.sha(initial)==old['initial_body_sha256']
            identity=f'{old["chain"]}-{old["stage"]}-{args.variant}'; sid='verbatim-'+identity
            body=make_body(initial,old['session_id'],sid,args.variant)
            b.immutable(args.output/'initial-bodies'/f'{identity}.json',body)
            jobs.append({'id':identity,'chain':old['chain'],'stage':old['stage'],'step':old['step'],'cut':old['cut'],
                'session_id':sid,'reference_identity':old['identity'],'initial_body_sha256':b.sha(body),
                'projection_sha256':old['projection_sha256'],'question_sha256':old['question_sha256']})
        random.Random(19403).shuffle(jobs); assert len(jobs)==18
        config={'version':1,'variant':args.variant,'model':reference['model'],'jobs':jobs,'rules':RULES,'example':EXAMPLE if args.variant=='example-tail' else '',
            'opening_spend':previous['total_spend'],'total_budget':min(300,previous['total_spend']+5),
            'prior_root':str(args.prior),'reference_root':str(args.reference),'source_root':str(args.source),
            'prior_ledger_sha256':b.sha(prior_ledger),'reference_manifest_sha256':b.sha(reference),
            'code_hashes':{str(path.relative_to(b.REPO)):b.sha(path.read_bytes()) for path in (Path(__file__),Path(a.__file__),Path(e.__file__),Path(e.t.__file__),Path(f.__file__),Path(b.__file__),b.HERE/'native_stores.py',b.HERE/'native_codex.py')},
            'binaries':{key:{'path':str(getattr(args,key)),'sha256':b.sha(getattr(args,key).read_bytes())} for key in ('future','dumper','bridge')},
            'logical_query_limit':a.QUERY_LIMIT,'byte_stop':a.BYTE_STOP,
            'scope':'prompt-directed complete literal archive audit, auto API not forced; all18 per version; no program candidate gate, labels or prior answer in context; tuning not held-out evidence'}
        b.immutable(args.output/'manifest.json',config)
        if args.prepare_only: return
        calls=a.Calls(args.output,config['total_budget'],config['model'],args.bridge,b.sha(config),previous_spend=config['opening_spend'])
        for job in jobs:
            dest=args.output/'results'/f'{job["id"]}.json'
            if dest.exists(): continue
            if any(row['identity'].startswith(job['id']+'-turn') for row in calls.rows.values()): raise RuntimeError('partial case: no silent retry')
            records=b.load(args.source/'corpus'/f'{job["chain"]}.json')['records'][:job['cut']]
            messages=f.normalize(args.output,args.dumper,config['model'],records); case=args.output/'cases'/job['id']
            engine=e.NativeFuture(args.future,case/'workspace',case/'home',case/'control',messages,job['session_id'])
            try:
                b.immutable(args.output/'database-proofs'/f'{job["id"]}.json',e.database_proof(engine,messages))
                initial=b.load(args.output/'initial-bodies'/f'{job["id"]}.json')
                result=answer(calls,job['id'],initial,engine,args.output)
                question=b.load(args.reference/'questions'/f'{job["chain"]}-{job["stage"]}.json')
                score=b.exam.score(result['answer'],dict.fromkeys(question['present']),dict.fromkeys(question['decoys']))
                b.immutable(dest,dict(result,**score,**job))
            finally: engine.close()
            report(args.output)
        report(args.output)
    finally: lock.unlink()


if __name__=='__main__': main()
