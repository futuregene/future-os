"""C/deterministic/summarized replay adapter using Future's PRODUCTION tool handlers.

Kept separate from native_stores.py so prior experiment sources remain intact.
The per-case `future` launcher validates resolved argv, not shell source text:
loops, semicolons, pipes and jq are interpreted by the original host shell.

Every tool is described and executed by `production_tool_executor`, which looks the name up in
production's `coding_tools()` and calls that handler. The replay therefore hands the model
production's own definitions and runs whatever it calls through production code; the only
additions are the harness's own path boundary and its command gate.
"""
import json
import os
from pathlib import Path
import subprocess
import sys

from native_stores import NativeFuture


class NativeFutureShell(NativeFuture):
    def __init__(self,binary,shell_probe,workspace,home,control,messages,sid,tool_names=None):
        if os.name=='nt':
            raise RuntimeError('read-isolating native replay is not verified on Windows; no downgrade')
        self.shell_probe=Path(shell_probe).resolve()
        super().__init__(binary,workspace,home,control,messages,sid)
        bindir=self.workspace/'native-bin'; bindir.mkdir(exist_ok=True)
        wrapper=bindir/'future'
        # This is only a read-only RPC command gate, not a shell parser or
        # history implementation. The original CLI does all querying/paging.
        wrapper.write_text('#!'+sys.executable+'\n'+
            'import subprocess,sys\n'+
            'args=sys.argv[1:]\n'+
            'ok=len(args)>=3 and args[:2]==["session","history"] and args[2] in ("search","get")\n'+
            'if ok and args[3:] not in (["--help"],["-h"]):\n'+
            '    sessions=[args[i+1] for i,a in enumerate(args[:-1]) if a=="--session"]\n'+
            '    ok=sessions==['+repr(sid)+']\n'+
            'if not ok:\n'+
            '    print("STUDY_SCOPE_DENIED: only native history search/get for this session",file=sys.stderr)\n'+
            '    sys.exit(2)\n'+
            'sys.exit(subprocess.call(['+repr(str(self.binary.resolve()))+']+args))\n')
        wrapper.chmod(0o700)
        self.env['PATH']=str(bindir)+os.pathsep+self.env['PATH']
        # The harness's own path boundary. Production's file tools resolve an absolute path
        # as-is and do not consult the rule-based denials (`run_read` reads whatever it is
        # given), so a replay that offers production's whole tool set must bound those tools
        # itself. Read and write alike are confined to this case's own two directories;
        # everything else — sibling cases, the control directory, the host's files — is
        # outside. `shell` is bounded separately by the OS sandbox and by the launcher above.
        self.allowed_roots=[str(self.workspace.resolve()),str(self.home.resolve())]
        self.denied_reads=[str(self.control.resolve())]
        self.denied_writes=[str(self.home.resolve())]
        names=list(tool_names) if tool_names else ['shell']
        self.tools=[self.describe(name) for name in names]
        self.tool=next((t for t in self.tools if t['function']['name']=='shell'),self.tools[0])

    def describe(self,name):
        descriptor=subprocess.run([str(self.shell_probe)],input=json.dumps({'mode':'describe','tool':name}),
            env=self.env,cwd=self.workspace,text=True,capture_output=True,check=True)
        return json.loads(descriptor.stdout)

    def execute_tool(self,name,arguments):
        request={'mode':'execute','tool':name,'workspace':str(self.workspace.resolve()),
                 'allowed_roots':self.allowed_roots,
                 'denied_reads':self.denied_reads,'denied_writes':self.denied_writes,'arguments':arguments}
        native=subprocess.run([str(self.shell_probe)],input=json.dumps(request),cwd=self.workspace,
            env=self.env,text=True,capture_output=True,timeout=150)
        if native.returncode:
            raise RuntimeError('native tool execution failed: '+native.stderr.strip())
        result=json.loads(native.stdout)
        assert result['native_tool']==name, 'executor returned a different tool'
        # Preserve nonzero native shell statuses. Do not claim that the final
        # status of a multi-command script describes every earlier command.
        return result

    def execute_shell(self,arguments):
        return self.execute_tool('shell',arguments)
