"""Execute real pinned Codex tool handlers against a LOCAL deterministic transport.

The local HTTP server is not a retrieval implementation and never answers the
exam. It supplies tool-call events exactly like upstream test fixtures, receives
the actual native tool result, and relays that unchanged to the paid examiner.
Codex's native sandbox, tool registry, output cap and exec session state run in
the upstream binary. No production credentials enter this subprocess.
"""
import gzip
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def isolated_env(home):
    env={k:v for k,v in os.environ.items() if k in ('PATH','LANG','LC_ALL','TERM','SYSTEMROOT','WINDIR')}
    (home/'tmp').mkdir(parents=True,exist_ok=True)
    env.update(HOME=str(home),USERPROFILE=str(home),CODEX_HOME=str(home/'.codex'),TMPDIR=str(home/'tmp'),
               XDG_CONFIG_HOME=str(home/'.config'),XDG_DATA_HOME=str(home/'.local/share'),
               XDG_CACHE_HOME=str(home/'.cache'),SHELL='/bin/bash',NO_PROXY='127.0.0.1,localhost')
    return env


def flatten_tools(tools):
    found={}
    for spec in tools:
        if spec['type']=='namespace':
            for name,entry in flatten_tools(spec.get('tools',[])).items():
                found[name]=(entry[0],spec['name'])
        elif spec['type']=='function':
            found[spec['name']]=(spec,None)
    return found


class NativeCodex:
    def __init__(self,binary,workdir,home,control):
        self.binary=Path(binary); self.workdir=Path(workdir); self.home=Path(home); self.control=Path(control)
        self.home.mkdir(parents=True,exist_ok=True); (self.home/'.codex').mkdir(exist_ok=True)
        self.control.mkdir(parents=True,exist_ok=True)
        self.actions=queue.Queue(); self.results=queue.Queue(); self.ready=threading.Event()
        self.calls=0; self.tools={}; self.pending=None; self.raw_requests=[]
        owner=self
        class Handler(BaseHTTPRequestHandler):
            def log_message(self,*args): pass
            def do_GET(self):
                self.send_response(404); self.end_headers()
            def do_POST(self):
                raw=self.rfile.read(int(self.headers.get('Content-Length','0')))
                if self.headers.get('Content-Encoding')=='gzip': raw=gzip.decompress(raw)
                request=json.loads(raw)
                owner.raw_requests.append(request)
                if not owner.ready.is_set():
                    owner.tools=flatten_tools(request.get('tools',[])); owner.ready.set()
                if owner.pending:
                    outputs=[x for x in request.get('input',[]) if x.get('call_id')==owner.pending and x.get('type') in ('function_call_output','custom_tool_call_output')]
                    if outputs:
                        owner.results.put(outputs[-1]); owner.pending=None
                action=owner.actions.get(timeout=900)
                rid=f'local-native-{len(owner.raw_requests)}'
                events=[{'type':'response.created','response':{'id':rid}}]
                if action is None:
                    item={'type':'message','role':'assistant','id':'done','content':[{'type':'output_text','text':'Native tool phase finished.'}]}
                else:
                    name,args,callid=action
                    item={'type':'function_call','call_id':callid,'name':name,'arguments':json.dumps(args)}
                    namespace=owner.tools[name][1]
                    if namespace: item['namespace']=namespace
                    owner.pending=callid
                events.extend([{'type':'response.output_item.done','item':item},
                    {'type':'response.completed','response':{'id':rid,'usage':{'input_tokens':0,'output_tokens':0,'total_tokens':0}}}])
                body=''.join('data: '+json.dumps(e)+'\n\n' for e in events).encode()
                self.send_response(200); self.send_header('Content-Type','text/event-stream'); self.send_header('Content-Length',str(len(body))); self.end_headers(); self.wfile.write(body)
        self.server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
        self.server.daemon_threads=True
        self.thread=threading.Thread(target=self.server.serve_forever,daemon=True)
        self.thread.start()
        provider='{name="native-replay",base_url="http://127.0.0.1:'+str(self.server.server_port)+'/v1",wire_api="responses",requires_openai_auth=false}'
        runtime_roots=set()
        for name in ('rg','jq'):
            found=shutil.which(name)
            if not found: raise RuntimeError(f'required native reader missing: {name}')
            runtime_roots.update((str(Path(found).parent),str(Path(found).resolve().parent)))
            if sys.platform=='darwin':
                pending=[Path(found).resolve()]; visited=set()
                while pending:
                    item=pending.pop()
                    if item in visited: continue
                    visited.add(item)
                    linked=subprocess.check_output(['otool','-L',str(item)],text=True)
                    for line in linked.splitlines()[1:]:
                        dependency=Path(line.strip().split(' (',1)[0])
                        if dependency.exists() and not str(dependency).startswith(('/usr/','/System/')):
                            runtime_roots.update((str(dependency.parent),str(dependency.resolve().parent)))
                            pending.append(dependency.resolve())
        if sys.platform=='darwin':
            # dyld checks both Homebrew's opt symlink namespace and Cellar.
            # These are trusted runtime packages, never study data; the request
            # guard still forbids querying them as evidence.
            runtime_roots.update(str(p) for p in (Path('/opt/homebrew/bin'),Path('/opt/homebrew/lib'),Path('/opt/homebrew/opt'),Path('/opt/homebrew/Cellar')) if p.exists())
        filesystem={':minimal':'read',str(self.workdir.resolve()):'read',str((self.home/'.codex/sessions').resolve()):'read'}
        filesystem.update({path:'read' for path in runtime_roots})
        permission_toml='{'+','.join(json.dumps(k)+'="read"' for k in filesystem)+'}'
        argv=[str(self.binary),'exec','--ephemeral','--ignore-user-config','--ignore-rules','--skip-git-repo-check','--json',
              '-C',str(self.workdir),'-m','deepseek-flash',
              '-c','default_permissions="study"',
              '-c','permissions.study.filesystem='+permission_toml,
              '-c','permissions.study.network.enabled=false',
              '-c','model_provider="study"','-c','model_providers.study='+provider,
              '-c','approval_policy="never"','-c','features.context_management=false','-c','features.token_budget=false',
              '-c','features.memories=false','-c','shell_environment_policy.inherit="all"',
              'Native tool execution harness. Follow only the supplied tool actions; do not perform project work.']
        self.stdout=(self.control/'native-stdout.jsonl').open('ab'); self.stderr=(self.control/'native-stderr.log').open('ab')
        # Use upstream named permissions, not a home-grown shell sandbox:
        # platform runtime reads + this workspace + this archive only.
        self.process=subprocess.Popen(argv,cwd=self.workdir,env=isolated_env(self.home),stdout=self.stdout,stderr=self.stderr)
        if not self.ready.wait(45):
            code=self.process.poll(); self.close()
            raise RuntimeError(f'native Codex did not advertise tools; exit={code}; inspect {self.control}')

    def execute(self,name,args,timeout=120):
        if name not in self.tools: raise ValueError('not an advertised native tool')
        self.calls+=1; callid=f'native-call-{self.calls}'
        self.actions.put((name,args,callid))
        try: result=self.results.get(timeout=timeout)
        except queue.Empty as error: raise TimeoutError('native tool result not delivered') from error
        output=result['output']
        return output if isinstance(output,str) else json.dumps(output,ensure_ascii=False)

    def close(self):
        self.actions.put(None)
        if hasattr(self,'process'):
            try: self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.terminate(); self.process.wait(timeout=10)
            self.stdout.close(); self.stderr.close()
        self.server.shutdown(); self.server.server_close()

    def __enter__(self): return self
    def __exit__(self,*args): self.close()
