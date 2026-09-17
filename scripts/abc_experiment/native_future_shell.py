"""C/C3 replay adapter using Future's PRODUCTION Rust shell tool.

Kept separate from native_stores.py so prior experiment sources remain intact.
The per-case `future` launcher validates resolved argv, not shell source text:
loops, semicolons, pipes and jq are interpreted by the original host shell.
"""
import json
import os
from pathlib import Path
import subprocess
import sys

from native_stores import NativeFuture


class NativeFutureShell(NativeFuture):
    def __init__(self,binary,shell_probe,workspace,home,control,messages,sid):
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
        self.denied_reads=[str(self.control.resolve())]
        # Native Future path policies are preserved. These explicit denials
        # protect this replay's sibling fixtures; they are not a global read
        # whitelist. Do not treat this adapter alone as a complete data sandbox.
        for child in self.workspace.parent.iterdir():
            if child.resolve() not in (self.workspace.resolve(),self.home.resolve(),self.control.resolve()):
                self.denied_reads.append(str(child.resolve()))
        self.denied_writes=[str(self.home.resolve())]
        descriptor=subprocess.run([str(self.shell_probe)],input=json.dumps({'mode':'describe'}),env=self.env,
            cwd=self.workspace,text=True,capture_output=True,check=True)
        self.tool=json.loads(descriptor.stdout)

    def execute_shell(self,arguments):
        request={'mode':'execute','workspace':str(self.workspace.resolve()),
                 'denied_reads':self.denied_reads,'denied_writes':self.denied_writes,'arguments':arguments}
        native=subprocess.run([str(self.shell_probe)],input=json.dumps(request),cwd=self.workspace,
            env=self.env,text=True,capture_output=True,timeout=150)
        if native.returncode:
            raise RuntimeError('native Future shell failed: '+native.stderr)
        result=json.loads(native.stdout)
        assert result['native_tool']=='shell'
        # Preserve nonzero native shell statuses. Do not claim that the final
        # status of a multi-command script describes every earlier command.
        return result
