#!/usr/bin/env python3
"""Autonomous open book with audited, source-derived functional recall guidance.

No prior answer or query checklist; never required tool_choice. Reuses unchanged
interface backends and the autonomous tool loop. Source adaptations are explicit.
"""
import argparse
import copy
import json
import math
import os
from pathlib import Path
import random
import subprocess
import sys

import autonomous_open_exam as a
import production_shape as ps
from recall_guidance import Guidance
b=a.b; e=a.e; f=a.f; t=a.t


def guided_base(closed_body,guidance):
    body=copy.deepcopy(closed_body)
    # Production's guidance begins with its own separator, so concatenating it directly keeps
    # the appended text byte-exact. Adding another one here would leave four newlines where
    # the session has two.
    if guidance:
        if not guidance.startswith('\n'):
            guidance='\n\n'+guidance
        body['messages'][0]['content']+=guidance
    return body


def assert_guidance_parity(closed_body,body,tools,guidance):
    a.assert_initial_parity(guided_base(closed_body,guidance),body,tools)
    assert body['messages'][1:]==closed_body['messages'][1:]


def account_for_later_runs(opening,paths):
    """Chain completed ledgers without dropping costs between comparison runs."""
    total=opening; receipts=[]; seen=set()
    for value in paths:
        root=Path(value).resolve()
        if root in seen: raise ValueError('duplicate later-run ledger')
        seen.add(root)
        if (root/'runner.lock').exists(): raise ValueError('later run is still active')
        manifest=b.load(root/'manifest.json'); report_=b.load(root/'verified-report.json'); ledger=b.load(root/'ledger.json')
        if not report_.get('complete') or not report_.get('artifact_consistent'):
            raise ValueError('later run is not verified complete')
        if not all(row['state']=='finished' for row in ledger.values()): raise ValueError('unsettled later-run ledger')
        if not math.isclose(manifest['opening_spend'],total,rel_tol=0,abs_tol=1e-8):
            raise ValueError('later-run budget chain is discontinuous')
        spend=sum(row.get('charged',row['reserved']) for row in ledger.values())
        reported=report_.get('total_spent_or_reserved',report_.get('total_spend'))
        if reported is None or not math.isclose(reported,total+spend,rel_tol=0,abs_tol=1e-8):
            raise ValueError('later-run cost does not match its ledger')
        receipts.append({'root':str(root),'opening_spend':total,'cost':spend,'ledger_sha256':b.sha(ledger),
                         'manifest_sha256':b.sha(manifest),'verified_report_sha256':b.sha(report_)})
        total+=spend
    return total,receipts


def report(root):
    result=a.report(root)
    manifest=b.load(root/'manifest.json')
    unguided=b.load(Path(manifest['prior_root'])/'verified-report.json')
    for arm in b.ARMS:
        result['arms'][arm]['unguided_open_hits']=unguided['arms'][arm]['hits']
        result['arms'][arm]['change_from_unguided']=result['arms'][arm]['hits']-unguided['arms'][arm]['hits']
    b.save(root/'report.json',result)
    return result


def main():
    parser=argparse.ArgumentParser()
    for key in ('closed','prior','output','future','dumper','bridge','codex-source','opencode-source',
                'shape-probe','base-prompt'):
        parser.add_argument('--'+key,type=Path,required=True)
    parser.add_argument('--prepare-only',action='store_true'); parser.add_argument('--detach',action='store_true')
    parser.add_argument('--spent-after',type=Path,action='append',default=[],help='verified later-run ledgers in chronological order; preserve all intervening costs')
    args=parser.parse_args()
    for key in ('closed','prior','output','future','dumper','bridge','codex_source','opencode_source',
                'shape_probe','base_prompt'):
        setattr(args,key,getattr(args,key).resolve())
    args.output.mkdir(parents=True,exist_ok=True); args.output.chmod(0o700)
    if args.detach:
        if (args.output/'runner.lock').exists(): raise RuntimeError('runner already active')
        with (args.output/'runner.log').open('ab') as log:
            child=subprocess.Popen([sys.executable,'-u',str(Path(__file__).resolve()),*[x for x in sys.argv[1:] if x!='--detach']],cwd=b.REPO,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        print(json.dumps({'pid':child.pid})); return
    lock=args.output/'runner.lock'; fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600); os.write(fd,str(os.getpid()).encode()); os.close(fd)
    try:
        expected_commits={'codex_source':'b13164d86f9a70adc48d22f4a5a07ed0c001a1d0','opencode_source':'e03db9bc6908f75c9334d8aa997deeaac81c0298'}
        for key,sha in expected_commits.items():
            checkout=getattr(args,key)
            assert subprocess.check_output(['git','-C',str(checkout),'rev-parse','HEAD'],text=True).strip()==sha
            assert not subprocess.check_output(['git','-C',str(checkout),'status','--porcelain'],text=True).strip()
        guides=Guidance(b.REPO,args.codex_source,args.opencode_source,
        shape_factory=lambda sid: ps.RequestShape(args.shape_probe,args.base_prompt,sid))
        frozen=b.load(args.closed/'fidelity-manifest.json'); prior=b.load(args.prior/'verified-report.json')
        prior_manifest=b.load(args.prior/'manifest.json'); prior_ledger=b.load(args.prior/'ledger.json')
        assert prior['complete'] and prior['artifact_consistent'] and not prior['prior_answer_injected'] and not prior['forced_tool_choice']
        assert all(row['state']=='finished' for row in prior_ledger.values())
        opening_spend,later_spend=account_for_later_runs(prior['total_spent_or_reserved'],args.spent_after)
        assert b.sha(args.bridge.read_bytes())==frozen['bridge_sha256'] and b.sha(args.dumper.read_bytes())==frozen['driver_sha256']
        source_receipts={key:{'path':str(path),'sha256':b.sha(path.read_bytes())} for key,path in guides.paths.items()}
        for key,text in guides.source.items(): b.immutable(args.output/'source-snapshots'/f'{key}.json',{'text':text})
        b.immutable(args.output/'guidance-adaptations.json',guides.adaptations())
        schedule=b.load(args.closed/'schedule.json'); jobs=[]; gates={}; rng=random.Random(b.SEED+10)
        # Retain the same blocks; shuffle arm order independently within each.
        grouped={}
        for old in prior_manifest['jobs']: grouped.setdefault((old['chain'],old['stage']),[]).append(old)
        for block,original_jobs in grouped.items():
            original_jobs=sorted(original_jobs,key=lambda job:job['arm']); rng.shuffle(original_jobs)
            for old in original_jobs:
                identity=old['identity'].replace('-autonomous','-guided'); arm=old['arm']; chain=old['chain']; stage=old['stage']
                request=b.load(args.closed/'requests'/f'{old["closed_request_key"]}.json')
                projection=b.load(args.closed/'projections'/f'{chain}-{old["step"]}-{arm}-compact.json')
                assert b.sha(request)==old['closed_request_sha256'] and b.sha(projection['text'])==old['projection_sha256']
                has_checkpoint=bool(projection.get('checkpoint')) if arm in ('C','C3') else True
                session_id='guided-'+identity
                text=guides.text(arm,session_id,has_checkpoint)
                initial=a.open_body(guided_base(request['body'],text),a.schemas(arm))
                assert_guidance_parity(request['body'],initial,a.schemas(arm),text)
                b.immutable(args.output/'closed-requests'/f'{identity}.json',request)
                b.immutable(args.output/'guidance'/f'{identity}.json',{'text':text,'has_checkpoint':has_checkpoint,'session_id':session_id})
                b.immutable(args.output/'initial-open-bodies'/f'{identity}.json',initial)
                gates[identity]=has_checkpoint
                jobs.append(dict(old,identity=identity,initial_open_body_sha256=b.sha(initial),guidance_sha256=b.sha(text),session_id=session_id))
        config={'version':2,'design':'autonomous tools plus source-derived functional recall guidance; not supplied-answer revision',
            'model':frozen['model'],'chains':frozen['chains'],'jobs':jobs,'budget':300,'opening_spend':opening_spend,
            'later_spend_receipts':later_spend,
            'prior_root':str(args.prior),'prior_ledger_sha256':b.sha(prior_ledger),'prior_manifest_sha256':b.sha(prior_manifest),
            'closed_manifest_sha256':b.sha(frozen),'closed_ledger_sha256':b.sha(b.load(args.closed/'ledger.json')),
            'code_hashes':{str(path.relative_to(b.REPO)):b.sha(path.read_bytes()) for path in (Path(__file__),b.HERE/'recall_guidance.py',Path(a.__file__),Path(e.__file__),Path(t.__file__),Path(f.__file__),Path(b.__file__),b.HERE/'native_stores.py',b.HERE/'native_codex.py')},
            'guidance_sources':source_receipts,'upstream_commits':expected_commits,'guidance_adaptations_sha256':b.sha(guides.adaptations()),
            'binaries':{key:{'path':str(getattr(args,key)),'sha256':b.sha(getattr(args,key).read_bytes())} for key in ('future','dumper','bridge')},
            'tool_schemas':{arm:a.schemas(arm) for arm in b.ARMS},'logical_query_limit':a.QUERY_LIMIT,'byte_stop':a.BYTE_STOP,
            'initial_request_diff_allowlist':['messages[0].content (append recorded guidance only)','tools','tool_choice:auto'],
            'limitations':'functional guidance adapted to local single-agent/text interfaces; no fabricated hosted thread_hint, model-only private state, Task/Bash/export/media support; inherited lexical scoring and substrate limits'}
        b.immutable(args.output/'manifest.json',config)
        b.immutable(args.output/'request-parity.json',{'cases':len(jobs),'all_passed':True,'unchanged_user_question_and_projection':True,
            'unchanged_generation_settings':True,'changes':config['initial_request_diff_allowlist'],
            'prior_answer_injected':False,'pending_list_injected':False,'forced_tool_choice':False,'checkpoint_gates':gates})
        if args.prepare_only: return
        calls=a.Calls(args.output,300,frozen['model'],args.bridge,b.sha(config),previous_spend=config['opening_spend'])
        for job in jobs:
            identity=job['identity']; chain=job['chain']; stage=job['stage']; arm=job['arm']; dest=args.output/'scores'/f'{identity}.json'
            if dest.exists(): assert b.load(dest)['guidance_sha256']==job['guidance_sha256']; continue
            if any(row['identity'].startswith(identity+'-turn') for row in calls.rows.values()): raise RuntimeError('partial case: no silent retry')
            records=b.load(args.closed/'corpus'/f'{chain}.json')['records'][:job['cut']]
            request=b.load(args.output/'closed-requests'/f'{identity}.json')
            text=b.load(args.output/'guidance'/f'{identity}.json')['text']; base=guided_base(request['body'],text); engine=None
            try:
                if arm in ('C','C3'):
                    messages=f.normalize(args.output,args.dumper,frozen['model'],records); case=args.output/'cases'/identity
                    engine=e.NativeFuture(args.future,case/'workspace',case/'home',case/'control',messages,job['session_id'])
                    b.immutable(args.output/'database-proofs'/f'{identity}.json',e.database_proof(engine,messages))
                    dispatch=lambda name,arguments:e.future_call(engine,name,arguments); lookup=[t.body(row) for row in records]
                elif arm=='codex':
                    backend=t.CodexHistory(records,[end for end in schedule[chain] if end<=job['cut']])
                    dispatch=backend.call; lookup=[row['content'] for row in backend.flat]
                else:
                    backend=t.ObservedFiles(records,args.output/'file-fixtures'/f'{chain}-{stage}')
                    dispatch=backend.call; lookup=list(backend.files)+list(backend.files.values())
                    b.immutable(args.output/'file-visibility'/f'{identity}.json',{'paths':list(backend.files),'content_sha256':b.sha(backend.files),
                        'unmapped_results':backend.unmapped_results,'invalidated':backend.invalidated})
                result=a.answer(calls,identity,base,a.schemas(arm),dispatch,args.output)
                baseline=b.load(args.closed/'scores'/f'{chain}-{stage}-{arm}-closed.json')
                question=b.load(args.closed/'questions'/f'{chain}-{stage}.json')
                projection=b.load(args.closed/'projections'/f'{chain}-{job["step"]}-{arm}-compact.json')['text']
                scored=a.evaluate(result,question,baseline,projection,lookup)
                b.immutable(dest,dict(scored,**job,baseline_sha256=b.sha(baseline)))
            finally:
                if engine: engine.close()
            report(args.output)
        report(args.output)
    finally: lock.unlink()


if __name__=='__main__': main()
