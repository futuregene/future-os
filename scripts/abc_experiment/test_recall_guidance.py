import copy
from pathlib import Path
import tempfile
import unittest

import guided_open_exam as g
from recall_guidance import Guidance


class GuidanceTests(unittest.TestCase):
    def test_source_mapping_and_request_boundary(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); codex=root/'codex'; opencode=root/'opencode'
            specs={
                codex/'codex-rs/ext/history-notes/src/tools.rs':'const HISTORY_DESCRIPTION: &str = "Recover prior conversation after a context-window reset. Pass returned IDs unchanged. Calls use the current agent by default; backend-only tail.";',
                codex/'codex-rs/ext/history-notes/src/extension.rs':'thread_hint comes from the backend',
                opencode/'packages/opencode/src/tool/glob.txt':'- Supports glob patterns\n- Use the Task tool instead\n',
                opencode/'packages/opencode/src/tool/grep.txt':'- Searches file contents using regular expressions\n- Use the Bash tool for counts\n',
                opencode/'packages/opencode/src/tool/read.txt':'- The offset parameter is 1-indexed\n- This tool can read image files and PDFs\n',
                opencode/'packages/opencode/src/session/compaction.ts':'continue synthetic user message',
            }
            for path,text in specs.items(): path.parent.mkdir(parents=True,exist_ok=True); path.write_text(text)
            guides=Guidance(g.b.REPO,codex,opencode)
            future=guides.text('C3','case-id')
            self.assertEqual(future,guides.text('C','case-id'))
            self.assertIn('Earlier context was compacted',future)
            self.assertIn('Only when exact earlier',future)
            self.assertIn('original records returned by these history commands are permitted evidence',future)
            self.assertIn('search before concluding that the answer is unknown',future)
            self.assertIn('do not search merely because compaction occurred',future)
            self.assertIn('history_search(query=',future)
            self.assertIn('history_get(entry_id=',future)
            self.assertNotIn('existing shell tool',future)
            self.assertNotIn('--session',future)
            self.assertEqual(guides.text('C3','case-id',False),'')
            self.assertIn('backend-only tail',guides.adaptations()['codex']['omitted_source_tail'])
            self.assertNotIn('backend-only tail',guides.text('codex','case-id'))
            base={'model':'future/deepseek-flash','messages':[{'role':'system','content':'base'},{'role':'user','content':'projection + original question'}],'max_tokens':8192}
            original=copy.deepcopy(base)
            amended=g.guided_base(base,future); body=g.a.open_body(amended,g.a.schemas('C3'))
            g.assert_guidance_parity(base,body,g.a.schemas('C3'),future)
            self.assertEqual(base,original)
            self.assertEqual(body['messages'][1],base['messages'][1])
            self.assertEqual(len(body['messages']),2)
            self.assertEqual(body['tool_choice'],'auto')
            self.assertNotIn('assistant',[message['role'] for message in body['messages']])
            self.assertEqual(g.guided_base(base,''),base)


class LaterSpendTests(unittest.TestCase):
    def test_intervening_ledgers_are_chained_not_dropped(self):
        with tempfile.TemporaryDirectory() as directory:
            roots=[Path(directory)/name for name in ('one','two')]
            for root,opening,cost in zip(roots,(10.0,10.2),(.2,.3)):
                g.b.save(root/'manifest.json',{'opening_spend':opening})
                g.b.save(root/'ledger.json',{'x':{'state':'finished','reserved':1,'charged':cost}})
                g.b.save(root/'verified-report.json',{'complete':True,'artifact_consistent':True,'total_spend':opening+cost})
            total,receipts=g.account_for_later_runs(10.0,roots)
            self.assertAlmostEqual(total,10.5)
            self.assertEqual(len(receipts),2)
            with self.assertRaisesRegex(ValueError,'duplicate'):
                g.account_for_later_runs(10.0,[roots[0],roots[0]])
            with self.assertRaisesRegex(ValueError,'discontinuous'):
                g.account_for_later_runs(10.0,list(reversed(roots)))
            g.b.save(roots[0]/'ledger.json',{'x':{'state':'started','reserved':1}})
            with self.assertRaisesRegex(ValueError,'unsettled'):
                g.account_for_later_runs(10.0,roots)


if __name__=='__main__': unittest.main()
