"""极简 MCP stdio 客户端：驱动 `horizon-launcher mcp`，供验收脚本使用。

用法：
  python -I tools/hzmcp.py list [正则]          列出工具及参数
  python -I tools/hzmcp.py call 工具 'JSON参数'   调用一个工具
"""
import json
import os
import re
import subprocess
import sys
import threading
import queue

LAUNCHER = os.environ.get(
    "HORIZON_LAUNCHER",
    os.path.expandvars(r"%LOCALAPPDATA%\Programs\Horizon Launcher\horizon-launcher.exe"),
)


class Mcp:
    def __init__(self):
        self.proc = subprocess.Popen(
            [LAUNCHER, "mcp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
        )
        self.lines = queue.Queue()
        threading.Thread(target=self._pump, daemon=True).start()
        self.next_id = 0
        self.request("initialize", {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "jade-verify", "version": "0.1"},
        })
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def _pump(self):
        for line in self.proc.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def _send(self, msg):
        self.proc.stdin.write(json.dumps(msg, ensure_ascii=False) + "\n")
        self.proc.stdin.flush()

    def request(self, method, params, timeout=600):
        self.next_id += 1
        rid = self.next_id
        self._send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        while True:
            line = self.lines.get(timeout=timeout)
            if line is None:
                raise RuntimeError("launcher mcp 已退出")
            msg = json.loads(line)
            if msg.get("id") == rid:
                if "error" in msg:
                    raise RuntimeError(f"{method}: {msg['error']}")
                return msg["result"]

    def tools(self):
        return self.request("tools/list", {})["tools"]

    def call(self, name, args=None, timeout=600):
        """调用工具，返回 (是否出错, 文本内容, 结构化内容)。"""
        res = self.call_raw(name, args, timeout)
        text = "\n".join(c.get("text", "") for c in res.get("content", []) if c.get("type") == "text")
        return res.get("isError", False), text, res.get("structuredContent")

    def call_raw(self, name, args=None, timeout=600):
        return self.request("tools/call", {"name": name, "arguments": args or {}}, timeout)

    def screenshot(self, path):
        """截一帧存为 PNG，成功返回 True。"""
        import base64
        res = self.call_raw("game_screenshot")
        for c in res.get("content", []):
            if c.get("type") == "image":
                with open(path, "wb") as f:
                    f.write(base64.b64decode(c["data"]))
                return True
        return False

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=10)
        except Exception:
            self.proc.kill()


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    mcp = Mcp()
    try:
        if sys.argv[1] == "list":
            pat = re.compile(sys.argv[2] if len(sys.argv) > 2 else ".")
            for t in mcp.tools():
                if pat.search(t["name"]):
                    props = t.get("inputSchema", {}).get("properties", {})
                    print(f"{t['name']}({', '.join(props)}): {t.get('description', '')[:120]}")
        elif sys.argv[1] == "call":
            args = json.loads(sys.argv[3]) if len(sys.argv) > 3 else {}
            err, text, _ = mcp.call(sys.argv[2], args)
            print(("ERROR " if err else "") + text)
    finally:
        mcp.close()


if __name__ == "__main__":
    main()
