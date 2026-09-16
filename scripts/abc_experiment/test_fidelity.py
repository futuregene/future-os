"""Offline policy and message-lowering regressions. No provider calls."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

import fidelity_rerun as f


class FidelityTests(unittest.TestCase):
    def test_codex_byte_estimate_and_middle_truncation(self):
        self.assertEqual(f.codex_tokens('中文'),2)
        self.assertEqual(f.codex_tokens('😀'),1)
        self.assertEqual(f.codex_truncate('abcdefghij',2),'abcd…1 tokens truncated…ghij')
        self.assertEqual(f.codex_truncate('中文',0),'…2 tokens truncated…')
        self.assertEqual(f.codex_truncate('abcd',1),'abcd')

    def test_codex_preserves_roles_and_tool_pairs(self):
        messages=[{'role':'user','content':[{'type':'text','text':'question'}]},
                  {'role':'assistant','content':[{'type':'text','text':'working'},
                      {'type':'tool_call','id':'a','name':'read','args':{'path':'x'}}]},
                  {'role':'tool','content':[{'type':'tool_result','tool_call_id':'a','content':'result','is_error':False}]}]
        chat=f.to_chat(messages)
        self.assertEqual([x['role'] for x in chat],['user','assistant','tool'])
        self.assertEqual(chat[1]['tool_calls'][0]['function']['arguments'],'{"path":"x"}')
        self.assertEqual(chat[2]['tool_call_id'],'a')
        with self.assertRaisesRegex(ValueError,'dangling'):
            f.to_chat(messages[:-1])

    def test_codex_prior_summary_is_input_not_reprotected_user(self):
        user={'role':'user','content':[{'type':'text','text':'directive'}]}
        prior=f.codex_keep([user],'old summary')
        self.assertIn('old summary',f.to_chat(prior)[-1]['content'])
        kept=f.codex_keep(prior,'new summary')
        self.assertEqual(len(kept),2)
        self.assertNotIn('old summary',f.render(kept))
        self.assertIn('directive',f.render(kept))

    def test_safe_schedule_never_splits_pairs(self):
        records=[]
        for i in range(3):
            records += [{'kind':'text','role':'user','text':'x','order':i*3,'id':f'u{i}'},
                        {'kind':'tool_call','role':'assistant','call':str(i),'order':i*3+1,'id':f'a{i}'},
                        {'kind':'tool_result','role':'tool','call':str(i),'order':i*3+2,'id':f't{i}','text':'result'}]
        data,ends=f.safe_plan({'records':records,'cuts':[2,5,9]})
        self.assertEqual(data['cuts'],[1,4,9])
        self.assertEqual(ends,[1,4,9])

    def test_prior_spend_is_in_the_budget(self):
        with tempfile.TemporaryDirectory() as root:
            calls=f.Calls(Path(root),300,'model',Path('bridge'),'id',previous_spend=299)
            self.assertEqual(calls.spent(),299)
            with self.assertRaisesRegex(RuntimeError,'BUDGET'):
                calls.execute('x',{},[],2,lambda s:({},0))

    def test_sdk_lowering_and_utf16_token_measurement(self):
        sdk=Path(os.environ.get('FIDELITY_SDK_ROOT',str(Path.home()/'compact-exp/fidelity-sdk')))
        self.assertTrue((sdk/'node_modules/ai/package.json').exists(),'install the pinned SDK before the fidelity pass')
        messages=[{'role':'user','content':[{'type':'text','text':'中文😀'}]},
                  {'role':'assistant','content':[{'type':'tool_call','id':'a','name':'read','args':{'path':'x'}}]},
                  {'role':'tool','content':[{'type':'tool_result','tool_call_id':'a','content':'OK','is_error':False}]}]
        request={'messages':messages,'sdkRoot':str(sdk),'window':128000,'maxOutput':32000,'includeAll':True}
        data=json.loads(subprocess.check_output(['node',str(f.b.HERE/'opencode_fidelity.mjs')],input=json.dumps(request),text=True))
        self.assertEqual(data['sdkVersion'],'6.0.168')
        self.assertEqual(data['budget'],15000)
        self.assertEqual(data['tailMessages'],[])  # upstream keep.start == 0 => no separable tail
        self.assertEqual(data['allModelMessages'][0]['content'],[{'type':'text','text':'中文😀'}])
        self.assertEqual(data['allModelMessages'][1]['content'][0]['type'],'tool-call')
        self.assertEqual(data['allModelMessages'][2]['content'][0]['output'],{'type':'text','value':'OK'})

    def test_sdk_tail_retains_newest_complete_turn(self):
        sdk=Path(os.environ.get('FIDELITY_SDK_ROOT',str(Path.home()/'compact-exp/fidelity-sdk')))
        messages=[{'role':'user','content':[{'type':'text','text':'x'*100000}]},
                  {'role':'assistant','content':[{'type':'text','text':'old'}]},
                  {'role':'user','content':[{'type':'text','text':'new'}]},
                  {'role':'assistant','content':[{'type':'text','text':'中文😀'*1000}]}]
        selected=f.sdk_select(messages,sdk,32000)
        self.assertGreater(selected['tailEntry'],0)
        self.assertEqual(selected['tailMessages'],messages[1:])
        encoded=json.dumps(selected['tailModelMessages'],ensure_ascii=False,separators=(',',':'))
        expected=(len(encoded.encode('utf-16-le'))//2+2)//4
        self.assertEqual(selected['tailEstimatedTokens'],expected)


if __name__=='__main__': unittest.main()
