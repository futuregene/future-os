"""Study resource guard; never implements search or alters native outputs.

Same read-only command subset for both native shells. Unsupported actions are
labelled STUDY_SCOPE_DENIED, not disguised as a native tool failure.
"""
import shlex
import re
from pathlib import Path


def allowed_path(value,workspace,home,cwd=None,roots=None):
    value=str(value)
    if value.startswith('~/'): value=str(home)+value[1:]
    p=Path(value)
    if not p.is_absolute(): p=Path(cwd or workspace)/p
    p=p.resolve()
    if not any(p.is_relative_to(Path(root).resolve()) for root in (roots or (workspace,home))):
        raise ValueError('STUDY_SCOPE_DENIED: path outside this case')
    return p


def check_shell(command,workspace,home,sid=None,workdir=None,roots=None):
    if not isinstance(command,str): raise ValueError('STUDY_SCOPE_DENIED: command must be text')
    cwd=allowed_path(workdir or workspace,workspace,home,roots=roots)
    if any(x in command for x in ('$', '`','\n','\r','\x00')):
        raise ValueError('STUDY_SCOPE_DENIED: shell expansion/control unsupported')
    lexer=shlex.shlex(command,posix=True,punctuation_chars='|&;<>'); lexer.whitespace_split=True; lexer.commenters=''
    tokens=list(lexer)
    if '&&' in tokens:
        at=tokens.index('&&')
        check_shell(shlex.join(tokens[:at]),workspace,home,sid,workdir,roots)
        check_shell(shlex.join(tokens[at+1:]),workspace,home,sid,workdir,roots)
        return
    if '>' in tokens:
        if len(tokens)!=5 or tokens[:4]!=['opencode','export',sid,'>']:
            raise ValueError('STUDY_SCOPE_DENIED: only a native archive export may create scratch output')
        target=allowed_path(tokens[4],workspace,home,cwd,roots=roots)
        if not target.is_relative_to((Path(workspace)/'retrieved').resolve()):
            raise ValueError('STUDY_SCOPE_DENIED: export destination must be in retrieved/')
        return
    groups=[[]]
    for token in tokens:
        if token=='|': groups.append([])
        elif token and all(c in '|&;<>' for c in token): raise ValueError('STUDY_SCOPE_DENIED: only read-only pipes allowed')
        else: groups[-1].append(token)
    for group in groups:
        if not group: raise ValueError('empty pipeline stage')
        name,*args=group
        if name=='opencode':
            if args!=['export',sid] and args!=['--help']: raise ValueError('STUDY_SCOPE_DENIED: only this archive export')
        elif name=='jq':
            while args and args[0] in ('-r','-c','-M','-s','-e'): args=args[1:]
            if not args or args[0].startswith('-') or re.search(r'(^|;)\s*(include|import)\b',args[0]):
                raise ValueError('STUDY_SCOPE_DENIED: unsupported jq program/options')
            for value in args[1:]: allowed_path(value,workspace,home,cwd,roots=roots)
        elif name in ('rg','grep'):
            permitted={'-F','-n','-o','-i','-l','-H','--fixed-strings','--line-number','--only-matching','--ignore-case','--no-heading','--color=never'}
            while args and args[0].startswith('-'):
                option=args.pop(0)
                if option=='--': break
                if option not in permitted: raise ValueError('STUDY_SCOPE_DENIED: unsupported search flag')
            if not args: raise ValueError('search pattern required')
            for value in args[1:]: allowed_path(value,workspace,home,cwd,roots=roots)
        elif name in ('cat','ls','head','tail','wc'):
            i=0
            while i<len(args):
                value=args[i]
                if name in ('head','tail') and value=='-n':
                    i+=1
                    if i>=len(args) or not args[i].isdigit(): raise ValueError('line count required')
                elif value.startswith('-'):
                    if value not in ('-a','-l','-la','-al','-c','-m','--'): raise ValueError('unsupported reader flag')
                else: allowed_path(value,workspace,home,cwd,roots=roots)
                i+=1
        else: raise ValueError('STUDY_SCOPE_DENIED: command is not an approved reader')


def check_tool(name,args,workspace,home,sid,roots=None):
    for key,maximum in (('timeout',120000),('max_output_tokens',10000)):
        value=args.get(key)
        if value is not None and (not isinstance(value,(int,float)) or isinstance(value,bool) or value>maximum):
            raise ValueError('STUDY_SCOPE_DENIED: invalid or excessive '+key)
    if name in ('exec_command','bash'):
        if (args.get('sandbox_permissions') or 'use_default')!='use_default' or args.get('additional_permissions'):
            raise ValueError('STUDY_SCOPE_DENIED: no permission escalation')
        if args.get('tty'): raise ValueError('STUDY_SCOPE_DENIED: no interactive terminal')
        check_shell(args.get('cmd',args.get('command','')),workspace,home,sid,args.get('workdir'),roots=roots)
    elif name=='write_stdin':
        if args.get('chars','') not in ('','\x03'): raise ValueError('STUDY_SCOPE_DENIED: polling/cancel only')
    elif name in ('read','glob','grep'):
        allowed_path(args.get('filePath',args.get('path',workspace)),workspace,home,roots=roots)
    else: raise ValueError('STUDY_SCOPE_DENIED: tool not available in retrieval-only condition')
