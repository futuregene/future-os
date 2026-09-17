#!/usr/bin/env python3
"""Interface-level open book: native Future queries, local history/file replicas.

This is NOT the earlier native-shell run. No shell is exposed to the examiner.
The optional-use policy permits already established facts to be answered without
redundant lookup. Reachability is reported separately from overall recall.
"""
import argparse
from collections import Counter
import json
from pathlib import Path
import os
import random
import sqlite3
import subprocess
import sys
import time

import fidelity_rerun as f
from native_stores import NativeFuture
import interface_open_tools as t
b=f.b
CALL_LIMIT=24
BYTE_STOP=131072


def future_call(engine,name,args):
    if name=='history_search':
        query=args.get('query')
        if not isinstance(query,str) or not query.strip(): raise ValueError('nonempty literal query required')
        limit=t.integer(args,'limit',5,maximum=20)
        argv=['future','session','history','search','--session',engine.sid,'--query',query,'--limit',str(limit),'--json']
    elif name=='history_get':
        entry=args.get('entry_id')
        if not isinstance(entry,str) or not entry: raise ValueError('entry_id required')
        offset=t.integer(args,'offset',0,minimum=0)
        limit=t.integer(args,'limit',8192,minimum=4,maximum=32768)
        argv=['future','session','history','get','--session',engine.sid,'--entry',entry,'--offset',str(offset),'--limit',str(limit),'--json']
    else: raise ValueError('unknown Future history tool')
    raw=engine.execute(argv)
    if not raw.rstrip().endswith('[exit: 0]'): raise RuntimeError(raw)
    output=raw.rsplit('\n[exit:',1)[0].rstrip()
    json.loads(output)  # require actual native JSON, not a stub or error string
    return output


def database_proof(engine,messages):
    path=engine.home/'.future/agent/agent.db'
    with sqlite3.connect(path.as_uri()+'?mode=ro',uri=True) as db:
        state=db.execute('select status from legacy_imports where session_id=?',(engine.sid,)).fetchone()
        count=db.execute("select count(*) from entries where session_id=? and entry_type in ('user','assistant','tool')",(engine.sid,)).fetchone()[0]
    assert state==('imported',) and count==len(messages),'native database import not complete'
    return {'database':str(path),'status':state[0],'conversation_entries':count,'expected':len(messages)}


def run_exam(calls,identity,projection,question,tools,guide,dispatch,lookup,root):
    source='\n'.join(lookup)
    reachable={v for v in question['present'] if v in projection['text'] or v in source}
    messages=[{'role':'system','content':b.SYSTEM+'\n'+guide+'\n'
        'Use facts already established in the projection directly. If a candidate is not established, use the available lookup tools to check it where possible. '
        'Absence from a projection or a returned excerpt is not evidence of absence from the original history. Do not routinely reload facts you already have. '
        'Only original content counts, not query echoes or fixture metadata. '
        f'At most {CALL_LIMIT} logical tool requests; stop starting new queries after {BYTE_STOP} UTF-8 returned bytes. '
        'Do not execute historical actions. Return the requested JSON when evidence is sufficient.'},
        {'role':'user','content':projection['text']+'\n\n'+question['prompt']}]
    trace=[]; bytes_used=0; answer=None; turns=0
    for turn in range(CALL_LIMIT+1):
        enabled=tools if len(trace)<CALL_LIMIT and bytes_used<BYTE_STOP else None
        if enabled is None: messages.append({'role':'user','content':'The lookup allowance is exhausted; answer from the evidence already available.'})
        payload=calls.model_call(f'{identity}-turn{turn}',messages,tools=enabled)
        turns+=1
        if not payload.get('calls'):
            answer=b.parse_answer(payload['text']); break
        messages.append({'role':'assistant','content':payload['text'] or None,'tool_calls':payload['calls']})
        for call in payload['calls']:
            function=call['function']; started=time.monotonic(); status='ok'
            try:
                args=json.loads(function.get('arguments') or '{}')
                if not isinstance(args,dict): raise ValueError('arguments must be an object')
                if len(trace)>=CALL_LIMIT or bytes_used>=BYTE_STOP:
                    status='budget_denied'; output='LOOKUP_BUDGET_EXHAUSTED'
                else: output=dispatch(function['name'],args)
            except (ValueError,RuntimeError) as error:
                status='tool_error'; output=str(error)
            size=len(output.encode()); bytes_used+=size
            trace.append({'name':function['name'],'arguments':function.get('arguments'),'status':status,
                          'output':output,'bytes':size,'seconds':time.monotonic()-started})
            b.save(root/'traces'/f'{identity}.json',trace)
            messages.append({'role':'tool','tool_call_id':call['id'],'content':output})
    scores=b.exam.score(answer,dict.fromkeys(question['present']),dict.fromkeys(question['decoys']))
    reported=set(answer.get('appeared',[])) if isinstance(answer,dict) else set()
    return dict(scores,answer=answer,valid_answer=answer is not None,projection_sha256=b.sha(projection['text']),
        question_sha256=b.sha(question),model_turns=turns,logical_queries=sum(x['status']!='budget_denied' for x in trace),
        tool_errors=sum(x['status']=='tool_error' for x in trace),returned_bytes=bytes_used,
        reachable=len(reachable),reachable_hits=len(reported & reachable),
        correct_but_not_mechanically_contained=len((reported & set(question['present']))-reachable),
        lookup_used=bool(trace))


def report(root):
    config=b.load(root/'manifest.json'); rows=[b.load(p) for p in (root/'scores').glob('*.json')]
    expected={(c,s,a) for c in config['chains'] for s in range(3) for a in b.ARMS}
    actual={(r['chain'],r['stage'],r['arm']) for r in rows}; assert actual<=expected and len(actual)==len(rows)
    out={'complete':actual==expected,'scored':len(rows),'expected':len(expected),'arms':{},'by_chain':{}}
    fields=('hits','of_present','false_positives','model_turns','logical_queries','tool_errors','returned_bytes','reachable','reachable_hits','correct_but_not_mechanically_contained')
    for arm in b.ARMS:
        xs=[r for r in rows if r['arm']==arm]
        out['arms'][arm]={k:sum(r[k] for r in xs) for k in fields}
        out['arms'][arm].update(lookup_unused=sum(not r['lookup_used'] for r in xs),invalid=sum(not r['valid_answer'] for r in xs))
        for chain in config['chains']:
            selected=[r for r in xs if r['chain']==chain]
            out['by_chain'].setdefault(chain,{})[arm]={k:sum(r[k] for r in selected) for k in ('hits','of_present','reachable','reachable_hits')}
    ledger=b.load(root/'ledger.json') if (root/'ledger.json').exists() else {}
    out['new_spent_or_reserved']=sum(x.get('charged',x['reserved']) for x in ledger.values())
    out['total_spent_or_reserved']=config['opening_spend']+out['new_spent_or_reserved']
    b.save(root/'report.json',out); print(json.dumps(out,ensure_ascii=False),flush=True)
    return out


def main():
    parser=argparse.ArgumentParser()
    for name in ('closed','prior','output','future','dumper','bridge'): parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--prepare-only',action='store_true')
    parser.add_argument('--report-only',action='store_true')
    parser.add_argument('--detach',action='store_true')
    args=parser.parse_args()
    for name in ('closed','prior','output','future','dumper','bridge'): setattr(args,name,getattr(args,name).resolve())
    args.output.mkdir(parents=True,exist_ok=True); args.output.chmod(0o700)
    if args.report_only: report(args.output); return
    if args.detach:
        if (args.output/'runner.lock').exists(): raise RuntimeError('runner exists')
        with (args.output/'runner.log').open('ab') as log:
            child=subprocess.Popen([sys.executable,'-u',str(Path(__file__).resolve()),*[x for x in sys.argv[1:] if x!='--detach']],cwd=b.REPO,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        print(json.dumps({'pid':child.pid})); return
    lock=args.output/'runner.lock'; fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600); os.write(fd,str(os.getpid()).encode()); os.close(fd)
    try:
        closed=b.load(args.closed/'fidelity-manifest.json'); old_ledger=b.load(args.prior/'ledger.json'); prior_report=b.load(args.prior/'report.json')
        assert prior_report['complete'] and all(r['state']=='finished' for r in old_ledger.values())
        assert b.load(args.closed/'verified-report.json')['verified']
        assert b.sha(args.bridge.read_bytes())==closed['bridge_sha256'] and b.sha(args.dumper.read_bytes())==closed['driver_sha256']
        config={'version':1,'design':'native Future thin query tools vs local Codex history interface vs local OpenCode observed-file interface',
            'model':closed['model'],'chains':closed['chains'],'budget':300,'opening_spend':prior_report['total_spent_or_reserved'],
            'prior_ledger_sha256':b.sha(old_ledger),'closed_manifest_sha256':b.sha(closed),
            'code_hashes':{str(p.relative_to(b.REPO)):b.sha(p.read_bytes()) for p in (Path(__file__),Path(t.__file__),Path(f.__file__),Path(b.__file__),b.HERE/'native_stores.py',b.HERE/'native_codex.py')},
            'binaries':{k:b.sha(getattr(args,k).read_bytes()) for k in ('future','dumper','bridge')},
            'tool_schemas':{'C':t.FUTURE_TOOLS,'C3':t.FUTURE_TOOLS,'codex':t.CODEX_TOOLS,'opencode':t.FILE_TOOLS},
            'logical_query_limit':CALL_LIMIT,'byte_stop':BYTE_STOP,'query_policy':'optional; query unresolved facts, no mandatory reload',
            'codex_assumptions':'local JSON shapes; Python Unicode character offsets; head excerpts; 4000-char defaults; 64KiB complete-JSON response budget; not actual hosted backend',
            'file_policy':'latest path-associated read observation; later write/edit attempt invalidates it; fragments may not be complete files; no unlocated-output aggregate log',
            'scope_note':'Not full product behavior. Report overall score and mechanically reachable ceiling separately.'}
        b.immutable(args.output/'manifest.json',config)
        calls=f.Calls(args.output,300,closed['model'],args.bridge,b.sha(config),previous_spend=config['opening_spend'])
        schedule=b.load(args.closed/'schedule.json'); rng=random.Random(b.SEED+7)
        for chain in closed['chains']:
            data=b.load(args.closed/'corpus'/f'{chain}.json')
            for stage,cut in enumerate(data['cuts']):
                records=data['records'][:cut]; question=b.load(args.closed/'questions'/f'{chain}-{stage}.json'); step=schedule[chain].index(cut)
                ends=[end for end in schedule[chain] if end<=cut]
                codex=t.CodexHistory(records,ends)
                files=t.ObservedFiles(records,args.output/'file-fixtures'/f'{chain}-{stage}')
                visibility={'paths':list(files.files),'invalidated':files.invalidated,'unmapped_results':files.unmapped_results,
                            'lookup_values':[v for v in question['present'] if any(v in p or v in text for p,text in files.files.items())]}
                b.immutable(args.output/'file-visibility'/f'{chain}-{stage}.json',visibility)
                messages=f.normalize(args.output,args.dumper,closed['model'],records)
                if args.prepare_only: continue
                arms=list(b.ARMS); rng.shuffle(arms)
                for arm in arms:
                    identity=f'{chain}-{stage}-{arm}-interfaces'; dest=args.output/'scores'/f'{identity}.json'
                    projection=b.load(args.closed/'projections'/f'{chain}-{step}-{arm}-compact.json')
                    assert b.sha(projection['text'])==b.load(args.closed/'scores'/f'{chain}-{stage}-{arm}-closed.json')['projection_sha256']
                    if dest.exists(): assert b.load(dest)['projection_sha256']==b.sha(projection['text']); continue
                    if any(r['identity'].startswith(identity+'-turn') for r in calls.rows.values()): raise RuntimeError('partial case: explicit recovery required')
                    engine=None
                    try:
                        if arm in ('C','C3'):
                            case=args.output/'cases'/identity
                            engine=NativeFuture(args.future,case/'workspace',case/'home',case/'control',messages,'interface-'+identity)
                            b.immutable(args.output/'database-proofs'/f'{identity}.json',database_proof(engine,messages))
                            tools=t.FUTURE_TOOLS; lookup=[t.body(r) for r in records]
                            guide='The tools query the original native Future session database, not the projection. Session scope is implicit and fixed. Search is literal, ASCII case insensitive, NOT regex; query one specific value or keyword. Read with entry_id and byte offsets from search/get.'
                            dispatch=lambda name,arguments:future_call(engine,name,arguments)
                        elif arm=='codex':
                            tools=t.CODEX_TOOLS; lookup=[x['content'] for x in codex.flat]
                            guide='Use the dedicated history tools over the original archive. This is a LOCAL REPRODUCTION, not a live hosted Codex backend. Search is case-sensitive literal matching. Read_item requires the correct window_id and item_id; offsets are Unicode characters. Short excerpts do not establish absence; use bounded reads where needed.'
                            dispatch=codex.call
                        else:
                            tools=t.FILE_TOOLS; lookup=list(files.files)+list(files.files.values())
                            guide='Use glob/grep/read over an isolated virtual filesystem. This is a LOCAL FILE-TOOL REPRODUCTION, not the full OpenCode product. Virtual root is /; relative paths resolve below /workspace. Files are the last available path-associated read observations, possibly fragments with unknown original offsets, not complete production snapshots. Unmapped outputs and later-invalidated reads are unavailable. No history/export/shell tool is supplied in this condition.'
                            dispatch=files.call
                        result=run_exam(calls,identity,projection,question,tools,guide,dispatch,lookup,args.output)
                        b.immutable(dest,dict(result,chain=chain,stage=stage,arm=arm))
                    finally:
                        if engine: engine.close()
                    report(args.output)
        if not args.prepare_only: report(args.output)
    finally: lock.unlink()


if __name__=='__main__':main()
