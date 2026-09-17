import copy
from pathlib import Path
import tempfile
import unittest

import full_history_exam as x


class FullHistoryTests(unittest.TestCase):
    def test_new_closed_open_question_and_candidates_match(self):
        old='Old instructions.\nValues:\n- alpha\n- beta\n- 37 MB'
        question=x.question_from(old)
        self.assertEqual(x.d.candidates(question),x.d.candidates(old))
        self.assertIn('COMPLETE original session',question)
        self.assertIn('If read-only history or file lookup tools are provided',question)
        original={'model':'future/deepseek-flash','messages':[{'role':'system','content':'old'},{'role':'user','content':'old text'}],
                  'max_tokens':8192,'thinking':{'type':'disabled'},'stream':True}
        unchanged=copy.deepcopy(original)
        closed=x.body_from(original,'projection',question)
        base=x.body_from(original,'projection',question,'source guidance')
        opened=x.a.open_body(base,x.a.schemas('C3'))
        x.g.assert_guidance_parity(closed,opened,x.a.schemas('C3'),'source guidance')
        self.assertEqual(closed['messages'][1],opened['messages'][1])
        self.assertNotIn('tools',closed)
        self.assertNotIn('tool_choice',closed)
        self.assertEqual(opened['tool_choice'],'auto')
        self.assertEqual([m['role'] for m in opened['messages']],['system','user'])
        self.assertEqual(original,unchanged)

    def test_pairing_only_after_both_independent_answers_exist(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); prefix='chain-0-C3-full'
            x.b.save(root/'manifest.json',{'chains':['chain'],'jobs':[{'identity':prefix+'-closed'},{'identity':prefix+'-open'}],'opening_spend':0})
            question={'present':['a','b'],'decoys':['c'],'prompt':'test'}
            x.b.save(root/'questions/chain-0.json',question)
            def save(mode,answer):
                result={'answer':{'appeared':answer},'valid_answer':True,'model_turns':1,'logical_queries':0,
                        'returned_bytes':0,'tool_errors':0,'lookup_used':False}
                x.b.save(root/'answers'/f'{prefix}-{mode}.json',dict(x.grade(result,question,'a',['b']),
                    identity=prefix+'-'+mode,chain='chain',stage=0,arm='C3',mode=mode))
            save('open',['a','b'])
            partial=x.report(root)
            self.assertEqual(partial['pairs_complete'],0)
            self.assertFalse(partial['complete'])
            save('closed',['a'])
            final=x.report(root)
            self.assertTrue(final['complete'])
            self.assertEqual(final['arms']['C3']['delta']['net_gain'],1)
            self.assertEqual(final['arms']['C3']['delta']['gained'],1)


if __name__=='__main__': unittest.main()
