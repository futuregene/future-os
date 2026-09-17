import json
import unittest

import history_trigger_calibration as c


class CalibrationTests(unittest.TestCase):
    def test_predeclared_matrix_and_no_hidden_answer_in_natural_prompts(self):
        rows=c.jobs()
        self.assertEqual(len(rows),12)
        self.assertEqual(len({r['id'] for r in rows}),12)
        for row in rows:
            messages=c.messages_for(row,'Read-only archive tools are available.')
            self.assertEqual([m['role'] for m in messages],['system','user'])
            if not row['visible'] and row['mode']!='recognition':
                self.assertNotIn(c.FACTS[row['fact']]['value'],json.dumps(messages,ensure_ascii=False))
            self.assertNotIn('required',json.dumps(messages,ensure_ascii=False))

    def test_positive_context_control_contains_the_answer(self):
        row=next(r for r in c.jobs() if r['id']=='visible-control')
        messages=c.messages_for(row,'guide')
        self.assertIn(c.FACTS[0]['value'],messages[1]['content'])
        self.assertIn('answer directly without redundant lookup',messages[0]['content'])

    def test_unknown_control_is_not_in_the_native_fixture(self):
        self.assertNotIn('Maple-audit',json.dumps(c.fixture(),ensure_ascii=False))
        row=next(r for r in c.jobs() if r['id']=='closed-control')
        system=c.messages_for(row,'guide')[0]['content']
        self.assertIn('No history tools are available',system)
        self.assertNotIn(c.POLICY,system)


if __name__=='__main__': unittest.main()
