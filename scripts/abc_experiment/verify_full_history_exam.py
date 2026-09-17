"""Offline verification of newly matched full-history closed/open outputs."""
import argparse
import json
from pathlib import Path
import sqlite3
import tempfile

import full_history_exam as x
b=x.b; a=x.a; t=x.t


def verify(root,source,prior):
    config=b.load(root/'manifest.json'); ledger=b.load(root/'ledger.json')
    assert b.sha(b.load(prior/'ledger.json'))==config['prior_ledger_sha256']
    assert b.load(prior/'verified-report.json')['total_spend']==config['opening_spend']
    assert b.sha(b.load(source/'fidelity-manifest.json'))==config['source_manifest_sha256']
    source_ledger=b.load(source/'ledger.json'); assert b.sha(source_ledger)==config['source_ledger_sha256']
    source_index={row['identity']:key for key,row in source_ledger.items()}
    for name,digest in config['code_hashes'].items(): assert b.sha((b.REPO/name).read_bytes())==digest,name
    for info in config['binaries'].values(): assert b.sha(Path(info['path']).read_bytes())==info['sha256']
    for key,info in config['guidance_sources'].items():
        assert b.sha(Path(info['path']).read_bytes())==info['sha256']
        assert b.load(root/'guidance-sources'/f'{key}.json')['text']==Path(info['path']).read_text()
    codex=Path(config['guidance_sources']['codex']['path']).parents[4]
    opencode=Path(config['guidance_sources']['opencode_read']['path']).parents[4]
    guides=x.Guidance(b.REPO,codex,opencode)
    assert b.sha(guides.adaptations())==config['guidance_adaptations_sha256']
    assert all(row['state']=='finished' for row in ledger.values())
    calls={row['identity']:(key,row) for key,row in ledger.items()}; seen=set(); dbs=0; native_queries=0; local_replays=0
    schedule=b.load(source/'schedule.json'); paired_bodies={}; glob_order_only=[]
    with tempfile.TemporaryDirectory(prefix='verify-files-',dir=root) as temporary:
        for job in config['jobs']:
            identity=job['identity']; chain=job['chain']; stage=job['stage']; arm=job['arm']; mode=job['mode']
            row=b.load(root/'answers'/f'{identity}.json')
            for key,value in job.items(): assert row[key]==value
            original_question=b.load(source/'questions'/f'{chain}-{stage}.json'); question=b.load(root/'questions'/f'{chain}-{stage}.json')
            assert question==dict(original_question,prompt=x.question_from(original_question['prompt']))
            assert x.d.candidates(question['prompt'])==x.d.candidates(original_question['prompt'])
            assert b.sha(original_question)==job['original_question_sha256'] and b.sha(question)==job['question_sha256']
            projection_data=b.load(source/'projections'/f'{chain}-{job["step"]}-{arm}-compact.json'); projection=projection_data['text']
            assert b.sha(projection)==job['projection_sha256']
            old=b.load(source/'requests'/f'{source_index[f"{chain}-{stage}-{arm}-closed"]}.json')['body']
            guide=guides.text(arm,job['session_id'],bool(projection_data.get('checkpoint')) if arm in ('C','C3') else True)
            assert b.load(root/'guidance'/f'{chain}-{stage}-{arm}.json')['text']==guide
            base=x.body_from(old,projection,question['prompt'],guide if mode=='open' else '')
            initial=a.open_body(base,a.schemas(arm)) if mode=='open' else base
            assert initial==b.load(root/'initial-bodies'/f'{identity}.json') and b.sha(initial)==job['initial_body_sha256']
            assert [m['role'] for m in initial['messages']]==['system','user']
            paired_bodies.setdefault((chain,stage,arm),{})[mode]=initial
            trace=b.load(root/'traces'/f'{identity}.json') if (root/'traces'/f'{identity}.json').exists() else []
            lookup=[]
            if mode=='open':
                records=b.load(source/'corpus'/f'{chain}.json')['records'][:job['cut']]
                if arm=='codex':
                    backend=t.CodexHistory(records,[end for end in schedule[chain] if end<=job['cut']]); lookup=[item['content'] for item in backend.flat]
                elif arm=='opencode':
                    backend=t.ObservedFiles(records,Path(temporary)/identity); lookup=list(backend.files)+list(backend.files.values())
                    assert b.load(root/'file-visibility'/f'{identity}.json')['content_sha256']==b.sha(backend.files)
                else:
                    lookup=[t.body(item) for item in records]; proof=b.load(root/'database-proofs'/f'{identity}.json')
                    with sqlite3.connect(Path(proof['database']).as_uri()+'?mode=ro',uri=True) as db:
                        status=db.execute('select status from legacy_imports where session_id=?',(job['session_id'],)).fetchone()
                        count=db.execute("select count(*) from entries where session_id=? and entry_type in ('user','assistant','tool')",(job['session_id'],)).fetchone()[0]
                    assert status==('imported',) and count==proof['expected']; dbs+=1
            messages=list(initial['messages']); offset=0; queries=0; total=0; final=None
            for turn in range(row['model_turns']):
                call_id=identity+f'-turn{turn}'; key,call=calls[call_id]; seen.add(call_id)
                request=b.load(root/'requests'/f'{key}.json'); assert b.sha(request)==call['request_sha256']
                body=request['body']; assert body.get('tool_choice') in (None,'auto')
                expected=dict(initial)
                if mode=='open' and (queries>=config['logical_query_limit'] or total>=config['byte_stop']):
                    expected.pop('tools'); expected.pop('tool_choice')
                    messages.append({'role':'user','content':'The lookup allowance is exhausted. Answer the original question using the evidence already available.'})
                expected['messages']=messages
                assert body==expected
                payload=b.load(root/call['artifact'])
                if payload.get('calls'):
                    assert mode=='open'
                    messages.append({'role':'assistant','content':payload['text'] or None,'tool_calls':payload['calls']})
                    for tool_call in payload['calls']:
                        item=trace[offset]; offset+=1; fn=tool_call['function']
                        assert item['name']==fn['name'] and item['arguments']==fn.get('arguments')
                        assert item['bytes']==len(item['output'].encode()); total+=item['bytes']
                        if item['status']!='budget_denied': queries+=1
                        if item['status']=='ok':
                            args=json.loads(item['arguments'])
                            if arm in ('C','C3'):
                                native=json.loads(item['output']); assert native['sessionId']==job['session_id']; native_queries+=1
                                if item['name']=='history_search': assert native['query']==args['query']
                                else: assert native['entryId']==args['entry_id']
                            else:
                                replay=backend.call(item['name'],args)
                                if replay!=item['output']:
                                    # rg --files traversal order is not stable across
                                    # materializations. Verify the identical complete
                                    # filename multiset, not an invented sorted order
                                    # in the original model input. Other differences
                                    # (including different capped subsets) still fail.
                                    actual_lines=item['output'].splitlines(); replay_lines=replay.splitlines()
                                    assert item['name']=='glob' and actual_lines and all(line.startswith('/') for line in actual_lines)
                                    assert sorted(actual_lines)==sorted(replay_lines),(identity,item['name'],args,replay[:900],item['output'][:900])
                                    glob_order_only.append({'identity':identity,'arguments':args,'file_count':len(actual_lines)})
                                local_replays+=1
                        messages.append({'role':'tool','tool_call_id':tool_call['id'],'content':item['output']})
                else:
                    assert turn==row['model_turns']-1,'direct answer was retried'
                    final=b.parse_answer(payload['text'])
            assert final==row['answer'] and offset==len(trace) and queries==row['logical_queries'] and total==row['returned_bytes']
            assert bool(queries)==row['lookup_used']
            if mode=='closed': assert row['model_turns']==1 and not trace and 'tools' not in initial
            recomputed=x.grade({'answer':final},question,projection,lookup)
            assert all(row[key]==value for key,value in recomputed.items())
    assert len(paired_bodies)==72 and len(config['jobs'])==144 and seen==set(calls)
    assert len(list((root/'answers').glob('*.json')))==144
    for key,pair in paired_bodies.items():
        chain,stage,arm=key; guide=b.load(root/'guidance'/f'{chain}-{stage}-{arm}.json')['text']
        x.g.assert_guidance_parity(pair['closed'],pair['open'],a.schemas(arm),guide)
    result=x.report(root)
    assert result['complete'] and result['pairs_complete']==72 and result['total_spend']<=300
    result.update(artifact_consistent=True,matched_initial_pairs_verified=72,model_requests_verified=len(calls),
                  native_databases_verified=dbs,native_query_outputs_checked=native_queries,local_queries_replayed=local_replays,
                  glob_order_only_replay_differences=glob_order_only,
                  prior_answer_injected=False,pending_list_injected=False,forced_tool_choice=False,
                  scope='new matched full-original-history questions; autonomous tool+guidance treatment; inherited replica/file/scoring limitations')
    b.save(root/'verified-report.json',result)
    return result


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for key in ('output','source','prior'): parser.add_argument('--'+key,type=Path,required=True)
    args=parser.parse_args(); print(json.dumps(verify(args.output,args.source,args.prior),ensure_ascii=False,indent=2))
