import json
from pathlib import Path
import tempfile
import unittest

import paired_retrieval_exam as p


PROMPT='Verify candidates.\nValues:\n- Known\n- Missing\n- Decoy'


class PairedTests(unittest.TestCase):
    def test_pending_is_public_and_label_independent(self):
        self.assertEqual(p.pending_candidates(PROMPT,'Known'),['Missing','Decoy'])
        self.assertEqual(p.public_candidates(PROMPT),['Known','Missing','Decoy'])

    def test_query_echo_does_not_verify_other_candidates(self):
        result=json.dumps({'query':'Decoy','matches':[]})
        self.assertEqual(p.checked_candidates('history_search',{'query':'Decoy'},result,{'Decoy','Missing'}),{'Decoy'})
        result=json.dumps({'query':'Missing','matches':[]})
        # The mismatched query echo is not original evidence.
        self.assertEqual(p.checked_candidates('history_search',{'query':'Other'},result,{'Missing'}),set())
        result=json.dumps({'chunks':[{'text':'Missing'}]})
        self.assertEqual(p.checked_candidates('history_get',{},result,{'Missing'}),{'Missing'})

    def test_literal_regex_spellings_are_equivalent(self):
        for pattern,value in [(r'16\.5\.2-diag','16.5.2-diag'),(r'16\.5\.2\-diag','16.5.2-diag'),
                              ('6 GB','6 GB'),(r'6\ GB','6 GB'),(r'cli\/commands','cli/commands'),
                              (r'^lib/adapter\.rs$','lib/adapter.rs')]:
            self.assertEqual(p.literal_regex_values(pattern),{value})
            self.assertEqual(p.checked_candidates('grep',{'pattern':pattern},'No files found',{value}),{value})
        self.assertEqual(p.literal_regex_values(r'(foo|bar)\.txt'),{'foo.txt','bar.txt'})
        self.assertEqual(p.checked_candidates('grep',{'pattern':r'harness-research\.md'},'No files found',{'./harness-research.md'}),{'./harness-research.md'})
        self.assertEqual(p.literal_regex_values('.*'),set())
        self.assertEqual(p.literal_regex_values(r'foo.*'),set())
        self.assertEqual(p.literal_regex_values(r'(?i)foo'),set())

    def test_gains_losses_and_false_positives_are_separate(self):
        result=p.paired_delta({'appeared':['a','b','d']},{'appeared':['b','c','e']},{'present':['a','b','c'],'decoys':['d','e']})
        self.assertEqual((result['gained'],result['lost'],result['net_gain']),(1,1,0))
        self.assertEqual((result['new_false_positives'],result['corrected_false_positives']),(1,1))

    def test_required_retrieval_precedes_scored_revision(self):
        class FakeCalls:
            def __init__(self): self.requirements=[]
            def model_call(self,identity,messages,tools=None,require_tool=False):
                self.requirements.append(require_tool)
                if len(self.requirements)==1:
                    self.assert_initial=messages[2]['content']
                    return {'text':'','calls':[{'id':'a','function':{'name':'history_search','arguments':json.dumps({'query':v,'limit':1})}} for v in ('Missing','Decoy')]}
                return {'text':'{"appeared":["Known","Missing"]}','calls':[]}
        def dispatch(name,args):
            return json.dumps({'query':args['query'],'matches':[{'snippet':'Missing'}] if args['query']=='Missing' else []})
        with tempfile.TemporaryDirectory() as root:
            calls=FakeCalls()
            result=p.augment(calls,'unit',{'text':'Known'},{'prompt':PROMPT,'present':['Known','Missing'],'decoys':['Decoy']},
                {'answer':{'appeared':['Known']},'hits':1,'false_positives':0},p.t.FUTURE_TOOLS,'',dispatch,['Known Missing'],Path(root))
            self.assertEqual(calls.requirements,[True,False])
            self.assertEqual(json.loads(calls.assert_initial),{'appeared':['Known']})
            self.assertEqual(result['gained'],1)
            self.assertTrue(result['verification_attempts_complete'])
            self.assertEqual(result['logical_queries'],2)


if __name__=='__main__': unittest.main()
