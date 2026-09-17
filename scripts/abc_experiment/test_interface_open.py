import json
from pathlib import Path
import tempfile
import unittest

import interface_open_tools as t


def record(i,role,kind,**fields): return dict(id=str(i),role=role,kind=kind,**fields)


class InterfaceTests(unittest.TestCase):
    def test_codex_window_scope_case_and_unicode_read(self):
        rows=[record(0,'user','text',text='Alpha中文😀'),record(1,'assistant','text',text='later Beta')]
        api=t.CodexHistory(rows,[1,2])
        self.assertEqual(json.loads(api.call('history_search_contents',{'query':'alpha'}))['matched'],0)
        found=json.loads(api.call('history_search_contents',{'query':'Alpha','window_id':'w000'}))
        self.assertEqual(found['matched'],1)
        wrong=json.loads(api.call('history_read_item',{'item_id':'i000000','window_id':'w001'}))
        self.assertIn('error',wrong)
        read=json.loads(api.call('history_read_item',{'item_id':'i000000','window_id':'w000','offset_chars':5,'limit_chars':3}))
        self.assertEqual(read['content'],'中文😀')
        self.assertEqual(read['next_offset_chars'],8)
        self.assertIn('error',json.loads(api.call('history_list_windows',{'agent_name':'other'})))

    def test_tool_filters_apply_to_results_without_future_records(self):
        rows=[record(0,'assistant','tool_call',call='c',tool='functions.read',path='a.txt'),
              record(1,'tool','tool_result',call='c',text='RESULT_SENTINEL'),record(2,'user','text',text='future-only')]
        api=t.CodexHistory(rows[:2],[2])
        result=json.loads(api.call('history_search_contents',{'query':'RESULT','role':'tool','tool_name':'read','tool_namespace':'functions'}))
        self.assertEqual(result['matched'],1)
        self.assertEqual(json.loads(api.call('history_search_contents',{'query':'future-only'}))['matched'],0)

    def test_files_preserve_directories_and_invalidate_changed_reads(self):
        rows=[record(0,'assistant','tool_call',call='a',tool='read',path='one/config.txt'),
              record(1,'tool','tool_result',call='a',text='ONE'),
              record(2,'assistant','tool_call',call='b',tool='read',path='two/config.txt'),
              record(3,'tool','tool_result',call='b',text='TWO'),
              record(4,'assistant','tool_call',call='c',tool='shell',path=''),
              record(5,'tool','tool_result',call='c',text='DO_NOT_INVENT_A_FILE')]
        with tempfile.TemporaryDirectory() as root:
            files=t.ObservedFiles(rows,Path(root))
            self.assertEqual(len(files.files),2)
            self.assertNotIn('tool-output.log',files.files)
            self.assertIn('ONE',files.call('read',{'filePath':'one/config.txt'}))
            self.assertIn('TWO',files.call('grep',{'pattern':'TWO','path':'/workspace/two'}))
            listing=files.call('glob',{'pattern':'**/config.txt'})
            self.assertIn('/workspace/one/config.txt',listing)
            self.assertIn('/workspace/two/config.txt',listing)
        with tempfile.TemporaryDirectory() as root:
            rows.append(record(6,'assistant','tool_call',call='d',tool='edit',path='one/config.txt'))
            files=t.ObservedFiles(rows,Path(root))
            self.assertNotIn('/workspace/one/config.txt',files.files)
            self.assertEqual(files.invalidated,1)

    def test_local_budget_returns_complete_json(self):
        api=t.CodexHistory([record(i,'assistant','text',text='中文'*20000) for i in range(20)],[20])
        output=api.call('history_list_items',{'limit':20,'max_chars_per_item':16384})
        json.loads(output)
        self.assertLessEqual(len(output.encode()),t.MAX_REPLY)
        with self.assertRaises(ValueError): t.virtual_path('../../outside')


if __name__=='__main__': unittest.main()
