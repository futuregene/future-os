"""Pinned OpenCode native import/export and real debug-agent tool execution."""
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.parse
import urllib.request

from native_codex import isolated_env


class NativeOpenCode:
    def __init__(self,source,bun,workspace,home,control):
        self.source=Path(source); self.bun=str(bun); self.workspace=Path(workspace); self.home=Path(home); self.control=Path(control)
        for p in (self.workspace,self.home,self.control): p.mkdir(parents=True,exist_ok=True)
        self.env=isolated_env(self.home)
        self.env.update(OPENCODE_DISABLE_MODELS_FETCH='true',OPENCODE_DISABLE_AUTOUPDATE='true',OPENCODE_DISABLE_DEFAULT_PLUGINS='true',OPENCODE_DISABLE_EXTERNAL_SKILLS='true',OPENCODE_DISABLE_CLAUDE_CODE_SKILLS='true',OPENCODE_DISABLE_CLAUDE_CODE='true')
        if not (self.workspace/'.git').exists():
            subprocess.run(['git','init','--quiet',str(self.workspace)],check=True,capture_output=True)
        config={'$schema':'https://opencode.ai/config.json','autoupdate':False,'model':'study/deepseek-flash',
            'provider':{'study':{'npm':'@ai-sdk/openai-compatible','name':'offline native tool driver',
                'options':{'baseURL':'http://127.0.0.1:9/v1','apiKey':'unused-offline'},
                'models':{'deepseek-flash':{'name':'DeepSeek (tools only; no model calls)',
                    'limit':{'context':1000000,'output':384000}}}}},
            'permission':{'*':'deny','read':'allow',
                'glob':'allow','grep':'allow','external_directory':{str((self.home/'.local/share/opencode/tool-output').resolve())+'/*':'allow'},
                'bash':{'opencode export *':'allow','opencode session list*':'allow','opencode --help':'allow',
                        'rg *':'allow','grep *':'allow','jq *':'allow','cat *':'allow','head *':'allow','tail *':'allow','ls*':'allow','wc *':'allow','sort*':'allow','uniq*':'allow','sed *':'allow','echo*':'allow','printf *':'allow'}},
            'agent':{'build':{'model':'study/deepseek-flash'}}}
        config_path=self.home/'.config/opencode/opencode.json'; config_path.parent.mkdir(parents=True,exist_ok=True)
        config_path.write_text(json.dumps(config,indent=2)+'\n')
        self.argv=[self.bun,'run',str(self.source/'packages/opencode/src/index.ts')]
        # Put a launcher for this exact source commit on the tool process PATH.
        # It forwards argv to the original CLI; no export/search implementation
        # is replaced here.
        (self.workspace/'retrieved').mkdir(exist_ok=True)
        bindir=self.workspace/'native-bin'; bindir.mkdir(exist_ok=True)
        launcher=bindir/'opencode'
        import sys
        launcher.write_text('#!'+sys.executable+'\nimport subprocess,sys\nsys.exit(subprocess.call('+repr(self.argv)+'+sys.argv[1:]))\n')
        launcher.chmod(0o700)
        self.env['PATH']=str(bindir)+os.pathsep+self.env['PATH']

    def cli(self,args,timeout=180):
        # Upstream CLI may exit before a large pipe write drains. A regular
        # stdout file preserves the native JSON response without modifying it.
        import uuid
        target=self.control/('cli-'+uuid.uuid4().hex+'.stdout')
        with target.open('wb') as stdout:
            result=subprocess.run([*self.argv,*args],cwd=self.workspace,env=self.env,stdout=stdout,stderr=subprocess.PIPE,timeout=timeout)
        if result.returncode: raise RuntimeError(f'native OpenCode {args[:2]} exit {result.returncode}: {result.stderr.decode(errors="replace")}')
        return target.read_text()

    def execute(self,name,args):
        # Strict JSON serialization never enters debug agent's JS-literal parser fallback.
        output=self.cli(['debug','agent','build','--tool',name,'--params',json.dumps(args)])
        try: result=json.loads(output)
        except json.JSONDecodeError as error: raise OSError('native debug-agent output is not complete JSON') from error
        assert result['tool']==name
        return result['result']['output'],result

    def import_session(self,messages,sid):
        payload=export_fixture(messages,sid,self.workspace)
        path=self.control/'native-import.json'; path.write_text(json.dumps(payload,ensure_ascii=False)+'\n')
        out=self.cli(['import',str(path)])
        assert 'Imported session: '+sid in out
        exported=json.loads(self.cli(['export',sid]))
        assert len(exported['messages'])==len(payload['messages'])
        before=payload['messages']; after=exported['messages']
        assert {x['info']['id']:x['parts'] for x in before}=={x['info']['id']:x['parts'] for x in after}, 'native import/export changed message parts'
        (self.control/'native-export.json').write_text(json.dumps(exported,ensure_ascii=False)+'\n')
        return exported

    def start_catalog(self):
        import socket
        with socket.socket() as s:
            s.bind(('127.0.0.1',0)); port=s.getsockname()[1]
        self.server_log=(self.control/'native-server.log').open('ab')
        self.server=subprocess.Popen([*self.argv,'serve','--hostname','127.0.0.1','--port',str(port)],cwd=self.workspace,env=self.env,stdout=self.server_log,stderr=subprocess.STDOUT)
        url=f'http://127.0.0.1:{port}/experimental/tool?'+urllib.parse.urlencode({'provider':'study','model':'deepseek-flash','directory':str(self.workspace)})
        deadline=time.monotonic()+40
        while time.monotonic()<deadline:
            if self.server.poll() is not None: raise RuntimeError('native OpenCode server exited')
            try:
                with urllib.request.urlopen(url,timeout=2) as response:
                    tools=json.load(response)
                return tools
            except (urllib.error.URLError,TimeoutError):
                time.sleep(.2)
        raise TimeoutError('native tool registry did not become ready')

    def close(self):
        if hasattr(self,'server'):
            self.server.terminate(); self.server.wait(timeout=15); self.server_log.close()


def export_fixture(messages,sid,workspace):
    now=1700000000000
    data={'info':{'id':sid,'slug':'native-replay','projectID':'global','directory':str(workspace),
                 'title':'Frozen conversation archive','version':'1.18.31','time':{'created':now,'updated':now}},'messages':[]}
    results={c['tool_call_id']:c for m in messages for c in m['content'] if c['type']=='tool_result'}
    import hashlib
    prefix=hashlib.sha256(sid.encode()).hexdigest()[:12]
    parent=f'msg_{prefix}_000000000000'
    for index,m in enumerate(messages):
        if m['role']=='tool': continue
        mid=f'msg_{prefix}_{index:012x}'
        info={'id':mid,'sessionID':sid,'role':m['role'],'time':{'created':now+index},'agent':'build'}
        if m['role']=='user':
            info['model']={'providerID':'study','modelID':'deepseek-flash'}; parent=mid
        else:
            info.update(parentID=parent,modelID='deepseek-flash',providerID='study',mode='build',
                        path={'cwd':str(workspace),'root':str(workspace)},cost=0,
                        tokens={'input':0,'output':0,'reasoning':0,'cache':{'read':0,'write':0}})
        parts=[]
        for j,c in enumerate(m['content']):
            part={'id':f'prt_{prefix}_{index:012x}{j:04x}','sessionID':sid,'messageID':mid}
            if c['type']=='text': part.update(type='text',text=c['text'])
            elif c['type']=='tool_call':
                result=results[c['id']]
                state={'status':'error' if result.get('is_error') else 'completed',
                       'input':c['args'],'time':{'start':now+index,'end':now+index+1},'metadata':{}}
                if result.get('is_error'): state['error']=result['content']
                else: state.update(output=result['content'],title=c['name'])
                part.update(type='tool',callID=c['id'],tool=c['name'],state=state)
            else: raise ValueError('unsupported frozen content')
            parts.append(part)
        data['messages'].append({'info':info,'parts':parts})
    return data
