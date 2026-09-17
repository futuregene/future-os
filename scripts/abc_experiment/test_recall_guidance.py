import copy
from pathlib import Path
import tempfile
import unittest

import guided_open_exam as g
from recall_guidance import Guidance


class _StubShape:
    """Stands in for production_shape.RequestShape so this unit test needs no Rust build.

    What the real shape returns is verified against the Rust code by
    `verify_production_shape.py`; here we only check that Guidance hands it through
    unmodified instead of rewriting it.
    """
    # A tool set that is obviously not production's, so a Future arm accepting it would be
    # caught here rather than in a paid run.
    TOOLS = [{"type": "function", "function": {"name": "stub_tool"}}]

    def __init__(self,text): self._text=text
    def system_prompt(self,has_checkpoint=True): return self._text if has_checkpoint else ''
    def guidance(self,has_checkpoint=True): return self._text if has_checkpoint else ''
    def tools(self): return self.TOOLS


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
            # Production's own text, with the real commands and the shell tool named: the
            # Future arms used to rewrite this into adapters the product does not have, and
            # the assertions below now forbid exactly that.
            native=('## Archived conversation recall\nCurrent session ID (literal): "case-id".\n'
                    'Only when exact earlier requirements are missing, use the existing shell tool:\n'
                    '- Find a record: `future session history search --session <current-session-id> --query <specific-keywords> --limit 5 --json`\n'
                    '- Read it: `future session history get --session <current-session-id> --entry <entryId> --json`\n'
                    'Search before concluding that the answer is unknown or omitting it as unverifiable.')
            guides=Guidance(g.b.REPO,codex,opencode,shape=_StubShape(native))
            future=guides.text('C3','case-id')
            self.assertEqual(future,guides.text('C','case-id'))
            self.assertEqual(future,native,'the Future arm must be served production text verbatim')
            self.assertIn('existing shell tool',future)
            self.assertIn('--session',future)
            self.assertNotIn('history_search(query=',future)
            self.assertNotIn('history_get(entry_id=',future)
            self.assertEqual(guides.future_mappings,{},'no routing substitutions remain')
            self.assertEqual(guides.text('C3','case-id',False),'')
            self.assertIn('backend-only tail',guides.adaptations()['codex']['omitted_source_tail'])
            self.assertNotIn('backend-only tail',guides.text('codex','case-id'))
            # Without a shape the Future arm must refuse rather than paraphrase.
            with self.assertRaises(ValueError):
                Guidance(g.b.REPO,codex,opencode).text('C3','case-id')
            base={'model':'future/deepseek-flash','messages':[{'role':'system','content':'base'},{'role':'user','content':'projection + original question'}],'max_tokens':8192}
            original=copy.deepcopy(base)
            tools=g.a.schemas('C3',_StubShape('irrelevant; only tools are read here'))
            amended=g.guided_base(base,future); body=g.a.open_body(amended,tools)
            g.assert_guidance_parity(base,body,tools,future)
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
