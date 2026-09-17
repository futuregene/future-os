import copy
import unittest

import nonuse_diagnosis as d


class NonuseTests(unittest.TestCase):
    def test_controlled_prompt_changes_and_public_target(self):
        projection='Known visible history'
        question='Original instruction.\nValues:\n- Known\n- Missing\n- Other'
        original={'model':'future/deepseek-flash','messages':[{'role':'system','content':d.b.SYSTEM+'\nNative hint, session original-id'},
            {'role':'user','content':projection+'\n\n'+question}],'tools':d.e.t.FUTURE_TOOLS,'tool_choice':'auto','max_tokens':8192}
        frozen=copy.deepcopy(original)
        for allow in (False,True):
            for framing in ('original','full','single'):
                body,target=d.make_body(original,projection,question,allow,framing,'original-id','test-id')
                self.assertEqual(target,'Missing')
                self.assertEqual(body['tools'],original['tools'])
                self.assertEqual(body['tool_choice'],'auto')
                self.assertTrue(body['messages'][1]['content'].startswith(projection+'\n\n'))
                self.assertEqual([x['role'] for x in body['messages']],['system','user'])
                self.assertEqual(body['messages'][0]['content'].startswith(d.BASE_ALLOWED),allow)
                if framing=='original': self.assertEqual(body['messages'][1],original['messages'][1])
                if framing=='full': self.assertEqual(d.candidates(body['messages'][1]['content']),d.candidates(question))
        self.assertEqual(original,frozen)


if __name__=='__main__': unittest.main()
