"""Native-format replay stores. No historical tools or model calls are executed."""
import json
from pathlib import Path
import socket
import subprocess
import threading
import time

from native_codex import isolated_env

STAMP='2026-09-17T00:00:00Z'


def codex_rollout(messages,sid,workspace,home,decoder):
    path=Path(home)/'.codex/sessions/2026/09/17'/f'rollout-2026-09-17T00-00-00-{sid}.jsonl'
    path.parent.mkdir(parents=True,exist_ok=True)
    rows=[{'timestamp':STAMP,'type':'session_meta','payload':{
        'session_id':sid,'id':sid,'timestamp':STAMP,'cwd':str(workspace),
        'originator':'native-history-replay-fixture','cli_version':'0.0.0','source':'cli','model_provider':'study'}}]
    for m in messages:
        blocks=[]
        for c in m['content']:
            if c['type']=='text': blocks.append({'type':'output_text' if m['role']=='assistant' else 'input_text','text':c['text']})
        if blocks: rows.append({'timestamp':STAMP,'type':'response_item','payload':{'type':'message','role':m['role'],'content':blocks}})
        for c in m['content']:
            if c['type']=='tool_call':
                item={'type':'function_call','call_id':c['id'],'name':c['name'],'arguments':json.dumps(c['args'],ensure_ascii=False)}
            elif c['type']=='tool_result':
                item={'type':'function_call_output','call_id':c['tool_call_id'],'output':c['content']}
            else: continue
            rows.append({'timestamp':STAMP,'type':'response_item','payload':item})
    encoded=''.join(json.dumps(r,ensure_ascii=False)+'\n' for r in rows)
    # Validate every record with the unmodified upstream Rust rollout parser.
    proof=json.loads(subprocess.check_output([str(decoder)],input=encoded,text=True))
    assert proof=={'validated_lines':len(rows),'response_items':len(rows)-1}
    if path.exists() and path.read_text()!=encoded: raise RuntimeError('refuse to change existing native rollout')
    path.write_text(encoded)
    return path,proof


class NativeFuture:
    def __init__(self,binary,workspace,home,control,messages,sid):
        self.binary=Path(binary); self.workspace=Path(workspace); self.home=Path(home); self.control=Path(control); self.sid=sid
        for p in (self.workspace,self.home,self.control): p.mkdir(parents=True,exist_ok=True)
        self.env=isolated_env(self.home)
        config=self.home/'.future/agent'; sessions=config/'sessions'; sessions.mkdir(parents=True,exist_ok=True)
        (config/'models.json').write_text(json.dumps({'providers':{'offline':{'api':'openai-completions','apiKey':'unused-offline',
            'baseUrl':'http://127.0.0.1:9/v1','models':[{'id':'model','modalities':['text'],'reasoning':False,'contextWindow':1000000,'maxTokens':32000}]}}}))
        (config/'settings.json').write_text(json.dumps({'defaultModel':'offline/model'}))
        source=sessions/f'{sid}.jsonl'
        rows=[{'id':'info','type':'session_info','role':'system','timestamp':STAMP,
               'content':{'cwd':str(self.workspace),'model':'offline/model'}}]
        for i,m in enumerate(messages):
            ident=(m.get('metadata') or {}).get('_future_journal_entry_id',f'entry-{i:08d}')
            rows.append({'id':ident,'type':m['role'],'role':m['role'],'timestamp':STAMP,'content':m['content']})
        source.write_text(''.join(json.dumps(r,ensure_ascii=False)+'\n' for r in rows))
        with socket.socket() as s: s.bind(('127.0.0.1',0)); port=s.getsockname()[1]
        self.env['FUTURE_AGENT_GRPC_ADDR']=f'127.0.0.1:{port}'
        self.log=(self.control/'native-agent.log').open('ab')
        self.process=subprocess.Popen([str(self.binary),'agent','--grpc-addr',self.env['FUTURE_AGENT_GRPC_ADDR']],
            env=self.env,cwd=self.workspace,stdout=self.log,stderr=subprocess.STDOUT)
        deadline=time.monotonic()+40
        while time.monotonic()<deadline:
            if self.process.poll() is not None: raise RuntimeError('isolated Future agent exited')
            try:
                with socket.create_connection(('127.0.0.1',port),timeout=.1): return
            except OSError: threading.Event().wait(.05)
        self.close(); raise TimeoutError('Future archive agent startup')

    def execute(self,argv):
        if argv[:3]!=['future','session','history'] or argv[3] not in ('search','get'):
            raise ValueError('only native session history search/get allowed')
        if argv[4:] not in (['--help'],['-h']):
            if '--session' not in argv or argv[argv.index('--session')+1]!=self.sid: raise ValueError('wrong archive scope')
        result=subprocess.run([str(self.binary),*argv[1:]],cwd=self.workspace,env=self.env,text=True,capture_output=True,timeout=30)
        return result.stdout+result.stderr+f'\n[exit: {result.returncode}]'

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try: self.process.wait(timeout=10)
            except subprocess.TimeoutExpired: self.process.kill(); self.process.wait(timeout=5)
        self.log.close()
