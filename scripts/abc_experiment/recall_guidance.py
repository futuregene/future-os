"""Source-derived functional recall guidance for the existing local interfaces.

Not a claim of full native hosted/client prompt equivalence. Every omission or
routing adaptation is recorded; no answer, query checklist or mandatory lookup.
"""
import json
from pathlib import Path
import re


class Guidance:
    def __init__(self,future_source,codex_source,opencode_source):
        self.paths={
            'future':Path(future_source)/'agent/src/agent/history_recall.rs',
            'future_gate':Path(future_source)/'agent/src/agent/run_loop.rs',
            'codex':Path(codex_source)/'codex-rs/ext/history-notes/src/tools.rs',
            'codex_hint':Path(codex_source)/'codex-rs/ext/history-notes/src/extension.rs',
            'opencode_glob':Path(opencode_source)/'packages/opencode/src/tool/glob.txt',
            'opencode_grep':Path(opencode_source)/'packages/opencode/src/tool/grep.txt',
            'opencode_read':Path(opencode_source)/'packages/opencode/src/tool/read.txt',
            'opencode_continue':Path(opencode_source)/'packages/opencode/src/session/compaction.ts',
        }
        self.source={key:path.read_text() for key,path in self.paths.items()}
        match=re.search(r'Cow::Owned\(format!\("((?:\\.|[^"\\])*)"\)\)',self.source['future'],re.S)
        if not match: raise ValueError('native Future recall format changed; review before running')
        raw=re.sub(r'\\\r?\n[ \t]*','',match.group(1))
        self.future=json.loads('"'+raw+'"')
        assert self.future.startswith('{base}\n\n')
        self.future=self.future.removeprefix('{base}\n\n')
        mappings={
            'use the existing shell tool:':'use these read-only adapters of the native Future history CLI:',
            '`future session history search --session <current-session-id> --query <specific-keywords> --limit 5 --json`':'`history_search(query=<specific-keywords>, limit=5)`',
            '`future session history get --session <current-session-id> --entry <entryId> --json`':'`history_get(entry_id=<entryId>, offset=0, limit=8192)`',
            'Quote arguments for the host shell.':'Pass arguments as JSON. Session scope is fixed by the adapter.',
            'use nextOffset as --offset':'use nextOffset as offset',
            'If these CLI commands are unavailable':'If these query tools are unavailable',
        }
        for old,new in mappings.items():
            if old not in self.future: raise ValueError('native Future recall text changed: '+old)
            self.future=self.future.replace(old,new)
        assert 'shell tool' not in self.future and '--session' not in self.future
        self.future_mappings=mappings
        match=re.search(r'const HISTORY_DESCRIPTION: &str = "((?:\\.|[^"\\])*)";',self.source['codex'])
        if not match: raise ValueError('Codex namespace description changed')
        self.codex_description=json.loads('"'+match.group(1)+'"')
        boundary=' Calls use the current agent by default;'
        if boundary not in self.codex_description: raise ValueError('Codex description boundary changed')
        # Preserve the native recovery/ID/ordering instructions. The tail includes
        # cross-agent access, eventual consistency and hidden model-only-state
        # nondisclosure rules not implemented by this user-owned local fixture.
        self.codex_functional=self.codex_description.split(boundary,1)[0]
        self.opencode_selected={}; self.opencode_omitted={}
        for name in ('glob','grep','read'):
            lines=self.source['opencode_'+name].splitlines()
            if name=='glob':
                selected=[line for line in lines if line.startswith('- ') and 'Task tool' not in line]
            elif name=='grep':
                selected=[line for line in lines if line.startswith('- ') and 'Task tool' not in line and 'Bash tool' not in line]
            else:
                selected=[line for line in lines if line.startswith('- ') and 'directories' not in line and 'image files' not in line]
            self.opencode_selected[name]=selected
            self.opencode_omitted[name]=[line for line in lines if line and line not in selected]

    def text(self,arm,session_id,has_checkpoint=True):
        if arm in ('C','C3'):
            return self.future.replace('{session_literal}',json.dumps(session_id,ensure_ascii=False)) if has_checkpoint else ''
        if arm=='codex':
            return ('## Archived conversation recall — history interface\n'+self.codex_functional+'\n'
                'This local reproduction contains this archived agent only; omit agent_name. Available read-only tools: '
                'history_list_windows, history_list_items, history_read_item, history_search_contents. '
                'Search is a case-sensitive literal substring. Read_item takes returned window_id/item_id and character offsets. '
                'No hosted notes.thread_hint or private model-only state is available in this user-authorized fixture; no hint content is fabricated.')
        if arm!='opencode': raise ValueError('unknown arm')
        parts=['## Available file lookup guidance',
               'This condition exposes observed file fragments, not a full conversation-history service. '
               'The virtual root is /; relative file names resolve under /workspace. Only known observations are available. '
               'Missing fragments are not proof of absence from the original conversation.']
        for name in ('glob','grep','read'):
            parts+=['### '+name,*self.opencode_selected[name]]
        parts+=['These local adapters are text-only. Task, Bash, media attachments and directory reads are not exposed.']
        return '\n'.join(parts)

    def adaptations(self):
        return {'future':{'routing_replacements':self.future_mappings,
                         'gate':'append only for frozen projections with checkpoints; CLI/shell routing mapped to the two available native-query adapters'},
                'codex':{'functional_prefix':self.codex_functional,
                         'omitted_source_tail':self.codex_description[len(self.codex_functional):],
                         'reason':'single-agent synchronous user-owned local history; no hosted private state, cross-agent access, or server thread_hint; do not simulate unavailable hints'},
                'opencode':{'selected_tool_lines':self.opencode_selected,'omitted_tool_lines':self.opencode_omitted,
                            'reason':'file-only/text-only replica; omit unavailable Task/Bash/media/directory operations and task auto-continuation unrelated to the new question'}}
