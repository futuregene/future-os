import copy
from pathlib import Path
import tempfile
import unittest

import autonomous_open_exam as a


BODY={'model':'future/deepseek-flash','messages':[{'role':'system','content':'same system'},
    {'role':'user','content':'same projection and original question'}],
    'thinking':{'type':'disabled'},'max_tokens':8192,'stream':True,
    'stream_options':{'include_usage':True}}


class AutonomousTests(unittest.TestCase):
    def test_initial_request_only_adds_tools_and_auto(self):
        before=copy.deepcopy(BODY)
        request=a.open_body(BODY,a.schemas('C3'))
        a.assert_initial_parity(BODY,request,a.schemas('C3'))
        self.assertEqual(BODY,before)
        self.assertEqual(set(request)-set(BODY),{'tools','tool_choice'})
        self.assertEqual(request['messages'],BODY['messages'])
        self.assertNotIn('assistant',[m['role'] for m in request['messages']])
        self.assertEqual(a.schemas('C'),a.schemas('C3'))
        names={t['function']['name'] for t in a.schemas('codex')}
        self.assertEqual(names,{'history_list_windows','history_list_items','history_read_item','history_search_contents'})
        self.assertNotIn('exec_command',names)

    def test_no_tool_use_is_accepted_without_forcing_a_retry(self):
        class Fake:
            def __init__(self): self.requests=[]
            def request(self,identity,body):
                self.requests.append(copy.deepcopy(body))
                return {'text':'{"appeared":["already known"]}','calls':[],'finish':'stop'}
        def forbidden(*args): raise AssertionError('model did not request retrieval')
        with tempfile.TemporaryDirectory() as root:
            calls=Fake(); result=a.answer(calls,'unit',BODY,a.schemas('C'),forbidden,Path(root))
            self.assertEqual(result['logical_queries'],0)
            self.assertEqual(result['model_turns'],1)
            self.assertFalse(result['lookup_used'])
            self.assertTrue(result['valid_answer'])
            a.assert_initial_parity(BODY,calls.requests[0],a.schemas('C'))

    def test_tool_result_reaches_next_turn_but_no_answer_is_injected(self):
        class Fake:
            def __init__(self): self.requests=[]
            def request(self,identity,body):
                self.requests.append(copy.deepcopy(body))
                if len(self.requests)==1:
                    return {'text':'','calls':[{'id':'call1','type':'function','function':{'name':'history_search','arguments':'{"query":"found"}'}}]}
                return {'text':'{"appeared":["found"]}','calls':[],'finish':'stop'}
        with tempfile.TemporaryDirectory() as root:
            calls=Fake(); result=a.answer(calls,'unit',BODY,a.schemas('C3'),lambda n,x:'{"matches":[{"snippet":"found"}]}',Path(root))
            self.assertEqual(result['logical_queries'],1)
            self.assertEqual(result['model_turns'],2)
            self.assertEqual(calls.requests[1]['messages'][:2],BODY['messages'])
            self.assertEqual(calls.requests[1]['messages'][-1]['role'],'tool')
            self.assertEqual(calls.requests[1]['tool_choice'],'auto')

    def test_evaluation_pairing_does_not_feed_back_into_generation(self):
        result={'answer':{'appeared':['old','new']}}
        out=a.evaluate(result,{'present':['old','new'],'decoys':['wrong']},
                       {'answer':{'appeared':['old']},'hits':1,'false_positives':0},'old',['new'])
        self.assertEqual((out['gained'],out['lost'],out['net_gain']),(1,0,1))
        self.assertEqual(out['reachable'],2)


if __name__=='__main__': unittest.main()
