"""Read-only study envelope, not a search implementation.

Admit normal native reader flags and pure transformations; reject execution
hooks, foreign files, arbitrary scripts, and writes except native scratch export.
"""
import re
import shlex
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
    def path(value):
        if value!='-': allowed_path(value,workspace,home,cwd,roots=roots)
    def sequence(tokens):
        groups=[[]]
        for token in tokens:
            if token in ('|','&&',';'): groups.append([])
            elif token and all(c in '|&;<>' for c in token) and token!='>':
                raise ValueError('STUDY_SCOPE_DENIED: unsupported control operator')
            else: groups[-1].append(token)
        for group in groups:
            if not group: raise ValueError('empty command stage')
            if '>' in group:
                if len(group)!=5 or group[:4]!=['opencode','export',sid,'>']:
                    raise ValueError('STUDY_SCOPE_DENIED: only native archive export may create scratch output')
                target=allowed_path(group[4],workspace,home,cwd,roots=roots)
                if not target.is_relative_to((Path(workspace)/'retrieved').resolve()): raise ValueError('export outside retrieved/')
                continue
            name,*args=group
            if name=='opencode':
                if args!=['export',sid] and args!=['--help']: raise ValueError('STUDY_SCOPE_DENIED: only this archive export')
            elif name=='jq':
                while args and args[0] in ('-r','-c','-M','-s','-e','--raw-output','--compact-output','--monochrome-output','--slurp','--exit-status'):
                    args=args[1:]
                if not args or args[0].startswith('-') or re.search(r'(^|;)\s*(include|import)\b',args[0]):
                    raise ValueError('STUDY_SCOPE_DENIED: unsupported jq program/options')
                for value in args[1:]: path(value)
            elif name in ('rg','grep'):
                booleans={'--fixed-strings','--line-number','--only-matching','--ignore-case','--no-heading','--color=never',
                          '--count','--count-matches','--files-with-matches','--no-messages','--text','--multiline','--files','--hidden'}
                value_flags={'-m','--max-count','-A','-B','-C','--after-context','--before-context','--context','-g','--glob','--iglob','--include','--exclude'}
                explicit_pattern=False; positionals=[]; i=0
                while i<len(args):
                    arg=args[i]
                    if arg=='--': positionals+=args[i+1:]; break
                    if arg in ('-e','--regexp'):
                        i+=1
                        if i>=len(args): raise ValueError('pattern required')
                        explicit_pattern=True
                    elif arg.startswith('--regexp='): explicit_pattern=True
                    elif arg in value_flags:
                        i+=1
                        if i>=len(args): raise ValueError('flag value required')
                    elif arg in booleans or (arg.startswith('-') and not arg.startswith('--') and len(arg)>1 and all(c in 'FnoilcvHhqsar' for c in arg[1:])):
                        pass
                    elif re.fullmatch(r'-[mABC]\d+',arg): pass
                    elif arg.startswith('-'): raise ValueError('STUDY_SCOPE_DENIED: unsupported search flag '+arg)
                    else: positionals.append(arg)
                    i+=1
                if '--files' in args: explicit_pattern=True
                if not explicit_pattern:
                    if not positionals: raise ValueError('search pattern required')
                    positionals=positionals[1:]
                for value in positionals: path(value)
            elif name in ('cat','ls','head','tail','wc','sort','uniq'):
                files=[]; i=0
                simple={'-a','-l','-la','-al','-c','-m','-u','-n','-r','-f','-b','-h','-d','-i','-q','-v','--'}
                while i<len(args):
                    value=args[i]
                    if name in ('head','tail') and value in ('-n','-c','--lines','--bytes'):
                        i+=1
                        if i>=len(args) or not re.fullmatch(r'[+-]?\d+',args[i]): raise ValueError('reader count required')
                    elif name in ('head','tail') and re.fullmatch(r'-(?:[nc])?[+-]?\d+',value): pass
                    elif name=='sort' and value in ('-k','-t'):
                        i+=1
                        if i>=len(args): raise ValueError('sort argument required')
                    elif value.startswith('-'):
                        if value not in simple: raise ValueError('STUDY_SCOPE_DENIED: unsupported reader flag '+value)
                    else: files.append(value)
                    i+=1
                if name=='uniq' and len(files)>1: raise ValueError('STUDY_SCOPE_DENIED: uniq output file forbidden')
                for value in files: path(value)
            elif name=='sed':
                if not args or args[0]!='-n' or len(args)<2 or not re.fullmatch(r'\d+(?:,\d+)?p',args[1]):
                    raise ValueError('STUDY_SCOPE_DENIED: sed supports numeric line printing only')
                for value in args[2:]: path(value)
            elif name=='echo':
                pass  # literal separators; expansions/redirections already rejected
            elif name=='printf':
                if len(args)!=1 or args[0].startswith('-') or '%' in args[0]:
                    raise ValueError('STUDY_SCOPE_DENIED: printf literal separators only')
            else: raise ValueError('STUDY_SCOPE_DENIED: command is not an approved reader')
    sequence(tokens)


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
