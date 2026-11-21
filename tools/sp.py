"""单人游戏实测：起一个装了 jade 的单人世界（内置服务端），看内置服务端是否加载 side="client" 的包。

要回答的问题（README「单人游戏」那条）：单人世界里客户端的 jade 能不能拿到服务端数据。
判据是箱子的提示框：如果出现容器内容（钻石×12、苹果×5），说明内置服务端加载了这份包；
如果只有块名和图标，说明内置服务端没加载（只有客户端那一半）。

用法：python -I tools/sp.py            跑一遍（离屏客户端，避开截图卡死的窗口模式）
      python -I tools/sp.py --keep    结束后保留客户端
"""
import json
import shutil
import sys
import time
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from PIL import Image  # noqa: E402

from e2e_mp import HMOD, Run, find_game  # noqa: E402
from hud import crop_tooltip  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SP = ROOT / "target" / "sp"
GAME_DIR = SP / "game"
SOCKET = SP / "sp.sock"
WORLD = "SPTest"
Y = 150

CHEST = 'minecraft:chest[facing=east]{Items:[{Slot:0b,id:"minecraft:diamond",count:12},{Slot:1b,id:"minecraft:apple",count:5}]}'
FURNACE = (
    'minecraft:furnace[facing=west,lit=true]{Items:[{Slot:0b,id:"minecraft:iron_ore",count:5},'
    '{Slot:1b,id:"minecraft:coal",count:3},{Slot:2b,id:"minecraft:iron_ingot",count:2}],'
    'cooking_time_spent:80s,cooking_total_time:200s,lit_time_remaining:1000s,lit_total_time:1600s}'
)


def write_options():
    """关掉新手提示和聊天框，跟验收脚本一致；语言用中文。"""
    GAME_DIR.mkdir(parents=True, exist_ok=True)
    path = GAME_DIR / "options.txt"
    lines = path.read_text(encoding="utf-8").splitlines() if path.exists() else []
    lines = [l for l in lines if not l.startswith(("tutorialStep:", "chatVisibility:", "lang:"))]
    lines += ["tutorialStep:none", "chatVisibility:2", "lang:zh_cn"]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    keep = "--keep" in sys.argv
    if not HMOD.exists():
        sys.exit("缺少 target/horizon-mod/jade.hmod，先运行 cargo horizon-mod build --wasm")
    out = ROOT / "logs" / f"sp-{datetime.now():%Y-%m-%d %H-%M-%S}"
    out.mkdir(parents=True, exist_ok=True)
    r = Run(out, "sp.log")
    game, resources = find_game()
    ctl_exe = game / "horizon-ctl.exe"
    r.note("游戏目录:", game)

    ctl = lambda *a, **kw: r.run([ctl_exe, "--socket", SOCKET, *a], **kw)  # noqa: E731

    SP.mkdir(parents=True, exist_ok=True)
    write_options()

    try:
        ctl(
            "launch",
            "--offscreen",
            "--wait",
            "--world",
            WORLD,
            "--creative",
            "--game-dir",
            GAME_DIR,
            "--home",
            GAME_DIR,
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
            check=True,
        )
        ctl("wait-ready", "120", timeout=150)
        _, state = ctl("state", timeout=30)
        (out / "state.json").write_text(state, encoding="utf-8")
        r.note("state:", state[:400])

        # 造一个像样的场地。命令能不能用取决于单人世界有没有开作弊。
        for command in [
            f"fill -6 {Y} -6 6 {Y} 6 stone",
            f"fill -6 {Y + 1} -6 6 {Y + 4} 6 air",
            "gamerule advance_time false",
            "time set 6000",
            "weather clear",
            f"tp @s 0.5 {Y + 1} 0.5",
            f"setblock -3 {Y + 1} 0 {CHEST}",
            f"setblock 0 {Y + 1} 3 minecraft:stone",
            f"setblock 3 {Y + 1} 0 {FURNACE}",
        ]:
            _, text = ctl("command", command, timeout=30)
            r.note(f"cmd {command} -> {text[:160]}")
        time.sleep(1.0)

        _, msgs = ctl("messages", timeout=30)
        (out / "messages.txt").write_text(msgs, encoding="utf-8")
        r.note("messages:", msgs[:600])

        # 看箱子：内容是否出现，直接决定内置服务端有没有加载这份包。
        ctl("look-at-block", -3, Y + 1, 0, timeout=30)
        time.sleep(2.5)
        _, looking = ctl("looking-at", timeout=30)
        (out / "looking-at.txt").write_text(looking, encoding="utf-8")
        r.note("looking-at:", looking[:400])
        shot = out / "chest.png"
        ctl("screenshot", shot, timeout=60)
        tip = crop_tooltip(Image.open(shot)) if shot.exists() else None

        # 熔炉进度同样需要服务端数据。
        ctl("look-at-block", 3, Y + 1, 0, timeout=30)
        time.sleep(2.5)
        shot3 = out / "furnace.png"
        ctl("screenshot", shot3, timeout=60)
        tip3 = crop_tooltip(Image.open(shot3)) if shot3.exists() else None
        if tip3 is not None:
            tip3.save(out / "furnace-tip.png")

        # 再看一个普通方块，确认提示框整体工作正常（不然“没内容”可能是压根没提示框）。
        ctl("look-at-block", 0, Y + 1, 3, timeout=30)
        time.sleep(2.0)
        shot2 = out / "stone.png"
        ctl("screenshot", shot2, timeout=60)
        tip2 = crop_tooltip(Image.open(shot2)) if shot2.exists() else None
        if tip is not None:
            tip.save(out / "chest-tip.png")
        if tip2 is not None:
            tip2.save(out / "stone-tip.png")
        r.note("箱子有提示框:", tip is not None, "熔炉有提示框:", tip3 is not None, "石头有提示框:", tip2 is not None)

        _, logs = ctl("logs", "--lines", "3000", timeout=60)
        (out / "client.log").write_text(logs, encoding="utf-8")
        keys = ("jade", "mod", "wasm", "server", "cheat", "permission")
        hits = [l for l in logs.splitlines() if any(k in l.lower() for k in keys)]
        (out / "client-modlines.txt").write_text("\n".join(hits), encoding="utf-8")
        r.note("日志里和 mod/服务端相关的行:", len(hits))
        for line in hits[:80]:
            r.note("  ", line[:200])
        r.note("输出目录:", out)
    finally:
        if keep:
            r.note("保留客户端（--keep）")
        else:
            ctl("quit", timeout=60)


if __name__ == "__main__":
    main()
