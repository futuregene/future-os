#!/usr/bin/env python3
"""Native local/API history-recovery replay, on the frozen v3 closed projections.

Native products execute retrieval; the only paid LLM is the same examiner as
closed book. No current project files, hosted history emulator, or answer keys
are placed inside a tool-visible case. See NATIVE_OPEN_PROTOCOL.md.
"""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import random
import shlex
import subprocess
import sys
import time
import uuid

import fidelity_rerun as f
import production_shape as ps
from native_codex import NativeCodex
from native_opencode import NativeOpenCode
from native_stores import codex_rollout
from native_future_shell import NativeFutureShell
from native_scope import check_tool
b=f.b
TOOL_LIMIT=12
BYTE_STOP=262144


def future_shape(args,sid):
    """Production's system prompt and tool definitions for this replay's session.

    Taken from the Rust code, not restated here: `--print-request-shape` calls
    `history_recall::system_prompt` and returns `coding_tools()`. The exam used to build a
    hand-written guide and rename the history commands to `history_search(query=...)`, tool
    names no product has; that is what this replaces.
    """
    return ps.RequestShape(args.shape_probe,args.base_prompt,sid)

class NativeExamCalls(f.Calls):
    def model_call(self,identity,messages,cap=b.OUTPUT,tools=None,require_tool=False):
        body={'model':self.model,'messages':messages,'max_tokens':cap,'stream':True,
              'stream_options':{'include_usage':True},'thinking':{'type':'disabled'}}
        if tools:
            body['tools']=tools
            if require_tool: body['tool_choice']='required'
        request={'model':self.model,'body':body}
        reserve=(len(json.dumps(body).encode())*5+cap*20)/1e6
        def parse(stdout):
            payload=b.parse_sse(stdout)
            if payload.get('error'): raise RuntimeError(str(payload['error']))
            return payload,(payload.get('usage') or {}).get('credit_cost')
        return self.execute(identity,request,[str(self.bridge)],reserve,parse,json.dumps(request))


def native_case(args,root,identity,arm,records):
    work=root/'cases'/identity/'workspace'; home=root/'cases'/identity/'home'; control=root/'control'/identity
    for p in (work,home,control): p.mkdir(parents=True,exist_ok=True)
    messages=f.normalize(root,args.dumper,args.model,records)
    if arm in ('C','C3'):
        sid='native-'+identity
        shape=future_shape(args,sid)
        # Describe every tool production installs, through the executor, and check that the
        # two sources of the definitions agree. They both come from `coding_tools()`, so a
        # mismatch means one of them is not what it claims to be.
        engine=NativeFutureShell(args.future,args.future_shell,work,home,control,messages,sid,
                                 tool_names=shape.tool_names())
        tools=engine.tools
        assert tools==shape.tools(),'executor and request-shape tool definitions disagree'
        # The system prompt is production's, verbatim: the captured base prompt plus the
        # recall guidance the runtime appends once a checkpoint exists. A replay starts from
        # a compacted projection, so the checkpoint form is the one a session would send.
        system=shape.system_prompt(has_checkpoint=True)
        assert ps.has_guidance(system,Path(args.base_prompt).read_text()), \
            'the replay system prompt is missing the recall guidance'
        def execute(name,arguments):
            if name not in {t['function']['name'] for t in tools}:
                raise ValueError('STUDY_SCOPE_DENIED: unknown tool')
            execute.native_semantics=None
            result=engine.execute_tool(name,arguments)
            execute.native_semantics={k:result.get(k) for k in ('exit_code','is_soft_fail','terminated_by_signal')}
            return result['output']
    elif arm=='codex':
        sid=str(uuid.uuid5(uuid.NAMESPACE_URL,'native-open/'+identity))
        archive,proof=codex_rollout(messages,sid,work,home,args.decoder)
        b.immutable(control/'native-format-proof.json',proof)
        engine=NativeCodex(args.codex,work,home,control)
        tools=[{'type':'function','function':{k:spec[k] for k in ('name','description','parameters')}}
               for name,(spec,_) in engine.tools.items() if name in ('exec_command','write_stdin')]
        assert {x['function']['name'] for x in tools}=={'exec_command','write_stdin'}
        # Codex's own guidance for its own tools; production for this arm is Codex.
        system=('Archive session ID: '+sid+'. Its native local Codex rollout is '+str(archive)+'. '
                'This third-party/API configuration does NOT provide hosted history/notes tools. '
                'Use the native exec_command tool to read/search this local rollout; write_stdin may poll a running reader. '
                'Only original response_item content is historical evidence; session metadata is synthetic fixture metadata.')
        def execute(name,arguments):
            check_tool(name,arguments,work,home,sid,roots=[work,home/'.codex/sessions'])
            return engine.execute(name,arguments)
    else:
        sid='ses_'+b.sha(identity)[:24]
        engine=NativeOpenCode(args.opencode,args.bun,work,home,control)
        engine.import_session(messages,sid)
        catalog=engine.start_catalog()
        tools=[{'type':'function','function':{'name':t['id'],'description':t['description'],'parameters':t['parameters']}}
               for t in catalog if t['id'] in ('bash','read','grep','glob')]
        assert {x['function']['name'] for x in tools}=={'bash','read','grep','glob'}
        engine.close()  # catalog only; actual execution uses native debug-agent ToolRegistry
        system=('Archive session ID: '+sid+'. Its messages were imported into the native OpenCode session store. '
                'The pinned native `opencode` CLI is on PATH. Use `opencode export '+sid+' > retrieved/archive.json` via bash, then read/search that file. '
                'Export to a regular file avoids the native CLI pipe-drain limitation for large JSON; do not rely on a large direct export pipe. '
                'native grep/read/glob can inspect files, including full-output files saved by native truncation. '
                'Only original message parts are historical evidence; session metadata is synthetic fixture metadata.')
        def execute(name,arguments):
            check_tool(name,arguments,work,home,sid,roots=[work,home/'.local/share/opencode/tool-output'])
            return engine.execute(name,arguments)[0]
    b.immutable(control/'native-tools.json',tools)
    b.save(control/'system-prompt.txt',system)
    return engine,tools,system,execute


def examine(calls,identity,projection,question,tools,system,execute,root):
    # The system prompt is the session's own, from the Rust code. The exam's own framing and
    # its retrieval allowance go in the user turn, which is where a turn's instructions live
    # in production too (the same position the compaction instruction occupies).
    exam_note=('This is a REQUIRED archive-verification exam; the questions below refer to it. '
                 'Before finalizing you MUST use the tools to inspect the original archive. '
                 'For candidate values not established by the projection, query the archive; do not assume omission means absence. '
                 'Exporting a file alone is not checking its contents; read/search the export before answering. '
                 f'You may perform at most {TOOL_LIMIT} retrieval tool calls. Stop starting new calls after '
                 f'{BYTE_STOP} UTF-8 bytes of native output have been delivered. Native outputs are not re-truncated. '
                 'Use noninteractive read-only commands. Shell readers allowed: rg/grep (including -e, -c, -o), jq, cat, ls, head, tail, wc, sort, uniq, numeric sed -n, literal echo/printf, '
                 'and the named archive export command. Read-only pipes, && and semicolon-separated readers are allowed. The sole write exception is native OpenCode export to workspace/retrieved/. No other shell expansion or redirection, '
                 'scripts, network, file mutation, other sessions, or other filesystem locations. '
                 'For exec_command/write_stdin request no more than 10000 output tokens; no interactive TTY. '
                 'Do not perform or repeat any actions described in the historical conversation. '
                 'Prefer literal matching for exact candidate strings; ignore synthetic metadata and tool-command echoes. '
                 'The working tree is empty apart from the native launcher and scratch exports; no historical project snapshot is claimed.')
    messages=[{'role':'system','content':system},
              {'role':'user','content':projection['text']+'\n\n'+exam_note+'\n\n'+question['prompt']}]
    trace=[]; turns=0; delivered=0; answer=None; finish=None
    for turn in range(TOOL_LIMIT+1):
        available=tools if len(trace)<TOOL_LIMIT and delivered<BYTE_STOP and turn<TOOL_LIMIT else None
        if available is None:
            messages.append({'role':'user','content':'The retrieval allowance has ended. Answer now with the requested JSON using only evidence already available.'})
        payload=calls.model_call(f'{identity}-turn{turn}',messages,tools=available,
                                 require_tool=not any(t['status']=='native' for t in trace))
        turns+=1; finish=payload.get('finish')
        if not payload.get('calls'):
            answer=b.parse_answer(payload['text']); break
        messages.append({'role':'assistant','content':payload['text'] or None,'tool_calls':payload['calls']})
        for call in payload['calls']:
            function=call['function']; began=time.monotonic(); status='native'; native_semantics=None
            try:
                arguments=json.loads(function.get('arguments') or '{}')
                if not isinstance(arguments,dict): raise ValueError('tool arguments must be an object')
                if len(trace)>=TOOL_LIMIT or delivered>=BYTE_STOP:
                    output='STUDY_BUDGET_EXHAUSTED: no further retrieval'; status='budget_denied'
                else:
                    output=execute(function['name'],arguments)
                    native_semantics=getattr(execute,'native_semantics',None)
            except ValueError as error:
                output=str(error); status='scope_denied'
            except RuntimeError as error:
                output=str(error); status='native_error'
            size=len(output.encode()); delivered+=size
            entry={'name':function['name'],'arguments':function.get('arguments'),'status':status,
                   'native_semantics':native_semantics,
                   'output':output,'bytes':size,'seconds':time.monotonic()-began}
            trace.append(entry)
            b.save(root/'traces'/f'{identity}.json',trace)
            messages.append({'role':'tool','tool_call_id':call['id'],'content':output})
    result=b.exam.score(answer,dict.fromkeys(question['present']),dict.fromkeys(question['decoys']))
    return dict(result,answer=answer,valid_answer=answer is not None,finish=finish,
        projection_sha256=b.sha(projection['text']),question_sha256=b.sha(question),
        native_invoked=any(t['status']=='native' for t in trace),
        model_turns=turns,tool_calls=len(trace),native_calls=sum(t['status'] in ('native','native_error') for t in trace),
        scope_denied=sum(t['status']=='scope_denied' for t in trace),native_errors=sum(t['status']=='native_error' for t in trace),
        native_nonzero_exits=sum((t.get('native_semantics') or {}).get('exit_code') not in (None,0) for t in trace),
        returned_bytes=delivered)


def report(root):
    config=b.load(root/'native-open-manifest.json'); rows=[b.load(p) for p in (root/'scores').glob('*.json')]
    expected={(c,s,a) for c in config['chains'] for s in range(3) for a in b.ARMS}
    actual={(r['chain'],r['stage'],r['arm']) for r in rows}; assert actual<=expected and len(actual)==len(rows)
    result={'complete':actual==expected,'scored':len(rows),'expected':len(expected),'arms':{},'by_chain':{}}
    for arm in b.ARMS:
        xs=[r for r in rows if r['arm']==arm]
        result['arms'][arm]={k:sum(r[k] for r in xs) for k in ('hits','of_present','false_positives','model_turns','native_calls','scope_denied','native_errors','native_nonzero_exits','returned_bytes')}
        result['arms'][arm]['invalid']=sum(not r['valid_answer'] for r in xs)
        result['arms'][arm]['without_native_invocation']=sum(not r['native_invoked'] for r in xs)
        for chain in config['chains']:
            ys=[r for r in xs if r['chain']==chain]
            result['by_chain'].setdefault(chain,{})[arm]={k:sum(r[k] for r in ys) for k in ('hits','of_present')}
    rows=b.load(root/'ledger.json') if (root/'ledger.json').exists() else {}
    result['new_spent_or_reserved']=sum(r.get('charged',r['reserved']) for r in rows.values())
    result['total_spent_or_reserved']=config['prior_spend']+result['new_spent_or_reserved']
    b.save(root/'report.json',result); print(json.dumps(result,ensure_ascii=False),flush=True)
    return result


def main():
    ap=argparse.ArgumentParser()
    for name in ('closed','output','codex','opencode','bun','future','future-shell','dumper','decoder','bridge'):
        ap.add_argument('--'+name,type=Path,required=True)
    ap.add_argument('--prior-native',type=Path,required=True,help='immutable first-run ledger, counted in total budget')
    ap.add_argument('--shape-probe',type=Path,required=True,help='abc_strategy_probe, for the production system prompt and tool definitions')
    ap.add_argument('--base-prompt',type=Path,required=True,help='a real captured session system prompt (capture_shape.py)')
    ap.add_argument('--prepare-only',action='store_true')
    ap.add_argument('--approve-native-future-scope',action='store_true',help='operator acknowledgment after reviewing full-shell data isolation; not an automatic safety guarantee')
    ap.add_argument('--detach',action='store_true')
    ap.add_argument('--report-only',action='store_true')
    args=ap.parse_args()
    for name in ('closed','output','codex','opencode','bun','future','future-shell','dumper','decoder','bridge','prior_native','shape_probe','base_prompt'): setattr(args,name,getattr(args,name).resolve())
    args.output.mkdir(parents=True,exist_ok=True)
    args.output.chmod(0o700)
    if args.report_only: report(args.output); return
    if not args.prepare_only and not args.approve_native_future_scope:
        raise RuntimeError('Full native Future shell is restored; review data isolation before a new paid run, then explicitly acknowledge --approve-native-future-scope')
    if args.detach:
        if (args.output/'runner.lock').exists(): raise RuntimeError('runner exists')
        with (args.output/'runner.log').open('ab') as log:
            p=subprocess.Popen([sys.executable,'-u',str(Path(__file__).resolve()),*[x for x in sys.argv[1:] if x!='--detach']],cwd=b.REPO,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        print(json.dumps({'pid':p.pid})); return
    lock=args.output/'runner.lock'; fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600); os.write(fd,str(os.getpid()).encode()); os.close(fd)
    try:
        closed=b.load(args.closed/'fidelity-manifest.json'); closed_report=b.load(args.closed/'verified-report.json')
        assert closed_report['verified'] and closed_report['complete']
        assert b.sha(args.bridge.read_bytes())==closed['bridge_sha256']
        assert b.sha(args.dumper.read_bytes())==closed['driver_sha256']
        assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=args.opencode,text=True).strip()=='e03db9bc6908f75c9334d8aa997deeaac81c0298'
        args.model=closed['model']
        prior_native=b.load(args.prior_native/'ledger.json')
        assert not (args.prior_native/'runner.lock').exists() and all(r['state']=='finished' for r in prior_native.values())
        opening_spend=closed_report['total_spent_or_reserved']+sum(r.get('charged',r['reserved']) for r in prior_native.values())
        code=[Path(__file__),b.HERE/'native_codex.py',b.HERE/'native_opencode.py',b.HERE/'native_stores.py',b.HERE/'native_scope.py',b.HERE/'native_future_shell.py',Path(f.__file__),Path(b.__file__)]
        config={'version':3,'scope':'native local/API history-recovery replay, not production filesystem restoration',
            'model':args.model,'chains':closed['chains'],'budget':300,'prior_spend':opening_spend,
            'prior_native_ledger_sha256':b.sha(prior_native),'required_archive_check':True,
            'first_run_assessment':'run-v1 is not qualified: guard rejected routine readers and optional retrieval was mostly unused; all charges retained',
            'closed_manifest_sha256':b.sha(closed),'closed_ledger_sha256':b.sha(b.load(args.closed/'ledger.json')),
            'code_hashes':{str(p.relative_to(b.REPO)):b.sha(p.read_bytes()) for p in code},
            'binaries':{name:b.sha(getattr(args,name).read_bytes()) for name in ('codex','bun','future','future_shell','dumper','decoder','bridge')},
            'future_shell_execution':'production Rust shell_tool handler; complete command string, native schema, native exit footer; explicit path denials, not a global read allowlist',
            'decoder_source_sha256':b.sha(args.decoder.with_suffix('.rs').read_bytes()),
            'opencode_lock_sha256':b.sha((args.opencode/'bun.lock').read_bytes()),
            'authorization':'User explicitly approved native local/API history-recovery replay after mechanism preflight; total budget 300 includes all prior runs',
            'codex_commit':'b13164d86f9a70adc48d22f4a5a07ed0c001a1d0','opencode_commit':'e03db9bc6908f75c9334d8aa997deeaac81c0298',
            'tool_limit':TOOL_LIMIT,'native_output_byte_stop':BYTE_STOP,'truncate_native_output':False,
            'codex_execution':'real CLI tool handler, deterministic local Responses event injector; no model in injector',
            'opencode_execution':'native CLI import/export and native debug-agent ToolRegistry; explicit allow/deny (no ask)',
            'missing_inputs':'no historical project snapshots or pre-existing output spools invented; reduced corpus metadata omissions retained'}
        b.immutable(args.output/'native-open-manifest.json',config)
        calls=NativeExamCalls(args.output,300,args.model,args.bridge,b.sha(config),previous_spend=config['prior_spend'])
        schedule=b.load(args.closed/'schedule.json'); rng=random.Random(b.SEED+4)
        for chain in closed['chains']:
            data=b.load(args.closed/'corpus'/f'{chain}.json')
            for stage,cut in enumerate(data['cuts']):
                question=b.load(args.closed/'questions'/f'{chain}-{stage}.json')
                step=schedule[chain].index(cut); arms=list(b.ARMS); rng.shuffle(arms)
                for arm in arms:
                    identity=f'{chain}-{stage}-{arm}-native-open'; dest=args.output/'scores'/f'{identity}.json'
                    projection=b.load(args.closed/'projections'/f'{chain}-{step}-{arm}-compact.json')
                    closed_score=b.load(args.closed/'scores'/f'{chain}-{stage}-{arm}-closed.json')
                    assert b.sha(projection['text'])==closed_score['projection_sha256']
                    if dest.exists():
                        assert b.load(dest)['projection_sha256']==closed_score['projection_sha256']; continue
                    if any(r['identity'].startswith(identity+'-turn') for r in calls.rows.values()):
                        raise RuntimeError('partial case requires explicit recovery; do not silently regenerate native outputs')
                    if args.prepare_only:
                        f.normalize(args.output,args.dumper,args.model,data['records'][:cut]); continue
                    engine=None
                    try:
                        engine,tools,system,execute=native_case(args,args.output,identity,arm,data['records'][:cut])
                        result=examine(calls,identity,projection,question,tools,system,execute,args.output)
                        b.immutable(dest,dict(result,chain=chain,stage=stage,arm=arm))
                    finally:
                        if engine is not None:
                            engine.close()
                            if hasattr(engine,'raw_requests'): b.save(args.output/'control'/identity/'native-requests.json',engine.raw_requests)
                    report(args.output)
        if not args.prepare_only: report(args.output)
    finally: lock.unlink()


if __name__=='__main__': main()
