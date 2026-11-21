"""复现「horizon-ctl screenshot 挂死」：窗口客户端连续操作 + 截图。

用法：python -I tools/shot_repro.py [轮数]
每个 ctl 调用都有超时：一超时就记为挂死（截图命令返回 -1 或状态查询不再回话）。
输出在 logs/shot-repro-<时间>/。
"""
import random
import sys
import time
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from e2e_mp import HMOD, Run, find_game  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / "target" / "accept"
Y = 150


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    rounds = int(sys.argv[1]) if len(sys.argv) > 1 else 40
    out = ROOT / "logs" / f"shot-repro-{datetime.now():%Y-%m-%d %H-%M-%S}"
    out.mkdir(parents=True, exist_ok=True)
    r = Run(out, "shot-repro.log")
    game, resources = find_game()
    ctl_exe = game / "horizon-ctl.exe"
    sock = WORK / "alice.sock"

    ctl = lambda *a, **kw: r.run([ctl_exe, "--socket", sock, *a], **kw)  # noqa: E731

    ctl(
        "launch",
        "--wait",
        "--server",
        "127.0.0.1:25598",
        "--name",
        "Alice",
        "--game-dir",
        WORK / "alice",
        "--home",
        WORK / "alice",
        "--resources",
        resources,
        "--dev-mod",
        HMOD,
        "--width",
        "1280",
        "--height",
        "720",
        "--render-distance",
        "4",
        timeout=300,
    )
    ctl("wait-ready", "120", timeout=150)

    hung = False
    for i in range(rounds):
        x, z = random.randint(-4, 4), random.randint(-4, 4)
        ctl("look-at-block", x, Y + 1, z, timeout=20)
        ctl("key key.keyboard.keypad.1 tap", timeout=20)
        ctl("mouse-move", random.randint(-60, 60), random.randint(-30, 30), timeout=20)
        t0 = time.time()
        code, text = ctl("screenshot", out / f"shot-{i:03d}.png", timeout=25)
        took = time.time() - t0
        r.note(f"第 {i} 轮截图：[{code}] {took:.1f}s {text[:120]}")
        if code == -1:
            hung = True
            r.note("截图超时，疑似挂死")
            break
        code, text = ctl("state", timeout=20)
        if code == -1:
            hung = True
            r.note("状态查询超时，客户端不再回话")
            break
    r.note("是否挂死:", hung, "输出目录:", out)
    if hung:
        code, text = ctl("logs", "--lines", "200", timeout=30)
        (out / "client-logs.txt").write_text(text, encoding="utf-8")
    ctl("quit", timeout=60)
    sys.exit(1 if hung else 0)


if __name__ == "__main__":
    main()
