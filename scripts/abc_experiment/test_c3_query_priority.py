import copy
import unittest

import c3_query_priority as c


class PriorityTests(unittest.TestCase):
    def test_policy_only_adds_system_text(self):
        original={'messages':[{'role':'system','content':'source guide, old-session'},{'role':'user','content':'unchanged projection and candidate question'}],
                  'tools':c.e.t.FUTURE_TOOLS,'tool_choice':'auto','model':'future/deepseek-flash','max_tokens':8192}
        before=copy.deepcopy(original)
        control=c.make_body(original,False,'old-session','same-session')
        policy=c.make_body(original,True,'old-session','same-session')
        self.assertEqual(policy['messages'][0]['content'],control['messages'][0]['content']+'\n\n'+c.POLICY)
        self.assertEqual(policy['messages'][1:],control['messages'][1:])
        self.assertEqual(policy['tools'],control['tools'])
        self.assertEqual(policy['tool_choice'],'auto')
        self.assertEqual(original,before)
        self.assertIn('no target number of calls',c.POLICY)
        self.assertIn('without re-reading them',c.POLICY)
        self.assertIn('silence about a candidate is not negative evidence',c.POLICY)


if __name__=='__main__': unittest.main()
