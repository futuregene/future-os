#!/usr/bin/env python3
"""Stronger prompt-only C3 archive-audit policy, against fresh old-policy control."""
import argparse
import copy
import json
import os
from pathlib import Path
import random
import subprocess
import sys

import c3_query_priority as c
b=c.b; a=c.a; e=c.e; f=c.f

AUDIT=('## Archive review before finalizing a complete-history answer\n'
    'Do not finalize by silently dropping the difficult candidates. For each requested value, distinguish '
    '(1) directly confirmed by visible original text or a retrieved original record, '
    '(2) checked against the available original archive without confirming evidence, and '
    '(3) not checked because it is merely absent from the projection or its summary. '
    'State (3) is an unresolved evidence gap, not a negative finding. '
    'When history tools are available and lookup allowance remains, move unresolved candidates out of state (3) by issuing targeted history_search requests before finalizing. '
    'Determine the unresolved candidates yourself from the question and evidence; no answer key is supplied. '
    'For a literal occurrence question, begin with the candidate literal and limit=1; do not use regex OR with history_search. '
    'Independent searches may be issued together in one response. A returned original snippet can settle a value without a full read. '
    'Use history_get only if context or pagination is needed, and never invent an entry ID. '
    'Do not query a value already settled by original evidence, and do not repeat a completed lookup without a specific unresolved ambiguity. '
    'If the tools are unavailable or the budget ends, answer conservatively from confirmed evidence. '
    'Return the requested final JSON, including all already confirmed values; do not print this bookkeeping or replace the complete answer with only new findings. '
    'This is an evidence-completeness requirement, not a target number of calls: do not make arbitrary or duplicate calls merely to increase usage.')


def make_body(original,strong,old_sid,sid):
    body=copy.deepcopy(original)
    assert c.POLICY in body['messages'][0]['content'],'control must already contain previous enhanced policy'
    body['messages'][0]['content']=body['messages'][0]['content'].replace(old_sid,sid)
    if strong: body['messages'][0]['content']+='\n\n'+AUDIT
    assert body['tools']==e.t.FUTURE_TOOLS and body['tool_choice']=='auto'
    assert [x['role'] for x in body['messages']]==['system','user']
    return body


def main():
    parser=argparse.ArgumentParser()
    for key in ('prior','source','output','future','dumper','bridge'): parser.add_argument('--'+key,type=Path,required=True)
    parser.add_argument('--prepare-only',action='store_true'); parser.add_argument('--detach',action='store_true')
    args=parser.parse_args()
    for key in ('prior','source','output','future','dumper','bridge'): setattr(args,key,getattr(args,key).resolve())
    args.output.mkdir(parents=True,exist_ok=True); args.output.chmod(0o700)
    if args.detach:
        if (args.output/'runner.lock').exists(): raise RuntimeError('runner already active')
        with (args.output/'runner.log').open('ab') as log:
            child=subprocess.Popen([sys.executable,'-u',str(Path(__file__).resolve()),*[x for x in sys.argv[1:] if x!='--detach']],stdout=log,stderr=subprocess.STDOUT,cwd=b.REPO,start_new_session=True)
        print(json.dumps({'pid':child.pid})); return
    lock=args.output/'runner.lock'; fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600); os.write(fd,str(os.getpid()).encode()); os.close(fd)
    try:
        previous=b.load(args.prior/'verified-report.json'); old_manifest=b.load(args.prior/'manifest.json'); ledger=b.load(args.prior/'ledger.json')
        assert previous['complete'] and previous['artifact_consistent'] and all(x['state']=='finished' for x in ledger.values())
        reference=Path(old_manifest['prior_root']); rng=random.Random(19379); jobs=[]
        for old in old_manifest['jobs']:
            if not old['policy']: continue
            original=b.load(args.prior/'initial-bodies'/f'{old["id"]}.json')
            assert b.sha(original)==old['initial_body_sha256']
            pair=[]
            for strong in (False,True):
                identity=f'{old["chain"]}-{old["stage"]}-a{int(strong)}'; sid='audit-'+identity
                body=make_body(original,strong,old['session_id'],sid)
                b.immutable(args.output/'initial-bodies'/f'{identity}.json',body)
                pair.append({'id':identity,'policy':strong,'chain':old['chain'],'stage':old['stage'],'step':old['step'],'cut':old['cut'],
                    'session_id':sid,'prior_identity':old['id'],'projection_sha256':old['projection_sha256'],
                    'question_sha256':old['question_sha256'],'initial_body_sha256':b.sha(body)})
            rng.shuffle(pair); jobs.extend(pair)
        assert len(jobs)==36
        config={'version':1,'kind':'C3 archive review prompt extension versus previous enhanced prompt','jobs':jobs,'model':old_manifest['model'],
            'opening_spend':previous['total_spend'],'total_budget':min(300,previous['total_spend']+3),'question_root':str(reference),
            'prior_root':str(args.prior),'prior_ledger_sha256':b.sha(ledger),'prior_manifest_sha256':b.sha(old_manifest),
            'source_root':str(args.source),'audit_policy':AUDIT,'previous_policy':c.POLICY,'allocation_seed':19379,
            'code_hashes':{str(path.relative_to(b.REPO)):b.sha(path.read_bytes()) for path in (Path(__file__),Path(c.__file__),Path(a.__file__),Path(e.__file__),Path(e.t.__file__),Path(f.__file__),Path(b.__file__),b.HERE/'native_stores.py',b.HERE/'native_codex.py')},
            'binaries':{key:{'path':str(getattr(args,key)),'sha256':b.sha(getattr(args,key).read_bytes())} for key in ('future','dumper','bridge')},
            'logical_query_limit':a.QUERY_LIMIT,'byte_stop':a.BYTE_STOP,
            'scope':'prompt requires evidence review before dropping unresolved values; no runtime candidate gate, no forced tool_choice, no prior answer; all18 cases including regressions; tuning not held-out evidence'}
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
                body=b.load(args.output/'initial-bodies'/f'{job["id"]}.json')
                base={key:value for key,value in body.items() if key not in ('tools','tool_choice')}
                answer=a.answer(calls,job['id'],base,e.t.FUTURE_TOOLS,lambda name,arguments:e.future_call(engine,name,arguments),args.output)
                question=b.load(reference/'questions'/f'{job["chain"]}-{job["stage"]}.json')
                score=b.exam.score(answer['answer'],dict.fromkeys(question['present']),dict.fromkeys(question['decoys']))
                b.immutable(dest,dict(answer,**score,**job))
            finally: engine.close()
            c.report(args.output)
        c.report(args.output)
    finally: lock.unlink()


if __name__=='__main__': main()
