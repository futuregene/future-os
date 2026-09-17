import copy
import unittest

import c3_archive_audit as x


class ArchiveAuditTests(unittest.TestCase):
    def test_strengthening_only_appends_prompt_and_keeps_auto(self):
        original={'messages':[{'role':'system','content':'old-session\n'+x.c.POLICY},
            {'role':'user','content':'same projection and question'}],'tools':x.e.t.FUTURE_TOOLS,
            'tool_choice':'auto','model':'future/deepseek-flash','max_tokens':8192}
        frozen=copy.deepcopy(original)
        control=x.make_body(original,False,'old-session','same-session')
        stronger=x.make_body(original,True,'old-session','same-session')
        self.assertEqual(stronger['messages'][0]['content'],control['messages'][0]['content']+'\n\n'+x.AUDIT)
        self.assertEqual(stronger['messages'][1],control['messages'][1])
        self.assertEqual(stronger['tools'],control['tools'])
        self.assertEqual(stronger['tool_choice'],'auto')
        self.assertEqual(original,frozen)
        self.assertIn('Determine the unresolved candidates yourself',x.AUDIT)
        self.assertIn('not a target number of calls',x.AUDIT)
        self.assertIn('Do not query a value already settled',x.AUDIT)


if __name__=='__main__': unittest.main()
