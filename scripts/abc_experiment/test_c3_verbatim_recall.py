import copy
import unittest

import c3_verbatim_recall as x


class VerbatimTests(unittest.TestCase):
    def test_only_prompt_changes_with_unchanged_user_data_and_auto_tools(self):
        original={'messages':[{'role':'system','content':'guide old-session'},{'role':'user','content':'same real projection and values'}],
            'tools':x.e.t.FUTURE_TOOLS,'tool_choice':'auto','model':'future/deepseek-flash','max_tokens':8192}
        frozen=copy.deepcopy(original)
        for variant in ('head','tail','example-tail'):
            result=x.make_body(original,'old-session','new-session',variant)
            self.assertEqual(result['messages'][1],original['messages'][1])
            self.assertEqual(result['tools'],original['tools'])
            self.assertEqual(result['tool_choice'],'auto')
            self.assertEqual(len(result['messages']),2 if variant=='head' else 3)
            self.assertFalse(any(m['role']=='assistant' for m in result['messages']))
            self.assertIn(x.RULES,result['messages'][0]['content'] if variant=='head' else result['messages'][-1]['content'])
        self.assertEqual(original,frozen)
        self.assertIn('CURRENT lookup arguments',x.RULES)
        self.assertIn('Original historical tool-call arguments',x.RULES)
        self.assertIn('matches=[]',x.RULES)


if __name__=='__main__': unittest.main()
