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


if __name__=='__main__': unittest.main()
