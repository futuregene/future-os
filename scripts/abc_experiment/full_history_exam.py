#!/usr/bin/env python3
"""Matched new closed/open questions targeting the complete original history.

Reuses frozen projections/candidates, but generates BOTH closed and open answers
under the same clarified question. No earlier answer enters model context.
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
import guided_open_exam as g
from recall_guidance import Guidance
import nonuse_diagnosis as d
b=a.b; e=a.e; f=a.f; t=a.t

SYSTEM=d.BASE_ALLOWED
QUESTION=('The text above is a compressed projection, not the complete original session. '
    'This question concerns the COMPLETE original session, including records omitted from the projection. '
    'Use the evidence available in this condition. If read-only history or file lookup tools are provided, '
    'the original records they return are permitted evidence; you decide whether and what to query. '
    'Do not treat a value missing from the projection as absent from the original session. '
    'Return one JSON object {"appeared":["..."]} listing only candidate values you can confirm occurred in the original session. '
    'Do not guess; if you still cannot confirm a value, leave it out.')


def question_from(original):
    values=d.candidates(original)
    if not values or len(values)!=len(set(values)): raise ValueError('invalid public candidates')
    return QUESTION+'\nValues:\n'+'\n'.join('- '+value for value in values)


def body_from(original_body,projection,question,guidance=''):
    body=copy.deepcopy(original_body)
    assert 'tools' not in body and 'tool_choice' not in body
    body['messages']=[{'role':'system','content':SYSTEM+('\n\n'+guidance if guidance else '')},
                      {'role':'user','content':projection+'\n\n'+question}]
    return body


def grade(result,question,projection,lookup):
    answer=result['answer']; reported=set(answer.get('appeared',[])) if isinstance(answer,dict) else set()
    available={v for v in question['present'] if v in projection or any(v in text for text in lookup)}
    score=b.exam.score(answer,dict.fromkeys(question['present']),dict.fromkeys(question['decoys']))
    return dict(result,**score,reachable=len(available),reachable_hits=len(reported&available),
                correct_outside_literal_containment=len((reported&set(question['present']))-available))


def report(root):
    config=b.load(root/'manifest.json'); answers={x['identity']:x for x in (b.load(path) for path in (root/'answers').glob('*.json'))}
    expected={job['identity'] for job in config['jobs']}; assert set(answers)<=expected
    result={'complete':set(answers)==expected,'answered':len(answers),'expected':len(expected),'pairs_complete':0,'arms':{},'by_chain':{}}
    fields=('hits','of_present','false_positives','model_turns','logical_queries','returned_bytes','tool_errors','reachable','reachable_hits')
    for arm in b.ARMS:
        result['arms'][arm]={}
        for mode in ('closed','open'):
            selected=[row for row in answers.values() if row['arm']==arm and row['mode']==mode]
            totals={key:sum(row[key] for row in selected) for key in fields}
            totals.update(cases=len(selected),lookup_cases=sum(row['lookup_used'] for row in selected),
                          invalid=sum(not row['valid_answer'] for row in selected))
            result['arms'][arm][mode]=totals
        pair_totals={'pairs':0,'gained':0,'lost':0,'net_gain':0,'new_false_positives':0,'corrected_false_positives':0}
        for chain in config['chains']:
            chain_totals={'pairs':0,'closed_hits':0,'open_hits':0,'of_present':0,'net_gain':0}
            for stage in range(3):
                prefix=f'{chain}-{stage}-{arm}-full'
                first=answers.get(prefix+'-closed'); second=answers.get(prefix+'-open')
                if first is None or second is None: continue
                q=b.load(root/'questions'/f'{chain}-{stage}.json')
                before=set((first['answer'] or {}).get('appeared',[])); after=set((second['answer'] or {}).get('appeared',[]))
                pos=set(q['present']); neg=set(q['decoys'])
                delta={'pair':prefix,'closed_hits':first['hits'],'open_hits':second['hits'],
                    'gained':len((after-before)&pos),'lost':len((before-after)&pos),
                    'net_gain':second['hits']-first['hits'],'new_false_positives':len((after-before)&neg),
                    'corrected_false_positives':len((before-after)&neg),
                    'closed_answer_sha256':b.sha(first),'open_answer_sha256':b.sha(second)}
                assert delta['net_gain']==delta['gained']-delta['lost']
                b.immutable(root/'pairs'/f'{prefix}.json',delta)
                pair_totals['pairs']+=1; result['pairs_complete']+=1
                for key in pair_totals:
                    if key!='pairs': pair_totals[key]+=delta[key]
                chain_totals['pairs']+=1; chain_totals['closed_hits']+=first['hits']; chain_totals['open_hits']+=second['hits']
                chain_totals['of_present']+=first['of_present']; chain_totals['net_gain']+=delta['net_gain']
            result['by_chain'].setdefault(chain,{})[arm]=chain_totals
        result['arms'][arm]['delta']=pair_totals
    ledger=b.load(root/'ledger.json') if (root/'ledger.json').exists() else {}
    result['new_spend']=sum(row.get('charged',row['reserved']) for row in ledger.values())
    result['total_spend']=config['opening_spend']+result['new_spend']
    b.save(root/'report.json',result); print(json.dumps({'answered':len(answers),'pairs':result['pairs_complete'],'total':result['total_spend']}),flush=True)
    return result


def main():
    parser=argparse.ArgumentParser()
    for key in ('source','prior','output','future','dumper','bridge','codex-source','opencode-source'):
        parser.add_argument('--'+key,type=Path,required=True)
    parser.add_argument('--prepare-only',action='store_true'); parser.add_argument('--detach',action='store_true')
    args=parser.parse_args()
    for key in ('source','prior','output','future','dumper','bridge','codex_source','opencode_source'):
        setattr(args,key,getattr(args,key).resolve())
    args.output.mkdir(parents=True,exist_ok=True); args.output.chmod(0o700)
    if args.detach:
        if (args.output/'runner.lock').exists(): raise RuntimeError('runner already active')
        with (args.output/'runner.log').open('ab') as log:
            child=subprocess.Popen([sys.executable,'-u',str(Path(__file__).resolve()),*[x for x in sys.argv[1:] if x!='--detach']],cwd=b.REPO,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        print(json.dumps({'pid':child.pid})); return
    lock=args.output/'runner.lock'; fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600); os.write(fd,str(os.getpid()).encode()); os.close(fd)
    try:
        previous=b.load(args.prior/'verified-report.json'); prior_ledger=b.load(args.prior/'ledger.json')
        assert previous['complete'] and previous['artifact_consistent'] and all(row['state']=='finished' for row in prior_ledger.values())
        source=b.load(args.source/'fidelity-manifest.json'); assert b.load(args.source/'verified-report.json')['verified']
        assert b.sha(args.bridge.read_bytes())==source['bridge_sha256'] and b.sha(args.dumper.read_bytes())==source['driver_sha256']
        upstream={'codex_source':'b13164d86f9a70adc48d22f4a5a07ed0c001a1d0','opencode_source':'e03db9bc6908f75c9334d8aa997deeaac81c0298'}
        for key,sha in upstream.items():
            assert subprocess.check_output(['git','-C',str(getattr(args,key)),'rev-parse','HEAD'],text=True).strip()==sha
            assert not subprocess.check_output(['git','-C',str(getattr(args,key)),'status','--porcelain'],text=True).strip()
        guides=Guidance(b.REPO,args.codex_source,args.opencode_source)
        for key,text in guides.source.items(): b.immutable(args.output/'guidance-sources'/f'{key}.json',{'text':text})
        b.immutable(args.output/'guidance-adaptations.json',guides.adaptations())
        ledger=b.load(args.source/'ledger.json'); index={row['identity']:key for key,row in ledger.items()}
        schedule=b.load(args.source/'schedule.json'); rng=random.Random(19331); jobs=[]
        for chain in source['chains']:
            data=b.load(args.source/'corpus'/f'{chain}.json')
            for stage,cut in enumerate(data['cuts']):
                old_question=b.load(args.source/'questions'/f'{chain}-{stage}.json')
                new_question=dict(old_question,prompt=question_from(old_question['prompt']))
                assert d.candidates(new_question['prompt'])==d.candidates(old_question['prompt'])
                b.immutable(args.output/'questions'/f'{chain}-{stage}.json',new_question)
                block=[]
                for arm in b.ARMS:
                    step=schedule[chain].index(cut); projection=b.load(args.source/'projections'/f'{chain}-{step}-{arm}-compact.json')
                    original=b.load(args.source/'requests'/f'{index[f"{chain}-{stage}-{arm}-closed"]}.json')['body']
                    assert original['model']==source['model'] and original['max_tokens']==8192 and original['thinking']=={'type':'disabled'}
                    sid=f'full-{chain}-{stage}-{arm}'
                    has_checkpoint=bool(projection.get('checkpoint')) if arm in ('C','C3') else True
                    guidance=guides.text(arm,sid,has_checkpoint)
                    base=body_from(original,projection['text'],new_question['prompt'])
                    opened=a.open_body(body_from(original,projection['text'],new_question['prompt'],guidance),a.schemas(arm))
                    g.assert_guidance_parity(base,opened,a.schemas(arm),guidance)
                    b.immutable(args.output/'guidance'/f'{chain}-{stage}-{arm}.json',{'text':guidance,'has_checkpoint':has_checkpoint,'session_id':sid})
                    for mode,body in (('closed',base),('open',opened)):
                        identity=f'{chain}-{stage}-{arm}-full-{mode}'
                        b.immutable(args.output/'initial-bodies'/f'{identity}.json',body)
                        block.append({'identity':identity,'chain':chain,'stage':stage,'arm':arm,'mode':mode,'cut':cut,'step':step,
                            'session_id':sid,'projection_sha256':b.sha(projection['text']),'question_sha256':b.sha(new_question),
                            'initial_body_sha256':b.sha(body),'original_question_sha256':b.sha(old_question)})
                rng.shuffle(block); jobs.extend(block)
        config={'version':1,'design':'new matched closed/open full-original-history questions; independent conversations',
            'model':source['model'],'chains':source['chains'],'jobs':jobs,'base_system':SYSTEM,'question_template':QUESTION,
            'opening_spend':previous['total_spend'],'budget':300,'prior_root':str(args.prior),'prior_ledger_sha256':b.sha(prior_ledger),
            'source_root':str(args.source),'source_manifest_sha256':b.sha(source),'source_ledger_sha256':b.sha(ledger),
            'code_hashes':{str(path.relative_to(b.REPO)):b.sha(path.read_bytes()) for path in (Path(__file__),Path(a.__file__),Path(e.__file__),Path(g.__file__),Path(d.__file__),Path(t.__file__),Path(f.__file__),Path(b.__file__),b.HERE/'recall_guidance.py',b.HERE/'native_stores.py',b.HERE/'native_codex.py')},
            'guidance_sources':{key:{'path':str(path),'sha256':b.sha(path.read_bytes())} for key,path in guides.paths.items()},
            'guidance_adaptations_sha256':b.sha(guides.adaptations()),'upstream_commits':upstream,
            'binaries':{key:{'path':str(getattr(args,key)),'sha256':b.sha(getattr(args,key).read_bytes())} for key in ('future','dumper','bridge')},
            'logical_query_limit':a.QUERY_LIMIT,'byte_stop':a.BYTE_STOP,'allocation_seed':19331,
            'constraints':'same new question/candidates/projection in each pair; open only adds recorded functional guidance/tools/auto; no closed answer or checklist injected; zero-tool results kept'}
        b.immutable(args.output/'manifest.json',config)
        b.immutable(args.output/'request-parity.json',{'pairs':72,'bodies':144,'all_passed':True,'same_question_and_projection':True,
            'new_closed_answers_required':True,'prior_answer_injected':False,'pending_list_injected':False,'forced_tool_choice':False,
            'open_differences':['source-derived guidance appended to system','tools','tool_choice:auto']})
        if args.prepare_only: return
        calls=a.Calls(args.output,300,source['model'],args.bridge,b.sha(config),previous_spend=config['opening_spend'])
        for job in jobs:
            identity=job['identity']; dest=args.output/'answers'/f'{identity}.json'
            if dest.exists(): assert b.load(dest)['initial_body_sha256']==job['initial_body_sha256']; continue
            if any(row['identity'].startswith(identity+'-turn') for row in calls.rows.values()): raise RuntimeError('partial case: no silent retry')
            body=b.load(args.output/'initial-bodies'/f'{identity}.json'); engine=None; lookup=[]
            try:
                if job['mode']=='closed':
                    payload=calls.request(identity+'-turn0',body); assert not payload.get('calls'),'closed condition emitted tool calls'
                    answer=b.parse_answer(payload['text'])
                    result={'answer':answer,'valid_answer':answer is not None,'finish':payload.get('finish'),'logical_queries':0,
                            'returned_bytes':0,'model_turns':1,'lookup_used':False,'tool_errors':0}
                else:
                    records=b.load(args.source/'corpus'/f'{job["chain"]}.json')['records'][:job['cut']]
                    arm=job['arm']
                    if arm in ('C','C3'):
                        messages=f.normalize(args.output,args.dumper,source['model'],records); case=args.output/'cases'/identity
                        engine=e.NativeFuture(args.future,case/'workspace',case/'home',case/'control',messages,job['session_id'])
                        b.immutable(args.output/'database-proofs'/f'{identity}.json',e.database_proof(engine,messages))
                        dispatch=lambda name,arguments:e.future_call(engine,name,arguments); lookup=[t.body(row) for row in records]
                    elif arm=='codex':
                        backend=t.CodexHistory(records,[end for end in schedule[job['chain']] if end<=job['cut']])
                        dispatch=backend.call; lookup=[row['content'] for row in backend.flat]
                    else:
                        backend=t.ObservedFiles(records,args.output/'file-fixtures'/f'{job["chain"]}-{job["stage"]}')
                        dispatch=backend.call; lookup=list(backend.files)+list(backend.files.values())
                        b.immutable(args.output/'file-visibility'/f'{identity}.json',{'paths':list(backend.files),'content_sha256':b.sha(backend.files),
                            'invalidated':backend.invalidated,'unmapped_results':backend.unmapped_results})
                    base={key:value for key,value in body.items() if key not in ('tools','tool_choice')}
                    result=a.answer(calls,identity,base,a.schemas(arm),dispatch,args.output)
                q=b.load(args.output/'questions'/f'{job["chain"]}-{job["stage"]}.json')
                projection=b.load(args.source/'projections'/f'{job["chain"]}-{job["step"]}-{job["arm"]}-compact.json')['text']
                b.immutable(dest,dict(grade(result,q,projection,lookup),**job))
            finally:
                if engine: engine.close()
            report(args.output)
        report(args.output)
    finally: lock.unlink()


if __name__=='__main__': main()
