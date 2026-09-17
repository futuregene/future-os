"""No-model integration tests for the production Rust Future shell adapter."""
import json
import os
from pathlib import Path
import tempfile
import unittest

from native_future_shell import NativeFutureShell

REPO=Path(__file__).resolve().parents[2]
ENABLED=os.environ.get('NATIVE_FUTURE_SHELL_TESTS')=='1'


@unittest.skipUnless(ENABLED,'enable after building abc_future_shell_probe and future')
class NativeFutureShellTests(unittest.TestCase):
    def test_native_shell_batch_pipe_loop_paging_and_errors(self):
        base=Path.home()/'compact-exp/native-open/preflight'
        base.mkdir(parents=True,exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='future-shell-',dir=base) as directory:
            root=Path(directory)
            # Create before scope construction so the explicit sibling denial
            # is fixed before invoking the native handler.
            outside=root/'outside.txt'; outside.write_text('OUTSIDE_SHOULD_NOT_BE_READ_739')
            messages=[{'role':'user','content':[{'type':'text','text':'BATCH_ALPHA_5123 BATCH_BETA_8264 中文'}]}]
            engine=NativeFutureShell(REPO/'target/fidelity/debug/future',
                REPO/'target/fidelity/debug/examples/abc_future_shell_probe',
                root/'work',root/'home',root/'control',messages,'native-shell-test')
            try:
                self.assertEqual(engine.tool['function']['name'],'shell')
                self.assertIn('timeout',engine.tool['function']['parameters']['properties'])
                query='future session history search --session native-shell-test --query '
                result=engine.execute_shell({'command':query+'BATCH_ALPHA_5123 --json; '+query+'BATCH_BETA_8264 --json'})
                self.assertEqual(result['exit_code'],0)
                self.assertEqual(result['output'].count('"matches"'),2)
                self.assertNotIn('unknown option',result['output'])
                result=engine.execute_shell({'command':query+"BATCH_ALPHA_5123 --json | jq -r '.matches[0].entryId'"})
                self.assertEqual(result['exit_code'],0)
                self.assertIn('entry-00000000',result['output'])
                loop='for term in BATCH_ALPHA_5123 BATCH_BETA_8264; do '+query+'"$term" --json; done'
                result=engine.execute_shell({'command':loop})
                self.assertEqual(result['exit_code'],0)
                self.assertEqual(result['output'].count('"matches"'),2)
                result=engine.execute_shell({'command':'future session history get --session native-shell-test --entry entry-00000000 --offset 17 --limit 8192 --json'})
                self.assertEqual(result['exit_code'],0)
                self.assertIn('BATCH_BETA_8264 中文',result['output'])
                result=engine.execute_shell({'command':'future session history search --session foreign --query x --json'})
                self.assertEqual(result['exit_code'],2)
                self.assertIn('STUDY_SCOPE_DENIED',result['output'])
                self.assertEqual(engine.execute_shell({'command':'false'})['exit_code'],1)
                no_match=engine.execute_shell({'command':'grep -F NEVER_PRESENT_SENTINEL native-bin/future'})
                self.assertEqual(no_match['exit_code'],1)
                self.assertTrue(no_match['is_soft_fail'])
                # Native shell semantics: final status alone does not prove all
                # commands in a sequence succeeded.
                self.assertEqual(engine.execute_shell({'command':'false; echo final-command'})['exit_code'],0)
                denied=engine.execute_shell({'command':f'cat "{outside}"'})
                self.assertNotEqual(denied['exit_code'],0)
                self.assertNotIn('OUTSIDE_SHOULD_NOT_BE_READ_739',denied['output'])
                denied=engine.execute_shell({'command':'touch should-not-exist'})
                self.assertNotEqual(denied['exit_code'],0)
                self.assertFalse((root/'work/should-not-exist').exists())
            finally:
                engine.close()


if __name__=='__main__': unittest.main()
