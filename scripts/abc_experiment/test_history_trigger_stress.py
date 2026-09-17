import json
import unittest

import history_trigger_stress as s


class StressTests(unittest.TestCase):
    def test_balanced_fixed_matrix_and_held_out_facts(self):
        jobs=s.jobs()
        self.assertEqual(len(jobs),26)
        self.assertEqual(len({job['id'] for job in jobs}),26)
        for length in ('short','long'):
            for policy in (False,True):
                for visible in (False,True):
                    self.assertEqual(sum(not j['absent'] and j['length']==length and j['policy']==policy and j['visible']==visible for j in jobs),3)
        self.assertEqual(len(set(f['value'] for f in s.FACTS)&set(f['value'] for f in s.c.FACTS)),0)

    def test_no_hidden_answers_and_long_context_size(self):
        for job in s.jobs():
            messages=s.messages_for(job,'NATIVE_GUIDE')
            self.assertEqual([m['role'] for m in messages],['system','user'])
            fact=s.FACTS[job['fact']]
            if job['visible']: self.assertIn(fact['value'],messages[1]['content'])
            else: self.assertNotIn(fact['value'],json.dumps(messages,ensure_ascii=False))
            self.assertEqual(s.c.POLICY in messages[0]['content'],job['policy'])
            if job['length']=='long': self.assertGreaterEqual(len(messages[1]['content']),60000)
            else: self.assertLess(len(messages[1]['content']),2000)
        self.assertNotIn('Willow-check',json.dumps(s.fixture(),ensure_ascii=False))

    def test_policy_only_changes_system_not_question_or_projection(self):
        job={'id':'test','fact':1,'length':'long','policy':False,'visible':False,'absent':False}
        before=s.messages_for(job,'guide'); after=s.messages_for(dict(job,policy=True),'guide')
        self.assertEqual(before[1],after[1])
        self.assertEqual(after[0]['content'],before[0]['content']+'\n\n'+s.c.POLICY)


if __name__=='__main__': unittest.main()
