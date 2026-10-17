import json, sys

ev, target = sys.argv[1], sys.argv[2]
text = None
src = None
for line in open(ev):
    try:
        d = json.loads(line)
    except Exception:
        continue
    k = d.get("kind") or d.get("type") or ""
    if k == "todo_added":
        t = d.get("todo") or {}
        if t.get("id") == target and t.get("text"):
            text, src = t["text"], "todo_added"
    elif k == "todo_updated" and d.get("todo_id") == target and d.get("text"):
        # NOTE: todo_updated carries `text` FLAT; todo_added nests it under "todo".
        text, src = d["text"], "todo_updated"
if text is None:
    sys.exit("no text found")
print("chars:", len(text), " (from", src + ")")
print("HEAD:", text[:110].replace("\n", " | "))
print("TAIL:", text[-300:].replace("\n", " | "))
