"""在 SDK 的 surface.json 里按名称查 API：handles / traits / types 的签名与是否支持 wasm。

用法：python -I tools/api.py <正则> [--doc]
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
surface = json.loads((ROOT / ".horizon/sdk/surface.json").read_text(encoding="utf-8"))
pattern = re.compile(sys.argv[1])
show_doc = "--doc" in sys.argv


def walk(section, entries):
    if isinstance(entries, dict):
        entries = [dict(v, id=v.get("id", k)) if isinstance(v, dict) else {"id": k} for k, v in entries.items()]
    for e in entries:
        if not isinstance(e, dict):
            continue
        ident = e.get("id", "")
        if pattern.search(ident):
            wasm = "" if e.get("wasm", True) else "  [Not on wasm]"
            print(f"{ident}: {e.get('signature', '')}{wasm}")
            if show_doc and e.get("doc"):
                print("    " + e["doc"].replace("\n", "\n    "))


for name in ("handles", "traits", "functions", "types"):
    walk(name, surface.get(name, []))
