"""Shared transport helpers: SSE parsing, a text size estimate and atomic JSON writes.

Extracted from the retired A/B/C/M harness, which the two surviving experiments used for
exactly these three functions. The rest of that file drove arms that no longer exist.
"""
import hashlib
import json


def digest(value):
    return hashlib.sha256(value.encode() if isinstance(value, str) else value).hexdigest()


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    temporary.replace(path)


def tokens(text):
    """Cheap size estimate in quarters of a token: ASCII 1, CJK 6, other non-ASCII 2."""
    quarters = 0
    for c in text:
        n = ord(c)
        quarters += (1 if n < 128 else
                     6 if (0x3400 <= n <= 0x9fff or 0x3040 <= n <= 0x30ff or
                           0xac00 <= n <= 0xd7af or 0xf900 <= n <= 0xfaff or
                           0x20000 <= n <= 0x2a6df) else 2)
    return (quarters + 3) // 4


def parse_sse(text):
    """The chat-completions stream, as this harness needs it: text, tool calls, usage, finish."""
    answer, calls, usage, finish, error = '', {}, None, None, None
    for line in text.splitlines():
        if not line.startswith('data:'):
            continue
        data = line[5:].strip()
        if data == '[DONE]':
            continue
        try:
            obj = json.loads(data)
        except json.JSONDecodeError:
            continue
        if isinstance(obj.get('usage'), dict):
            usage = obj['usage']
        if obj.get('error'):
            error = str(obj['error'])
        for choice in obj.get('choices', []):
            delta = choice.get('delta') or {}
            answer += delta.get('content') or ''
            if choice.get('finish_reason'):
                finish = choice['finish_reason']
            for call in delta.get('tool_calls', []):
                slot = calls.setdefault(call.get('index', 0), {
                    'id': '', 'type': 'function', 'function': {'name': '', 'arguments': ''}})
                if call.get('id'):
                    slot['id'] = call['id']
                function = call.get('function') or {}
                if function.get('name'):
                    slot['function']['name'] = function['name']
                if function.get('arguments') is not None:
                    slot['function']['arguments'] += (
                        function['arguments'] if isinstance(function['arguments'], str)
                        else json.dumps(function['arguments']))
    return {'text': answer, 'calls': list(calls.values()), 'usage': usage,
            'finish': finish, 'error': error}
