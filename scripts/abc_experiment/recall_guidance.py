"""Source-derived functional recall guidance for the existing local interfaces.

For Future arms this returns **production's own text, unmodified**, fetched from
`history_recall::system_prompt` by `production_shape`. It previously rewrote it: "use the
existing shell tool" became "use these read-only adapters of the native Future history CLI",
and the two `future session history ...` commands became `history_search(query=...)` /
`history_get(entry_id=...)` — tool names no product has. A replay that measures those
measures an adaptation, so the substitutions are gone; the `future_mappings` attribute is
retained, empty, so older callers that recorded it keep working.

Codex and OpenCode keep their own products' guidance, which is production for those arms;
those are still selected/assembled from the pinned sources below.
"""
import json
from pathlib import Path
import re


class Guidance:
    def __init__(self,future_source,codex_source,opencode_source,shape=None,shape_factory=None):
        # A fixed shape, or a factory when each case has its own session id (the guidance
        # embeds the session id literally, so it cannot be shared across cases).
        self.shape=shape
        self.shape_factory=shape_factory
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
        # Reads are lazy: a run over the Future arms alone must not require the Codex and
        # OpenCode checkouts to exist, and vice versa.
        self._source=None
        self._derived=False
        # Production's guidance is fetched, not extracted-and-edited. `shape` is a
        # `production_shape.RequestShape`; without it the Future arms cannot be served and say
        # so rather than falling back to a hand-written paraphrase.
        self.future_mappings={}

    @property
    def source(self):
        if self._source is None:
            self._source={key:path.read_text() for key,path in self.paths.items()}
        return self._source

    def _derive(self):
        """Read and select the Codex/OpenCode guidance, on first use of those arms."""
        if self._derived:
            return
        source=self.source
        match=re.search(r'const HISTORY_DESCRIPTION: &str = "((?:\\.|[^"\\])*)";',source['codex'])
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
            lines=source['opencode_'+name].splitlines()
            if name=='glob':
                selected=[line for line in lines if line.startswith('- ') and 'Task tool' not in line]
            elif name=='grep':
                selected=[line for line in lines if line.startswith('- ') and 'Task tool' not in line and 'Bash tool' not in line]
            else:
                selected=[line for line in lines if line.startswith('- ') and 'directories' not in line and 'image files' not in line]
            self.opencode_selected[name]=selected
            self.opencode_omitted[name]=[line for line in lines if line and line not in selected]
        self._derived=True

    def text(self,arm,session_id,has_checkpoint=True):
        if arm in ('C','C3'):
            if not has_checkpoint:
                return ''
            shape=self.shape_factory(session_id) if self.shape_factory else self.shape
            if shape is None:
                raise ValueError('Future arms need a production_shape.RequestShape; a hand-written paraphrase is not production')
            # Only the appended guidance, not the whole prompt: callers already hold the base
            # prompt in place and append this to it. Production's leading blank line is kept
            # so the concatenation is byte-exact.
            return shape.guidance(has_checkpoint=True)
        if arm=='codex':
            self._derive()
            return ('## Archived conversation recall — history interface\n'+self.codex_functional+'\n'
                'This local reproduction contains this archived agent only; omit agent_name. Available read-only tools: '
                'history_list_windows, history_list_items, history_read_item, history_search_contents. '
                'Search is a case-sensitive literal substring. Read_item takes returned window_id/item_id and character offsets. '
                'No hosted notes.thread_hint or private model-only state is available in this user-authorized fixture; no hint content is fabricated.')
        if arm!='opencode': raise ValueError('unknown arm')
        self._derive()
        parts=['## Available file lookup guidance',
               'This condition exposes observed file fragments, not a full conversation-history service. '
               'The virtual root is /; relative file names resolve under /workspace. Only known observations are available. '
               'Missing fragments are not proof of absence from the original conversation.']
        for name in ('glob','grep','read'):
            parts+=['### '+name,*self.opencode_selected[name]]
        parts+=['These local adapters are text-only. Task, Bash, media attachments and directory reads are not exposed.']
        return '\n'.join(parts)

    def adaptations(self):
        self._derive()
        return {'future':{'routing_replacements':self.future_mappings,
                         'source':'verbatim from history_recall::system_prompt via production_shape',
                         'gate':'append only for frozen projections with checkpoints; the CLI and shell tool are the real ones, so no routing substitution remains'},
                'codex':{'functional_prefix':self.codex_functional,
                         'omitted_source_tail':self.codex_description[len(self.codex_functional):],
                         'reason':'single-agent synchronous user-owned local history; no hosted private state, cross-agent access, or server thread_hint; do not simulate unavailable hints'},
                'opencode':{'selected_tool_lines':self.opencode_selected,'omitted_tool_lines':self.opencode_omitted,
                            'reason':'file-only/text-only replica; omit unavailable Task/Bash/media/directory operations and task auto-continuation unrelated to the new question'}}
