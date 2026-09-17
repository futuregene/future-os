#!/usr/bin/env python3
"""Fresh C3 control/policy pairs on all 18 cases; never force a call count."""
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

POLICY=('## Complete-history evidence discipline\n'
    'For a request covering the complete original conversation, completeness matters as well as precision. '
    'A fluent handoff summary and a head/tail evidence index are not a complete searchable copy of the archive. '
    'Their silence about a candidate is not negative evidence. Do not silently turn a full-history question into a list of only the values visible in the projection. '
    'Keep values directly supported by visible original text without re-reading them. '
    'Before omitting an unresolved historical value merely because it is missing from the projection or summary, use history_search with that literal value or a distinctive related keyword. '
    'Read a returned entry only when its snippet is insufficient, and refine a query if needed. '
    'A query echo is not evidence, and an empty narrow lookup does not establish absence from every original record. '
    'Once you have enough evidence, stop searching and return the complete requested answer, including facts already supported before lookup, not just newly found facts. '
    'If the tools or available records cannot establish a value, leave it unconfirmed rather than guessing. '
    'Choose the relevant queries yourself; there is no target number of calls and no reason to repeat settled lookups.')


def make_body(original,policy,old_sid,sid):
    body=copy.deepcopy(original)
    body['messages'][0]['content']=body['messages'][0]['content'].replace(old_sid,sid)
    if policy: body['messages'][0]['content']+='\n\n'+POLICY
    assert [m['role'] for m in body['messages']]==['system','user']
    assert body['tools']==e.t.FUTURE_TOOLS and body['tool_choice']=='auto'
    return body


def report(root):
    config=b.load(root/'manifest.json'); rows=[b.load(p) for p in (root/'results').glob('*.json')]
    ledger=b.load(root/'ledger.json') if (root/'ledger.json').exists() else {}
    out={'complete':len(rows)==len(config['jobs']),'cases':len(rows),'groups':{}}
    for policy in (False,True):
        selected=[row for row in rows if row['policy']==policy]
        totals={key:sum(row[key] for row in selected) for key in ('hits','of_present','false_positives','logical_queries','returned_bytes','tool_errors','model_turns')}
        totals.update(cases=len(selected),lookup_cases=sum(row['lookup_used'] for row in selected),
            invalid=sum(not row['valid_answer'] for row in selected),
            real_hits=sum(row['hits'] for row in selected if row['chain'].startswith('real-')),
            real_lookup_cases=sum(row['lookup_used'] for row in selected if row['chain'].startswith('real-')))
        out['groups']['policy' if policy else 'control']=totals
    out['new_spend']=sum(row.get('charged',row['reserved']) for row in ledger.values())
    out['total_spend']=config['opening_spend']+out['new_spend']
    b.save(root/'report.json',out); print(json.dumps(out,ensure_ascii=False),flush=True)
    return out


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
            child=subprocess.Popen([sys.executable,'-u',str(Path(__file__).resolve()),*[v for v in sys.argv[1:] if v!='--detach']],stdout=log,stderr=subprocess.STDOUT,cwd=b.REPO,start_new_session=True)
        print(json.dumps({'pid':child.pid})); return
    lock=args.output/'runner.lock'; fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600); os.write(fd,str(os.getpid()).encode()); os.close(fd)
    try:
        previous=b.load(args.prior/'verified-report.json'); original_manifest=b.load(args.prior/'manifest.json'); ledger=b.load(args.prior/'ledger.json')
        assert previous['complete'] and previous['artifact_consistent'] and all(row['state']=='finished' for row in ledger.values())
        jobs=[]; rng=random.Random(19357)
        for original in original_manifest['jobs']:
            if original['arm']!='C3' or original['mode']!='open': continue
            initial=b.load(args.prior/'initial-bodies'/f'{original["identity"]}.json')
            assert b.sha(initial)==original['initial_body_sha256']
            pair=[]
            for policy in (False,True):
                identity=original['chain']+'-'+str(original['stage'])+'-p'+str(int(policy)); sid='priority-'+identity
                body=make_body(initial,policy,original['session_id'],sid)
                b.immutable(args.output/'initial-bodies'/f'{identity}.json',body)
                pair.append({'id':identity,'policy':policy,'chain':original['chain'],'stage':original['stage'],
                    'step':original['step'],'cut':original['cut'],'session_id':sid,'source_identity':original['identity'],
                    'projection_sha256':original['projection_sha256'],'question_sha256':original['question_sha256'],'initial_body_sha256':b.sha(body)})
            rng.shuffle(pair); jobs.extend(pair)
        assert len(jobs)==36
        config={'version':1,'kind':'C3 completeness-priority optimization, fresh paired control and policy','jobs':jobs,
            'model':original_manifest['model'],'opening_spend':previous['total_spend'],'total_budget':min(300,previous['total_spend']+3),
            'prior_root':str(args.prior),'prior_ledger_sha256':b.sha(ledger),'prior_manifest_sha256':b.sha(original_manifest),
            'source_root':str(args.source),'policy':POLICY,'allocation_seed':19357,
            'code_hashes':{str(path.relative_to(b.REPO)):b.sha(path.read_bytes()) for path in (Path(__file__),Path(a.__file__),Path(e.__file__),Path(e.t.__file__),Path(f.__file__),Path(b.__file__),b.HERE/'native_stores.py',b.HERE/'native_codex.py')},
            'binaries':{key:{'path':str(getattr(args,key)),'sha256':b.sha(getattr(args,key).read_bytes())} for key in ('future','dumper','bridge')},
            'logical_query_limit':a.QUERY_LIMIT,'byte_stop':a.BYTE_STOP,
            'scope':'all 18 C3 cases; system-only policy, identical questions/tools, auto/no draft/no generated checklist; tuning result not a new four-arm ranking'}
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
                result=a.answer(calls,job['id'],base,e.t.FUTURE_TOOLS,lambda name,arguments:e.future_call(engine,name,arguments),args.output)
                q=b.load(args.prior/'questions'/f'{job["chain"]}-{job["stage"]}.json')
                score=b.exam.score(result['answer'],dict.fromkeys(q['present']),dict.fromkeys(q['decoys']))
                b.immutable(dest,dict(result,**score,**job))
            finally: engine.close()
            report(args.output)
        report(args.output)
    finally: lock.unlink()


if __name__=='__main__': main()
