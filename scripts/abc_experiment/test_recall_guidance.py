import copy
from pathlib import Path
import tempfile
import unittest

import guided_open_exam as g
from recall_guidance import Guidance


class _StubShape:
    """Stands in for production_shape.RequestShape so this unit test needs no Rust build.

    The real shape is checked against the Rust code by `verify_production_shape.py`. What
    matters here is only that Guidance consults it and refuses when the runtime has started
    appending a guidance again.
    """
    TOOLS = [{"type": "function", "function": {"name": n}} for n in ("read", "write", "edit", "shell")]

    def __init__(self, system_prompt):
        self._system = system_prompt

    def system_prompt(self, has_checkpoint=True):
        return self._system

    def guidance(self, has_checkpoint=True):
        return "" if has_checkpoint else ""

    def tools(self):
        return self.TOOLS


BASE_PROMPT = "You are an expert coding assistant."


class GuidanceTests(unittest.TestCase):
    def test_future_arm_appends_nothing_and_guards_against_regression(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); codex = root / 'codex'; opencode = root / 'opencode'
            specs = {
                codex / 'codex-rs/ext/history-notes/src/tools.rs': 'const HISTORY_DESCRIPTION: &str = "Recover prior conversation after a context-window reset. Pass returned IDs unchanged. Calls use the current agent by default; backend-only tail.";',
                codex / 'codex-rs/ext/history-notes/src/extension.rs': 'thread_hint comes from the backend',
                opencode / 'packages/opencode/src/tool/glob.txt': '- Supports glob patterns\n- Use the Task tool instead\n',
                opencode / 'packages/opencode/src/tool/grep.txt': '- Searches file contents using regular expressions\n- Use the Bash tool for counts\n',
                opencode / 'packages/opencode/src/tool/read.txt': '- The offset parameter is 1-indexed\n- This tool can read image files and PDFs\n',
                opencode / 'packages/opencode/src/session/compaction.ts': 'continue synthetic user message',
            }
            for path, text in specs.items():
                path.parent.mkdir(parents=True, exist_ok=True); path.write_text(text)

            # The runtime stops appending the recall guidance, so the Future arms add nothing.
            guides = Guidance(g.b.REPO, codex, opencode, shape=_StubShape(BASE_PROMPT))
            self.assertEqual(guides.text('C3', 'case-id'), '')
            self.assertEqual(guides.text('C', 'case-id'), '')
            self.assertEqual(guides.text('C3', 'case-id', False), '')
            self.assertEqual(guides.future_mappings, {}, 'no routing substitutions remain')

            # If a guidance ever reappears, the exam must refuse rather than silently
            # measuring a different request shape than it recorded.
            regressed = Guidance(g.b.REPO, codex, opencode,
                                 shape=_StubShape(BASE_PROMPT + '\n\n## Archived conversation recall\n...'))
            with self.assertRaises(ValueError):
                regressed.text('C3', 'case-id')

            # Codex/OpenCode keep their own products' guidance.
            self.assertIn('backend-only tail', guides.adaptations()['codex']['omitted_source_tail'])
            self.assertNotIn('backend-only tail', guides.text('codex', 'case-id'))

            base={'model':'future/deepseek-flash','messages':[{'role':'system','content':'base'},{'role':'user','content':'projection + original question'}],'max_tokens':8192}
            original=copy.deepcopy(base)
            tools=g.a.schemas('C3',_StubShape(BASE_PROMPT))
            amended=g.guided_base(base,guides.text('C3','case-id')); body=g.a.open_body(amended,tools)
            # With no guidance to append, amending must leave the body untouched.
            self.assertEqual(amended, base)
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
