"""Black-box native retrieval tests. Set NATIVE_RETRIEVAL_TEST_ROOT to run binaries."""
import json
import os
from pathlib import Path
import tempfile
import unittest
import uuid

from native_scope import check_shell,check_tool
from native_codex import NativeCodex
from native_opencode import NativeOpenCode
from native_stores import NativeFuture,codex_rollout

ROOT=Path(os.environ.get('NATIVE_RETRIEVAL_TEST_ROOT','/nonexistent-native-test-root'))


class ScopeTests(unittest.TestCase):
    def test_read_pipeline_and_reject_writes_or_other_archives(self):
        with tempfile.TemporaryDirectory() as d:
            work=Path(d)/'work'; home=Path(d)/'home'
            check_shell("opencode export ses_case | jq -r '.messages[].parts[].text // empty' | rg -F -o 'needle'",work,home,'ses_case')
            check_shell("rg -o -F -e 'one' -e 'two' archive.jsonl | sort | uniq -c",work,home,'ses_case')
            check_shell("rg -c 'one' archive.jsonl; rg -nF 'two' archive.jsonl | head -c 300; echo '---'",work,home,'ses_case')
            check_shell("opencode export ses_case > retrieved/archive.json && jq -r '.messages[].parts[].text // empty' retrieved/archive.json | sort -u",work,home,'ses_case')
            check_shell("grep -o -F 'one' archive.jsonl | head -1",work,home,'ses_case')
            for command in ('cat /etc/passwd','opencode export ses_other','rg --pre rm needle','cat x > y','cat $(env)','rm x'):
                with self.assertRaises(ValueError): check_shell(command,work,home,'ses_case')
            with self.assertRaises(ValueError): check_tool('exec_command',{'cmd':'cat x','sandbox_permissions':'require_escalated'},work,home,'ses_case')
            with self.assertRaises(ValueError): check_tool('read',{'filePath':str(home/'logs/current-query.log')},work,home,'ses_case',roots=[work,home/'tool-output'])


@unittest.skipUnless((ROOT/'codex-target/debug/codex').exists(),'requires explicitly enabled pinned native toolchain')
class NativeTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='blackbox-',dir=ROOT/'preflight')
        self.base=Path(self.temp.name)
        self.work=self.base/'workspace'; self.work.mkdir()
        self.home=self.base/'home'; self.control=self.base/'control'
        self.messages=[{'role':'user','content':[{'type':'text','text':'Original request'}]},
                       {'role':'assistant','content':[{'type':'text','text':'NATIVE_TEST_VALUE_739164 中文'}]}]
    def tearDown(self): self.temp.cleanup()

    def test_codex_parser_native_exec_and_native_sandbox(self):
        sid=str(uuid.uuid4())
        archive,proof=codex_rollout(self.messages,sid,self.work,self.home,ROOT/'native_validate_rollout')
        self.assertEqual(proof['response_items'],2)
        outside=self.base/'not-in-archive.txt'; outside.write_text('OUTSIDE_NATIVE_TEST_888')
        with NativeCodex(ROOT/'codex-target/debug/codex',self.work,self.home,self.control) as engine:
            result=engine.execute('exec_command',{'cmd':f"rg -F -o 'NATIVE_TEST_VALUE_739164' '{archive}'",'max_output_tokens':1000})
            self.assertIn('NATIVE_TEST_VALUE_739164',result)
            denied=engine.execute('exec_command',{'cmd':f"cat '{outside}'",'max_output_tokens':1000})
            self.assertNotIn('OUTSIDE_NATIVE_TEST_888',denied)
            engine.execute('exec_command',{'cmd':'touch forbidden-write','max_output_tokens':1000})
            self.assertFalse((self.work/'forbidden-write').exists())

    def test_opencode_native_import_export_pipe_and_output_spooling(self):
        bun=ROOT.parent/'native-tools/node_modules/@oven/bun-darwin-aarch64/bin/bun'
        engine=NativeOpenCode(ROOT.parent/'native-src/opencode',bun,self.work,self.home,self.control)
        tool_messages=[self.messages[0],{'role':'assistant','content':[{'type':'tool_call','id':'call_native','name':'read','args':{'path':'old-file.txt'}}]},
                       {'role':'tool','content':[{'type':'tool_result','tool_call_id':'call_native','content':'ORIGINAL_TOOL_RESULT_374 中文','is_error':False}]}]
        tool_export=engine.import_session(tool_messages,'ses_native_tool_test')
        self.assertEqual(tool_export['messages'][1]['parts'][0]['state']['output'],'ORIGINAL_TOOL_RESULT_374 中文')
        sid='ses_native_large_test'
        messages=[self.messages[0],{'role':'assistant','content':[{'type':'text','text':'native fixture line\n'*12000+'NATIVE_LARGE_TAIL_824917'}]}]
        exported=engine.import_session(messages,sid)
        self.assertEqual(exported['messages'][1]['parts'][0]['text'],messages[1]['content'][0]['text'])
        engine.execute('bash',{'command':f'opencode export {sid} > retrieved/archive.json'})
        output,raw=engine.execute('bash',{'command':'cat retrieved/archive.json'})
        self.assertTrue(raw['result']['metadata']['truncated'])
        path=Path(raw['result']['metadata']['outputPath'])
        self.assertTrue(path.is_relative_to(self.home))
        self.assertIn('NATIVE_LARGE_TAIL_824917',path.read_text())
        command="jq -r '.messages[].parts[].text // empty' retrieved/archive.json | rg -F -o 'NATIVE_LARGE_TAIL_824917'"
        check_shell(command,self.work,self.home,sid)
        output,_=engine.execute('bash',{'command':command})
        self.assertIn('NATIVE_LARGE_TAIL_824917',output)
        outside=self.base/'outside.txt'; outside.write_text('NO_READ_THIS_918')
        with self.assertRaises(RuntimeError): engine.execute('read',{'filePath':str(outside)})
        with self.assertRaises(RuntimeError): engine.execute('write',{'filePath':str(self.work/'bad'),'content':'bad'})

    def test_future_native_history_cli(self):
        binary=Path(__file__).resolve().parents[2]/'target/fidelity/debug/future'
        engine=NativeFuture(binary,self.work,self.home,self.control,self.messages,'native-blackbox')
        try:
            output=engine.execute(['future','session','history','search','--session','native-blackbox','--query','NATIVE_TEST_VALUE_739164','--json'])
            self.assertIn('NATIVE_TEST_VALUE_739164',output)
            with self.assertRaises(ValueError): engine.execute(['future','session','history','search','--session','wrong','--query','x'])
        finally: engine.close()


if __name__=='__main__': unittest.main()
