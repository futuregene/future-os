"""Controlled interface-level replicas; never advertised as hosted/native services.

C/C3 delegates to the real Future CLI. Codex mirrors the dedicated history
contract with explicitly documented local defaults. OpenCode is a file-tool
replica over known observed file fragments, not a fabricated transcript file.
"""
import fnmatch
import json
from pathlib import Path, PurePosixPath
import re
import subprocess

MAX_REPLY=65536


def tool(name,description,properties,required=()):
    return {'type':'function','function':{'name':name,'description':description,
        'parameters':{'type':'object','properties':properties,'required':list(required)}}}

S={'type':'string'}
N={'type':'integer','minimum':1}
NULL_STRING={'anyOf':[{'type':'string'},{'type':'null'}]}
FILTERS={'agent_name':NULL_STRING,'window_id':NULL_STRING,
    'role':{'anyOf':[{'type':'string','enum':['user','assistant','tool','system','developer']},{'type':'null'}]},
    'tool_name':NULL_STRING,'tool_namespace':NULL_STRING,
    'limit':N,'recent_first':{'type':'boolean'}}
FUTURE_TOOLS=[tool('history_search','Search the current archived session using the native Future CLI. Literal substring, ASCII case insensitive.',
                  {'query':S,'limit':{'type':'integer','minimum':1,'maximum':20}},['query']),
              tool('history_get','Read an original entry using the native Future CLI. Offset and limit are UTF-8 bytes; use returned nextOffset.',
                   {'entry_id':S,'offset':{'type':'integer','minimum':0},'limit':{'type':'integer','minimum':4,'maximum':32768}},['entry_id'])]
CODEX_TOOLS=[tool('history_list_windows','List archived context windows and item counts.',
                 {'limit':N,'recent_first':{'type':'boolean'},'agent_name':NULL_STRING}),
             tool('history_list_items','List items with optional window, role and tool filters. Use item_id and window_id for reads.',
                  dict(FILTERS,max_chars_per_item=N)),
             tool('history_read_item','Read an item in its specified window. Character offsets are zero-based.',
                  {'window_id':S,'item_id':S,'offset_chars':{'type':'integer','minimum':0},'limit_chars':N,'agent_name':NULL_STRING},['window_id','item_id']),
             tool('history_search_contents','Search item content by case-sensitive literal substring; optional window/role/tool filters.',
                  dict(FILTERS,query=S),['query'])]
FILE_TOOLS=[tool('glob','Find observed file fragments by recursive glob. Virtual root is /; no host filesystem access.',{'pattern':S,'path':S},['pattern']),
            tool('grep','Search observed file fragments with ripgrep regular expressions. At most 100 matches.',{'pattern':S,'path':S,'include':S},['pattern']),
            tool('read','Read an observed file fragment, not a guaranteed complete historical file. Offset is a 1-based line in this fragment.',
                 {'filePath':S,'offset':N,'limit':N},['filePath'])]


def integer(args,key,default,minimum=1,maximum=1000000):
    value=args.get(key,default)
    if value is None: value=default
    if not isinstance(value,int) or isinstance(value,bool) or not minimum<=value<=maximum:
        raise ValueError(f'{key} outside local interface range')
    return value


def body(record):
    if record['kind']=='tool_call':
        return json.dumps({'tool':record.get('tool','read'),'path':record.get('path','')},ensure_ascii=False)
    return record.get('text','')


def encode(value): return json.dumps(value,ensure_ascii=False)


def bounded_items(items,key,**extra):
    result={key:[],**extra}
    for item in items:
        candidate={**result,key:result[key]+[item]}
        if len(encode(candidate).encode())>MAX_REPLY:
            result['budget_truncated']=True; break
        result=candidate
    return encode(result)


class CodexHistory:
    def __init__(self,records,ends):
        self.windows=[]; self.flat=[]; start=0
        calls={r['call']:r for r in records if r['kind']=='tool_call'}
        for number,end in enumerate(ends):
            items=[]
            for index,r in enumerate(records[start:end],start):
                caller=r if r['kind']=='tool_call' else calls.get(r.get('call'),{})
                name=caller.get('tool','read') if r['kind'] in ('tool_call','tool_result') else None
                namespace,name=(name.rsplit('.',1) if name and '.' in name else (None,name))
                item={'item_id':f'i{index:06d}','window_id':f'w{number:03d}','role':r['role'],
                      'tool_name':name,'tool_namespace':namespace,'content':body(r)}
                items.append(item); self.flat.append(item)
            self.windows.append({'window_id':f'w{number:03d}','items':items}); start=end
        assert start==len(records)

    def call(self,name,args):
        if args.get('agent_name') not in (None,'','current'):
            return encode({'error':'unknown agent in this single-agent fixture'})
        if name=='history_list_windows':
            items=list(reversed(self.windows)) if args.get('recent_first') else self.windows
            limit=integer(args,'limit',20,maximum=100)
            return bounded_items([{'window_id':w['window_id'],'item_count':len(w['items'])} for w in items[:limit]],'windows')
        if name=='history_read_item':
            item=next((x for x in self.flat if x['window_id']==args.get('window_id') and x['item_id']==args.get('item_id')),None)
            if item is None: return encode({'error':'item not found in this window'})
            offset=integer(args,'offset_chars',0,minimum=0)
            length=integer(args,'limit_chars',4000,maximum=32768)
            text=item['content']; chunk=text[offset:offset+length]
            while len(encode({'content':chunk}).encode())>MAX_REPLY-1024: chunk=chunk[:len(chunk)//2]
            return encode({'content':chunk,'n_chars':len(chunk),'next_offset_chars':offset+len(chunk),
                           'total_chars':len(text),'has_more':offset+len(chunk)<len(text)})
        if name not in ('history_list_items','history_search_contents'): raise ValueError('unknown history tool')
        items=self.flat
        for field in ('window_id','role','tool_name','tool_namespace'):
            if args.get(field) is not None: items=[x for x in items if x[field]==args[field]]
        if name=='history_search_contents':
            query=args.get('query')
            if not isinstance(query,str) or not query: raise ValueError('nonempty query required')
            items=[x for x in items if query in x['content']]
        if args.get('recent_first'): items=list(reversed(items))
        limit=integer(args,'limit',20,maximum=100)
        width=integer(args,'max_chars_per_item',4000,maximum=16384) if name=='history_list_items' else 4000
        # Head excerpts follow the original study's explicit assumption, not a
        # claimed observation of the private hosted backend. Read returns full ranges.
        output=[{k:v for k,v in item.items() if k!='content'}|{'truncated_content':item['content'][:width],
                   'content_chars':len(item['content'])} for item in items[:limit]]
        return bounded_items(output,'items',matched=len(items),has_more=len(items)>limit)


def virtual_path(raw):
    raw=str(raw).replace('\\','/')
    if re.match(r'^[A-Za-z]:/',raw): raw='/drive-'+raw[0].upper()+raw[2:]
    if not raw.startswith('/'): raw='/workspace/'+raw
    if '..' in PurePosixPath(raw).parts: raise ValueError('parent traversal is not a virtual file path')
    return str(PurePosixPath(raw))


class ObservedFiles:
    def __init__(self,records,root):
        self.root=Path(root); self.root.mkdir(parents=True,exist_ok=True)
        self.files={}; self.invalidated=0; self.unmapped_results=0
        calls={}
        for r in records:
            if r['kind']=='tool_call':
                calls[r['call']]=r
                if r.get('tool') in ('write','edit') and r.get('path'):
                    self.files.pop(virtual_path(r['path']),None); self.invalidated+=1
            elif r['kind']=='tool_result':
                call=calls.get(r.get('call'),{})
                path=r.get('path') or call.get('path')
                if path and call.get('tool','read') in ('read','functions.read'):
                    self.files[virtual_path(path)]=r.get('text','')
                else: self.unmapped_results+=1
        # Only known read observations. Never invent tool-output.log or collapse
        # distinct directories to basenames. Read observations can be fragments;
        # original offsets and unobserved subsequent writes are unavailable.
        for logical,text in self.files.items():
            physical=self.root/logical.lstrip('/'); physical.parent.mkdir(parents=True,exist_ok=True); physical.write_text(text)

    def paths(self,path='/'):
        if path in (None,'/','.'): return list(self.files)
        prefix=virtual_path(path)
        return [p for p in self.files if p==prefix or p.startswith(prefix.rstrip('/')+'/')]

    def call(self,name,args):
        if name=='read':
            key=virtual_path(args.get('filePath',''))
            if key not in self.files: return f'File observation not available: {key}'
            lines=self.files[key].splitlines(); offset=integer(args,'offset',1); limit=integer(args,'limit',2000,maximum=2000)
            out=[]; used=0
            for i,line in enumerate(lines[offset-1:offset-1+limit],offset):
                line=line if len(line)<=2000 else line[:2000]+'... (line truncated)'
                value=f'{i}: {line}'
                if used+len(value.encode())>50*1024: break
                out.append(value); used+=len(value.encode())+1
            return f'<path>{key}</path>\n<type>observed-fragment</type>\n<content>\n'+'\n'.join(out)+f'\n</content>\nfragmentLines={len(lines)}, nextLine={offset+len(out)}'
        selected=self.paths(args.get('path','/'))
        if name=='glob':
            pattern=args.get('pattern','*')
            # Use the real rg glob implementation, not Python fnmatch recursion.
            result=subprocess.run(['rg','--files','--hidden','--no-ignore','--glob',pattern,'.'],cwd=self.root,text=True,capture_output=True)
            if result.returncode not in (0,1): return result.stderr
            matches=['/'+p.removeprefix('./') for p in result.stdout.splitlines()]
            matches=[p for p in matches if p in selected][:100]
            return '\n'.join(matches) or 'No files found'
        if name!='grep': raise ValueError('unknown file tool')
        pattern=args.get('pattern')
        if not isinstance(pattern,str) or not pattern: raise ValueError('pattern required')
        if args.get('include'): selected=[p for p in selected if fnmatch.fnmatch(PurePosixPath(p).name,args['include'])]
        matches=[]; used=0
        for logical in selected:
            result=subprocess.run(['rg','--json','--max-count','100','--',pattern,str(self.root/logical.lstrip('/'))],text=True,capture_output=True)
            if result.returncode not in (0,1): return result.stderr
            for line in result.stdout.splitlines():
                event=json.loads(line)
                if event['type']!='match': continue
                data=event['data']; content=data['lines'].get('text','').rstrip('\n')
                rendered=f'{logical}:{data["line_number"]}: {content}'
                if len(rendered.encode())>MAX_REPLY-1024:
                    rendered=rendered.encode()[:MAX_REPLY-1024].decode('utf-8','ignore')+' [excerpt truncated]'
                if used+len(rendered.encode())>MAX_REPLY-100: return '\n'.join(matches)+'\n[output budget reached]'
                matches.append(rendered); used+=len(rendered.encode())+1
                if len(matches)>=100: return '\n'.join(matches)+'\n[100-match limit reached]'
        return '\n'.join(matches) or 'No files found'
