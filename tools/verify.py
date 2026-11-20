"""游戏内验收：部署 jade.hmod，离屏启动测试世界，搭场景，逐个对准并截图。

用法：python -I tools/verify.py [--no-deploy] [--keep-open]
截图与日志写到 logs/verify-<时间>/，之后人工或 agent 看图确认 HUD。
"""
import json
import os
import shutil
import sys
import time
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from hzmcp import Mcp  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
GAME = Path(os.path.expandvars(r"%APPDATA%\horizon"))
WORLD = "jade-verify"
Y = 150  # 平台高度，远离地形

# (名称, 方块位置, setblock 参数)
BLOCKS = [
    ("furnace", (3, Y + 1, 0), 'minecraft:furnace[facing=west,lit=true]{Items:[{Slot:0b,id:"minecraft:iron_ore",count:5},{Slot:1b,id:"minecraft:coal",count:3},{Slot:2b,id:"minecraft:iron_ingot",count:2}],cooking_time_spent:80s,cooking_total_time:200s,lit_time_remaining:1000s,lit_total_time:1600s}'),
    ("chest", (-3, Y + 1, 0), 'minecraft:chest[facing=east]{Items:[{Slot:0b,id:"minecraft:diamond",count:12},{Slot:1b,id:"minecraft:oak_log",count:64},{Slot:2b,id:"minecraft:oak_log",count:30},{Slot:5b,id:"minecraft:apple",count:1}]}'),
    ("wheat", (0, Y + 1, 3), "minecraft:wheat[age=3]"),
    ("comparator", (0, Y + 1, -3), "minecraft:comparator[facing=north,mode=subtract]"),
    ("note_block", (2, Y + 1, 2), "minecraft:note_block[instrument=harp,note=7]"),
    ("brewing_stand", (-2, Y + 1, 2), 'minecraft:brewing_stand{Items:[{Slot:0b,id:"minecraft:potion",count:1},{Slot:3b,id:"minecraft:nether_wart",count:4},{Slot:4b,id:"minecraft:blaze_powder",count:2}],BrewTime:200s,Fuel:15b}'),
]
# (名称, 实体类型, 位置, NBT)
ENTITIES = [
    ("baby_pig", "minecraft:pig", (-2, Y + 1, -3), "{Age:-12000,NoAI:1b}"),
    ("husk", "minecraft:husk", (3, Y + 1, -3), "{NoAI:1b,Health:13f,PersistenceRequired:1b}"),
]


def deploy():
    hmod = ROOT / "target/horizon-mod/jade.hmod"
    if not hmod.exists():
        sys.exit("缺少 target/horizon-mod/jade.hmod，先运行 cargo horizon-mod build --wasm")
    (GAME / "horizon-mods").mkdir(parents=True, exist_ok=True)
    shutil.copy2(hmod, GAME / "horizon-mods/jade.hmod")
    txt = GAME / "horizon-mods.txt"
    lines = txt.read_text(encoding="utf-8").splitlines() if txt.exists() else []
    lines = [l for l in lines if not l.startswith("mod.jade:")] + ["mod.jade:true"]
    txt.write_text("\n".join(lines) + "\n", encoding="utf-8")


def wait_game_exit(timeout):
    """等 horizon-client.exe 全部退出，返回是否在超时前退出。"""
    import subprocess
    deadline = time.time() + timeout
    while time.time() < deadline:
        out = subprocess.run(["tasklist", "/FI", "IMAGENAME eq horizon-client.exe", "/NH"],
                             capture_output=True, text=True, errors="replace").stdout
        if "horizon-client" not in out:
            return True
        time.sleep(1)
    return False


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    out = ROOT / "logs" / f"verify-{datetime.now():%Y-%m-%d %H-%M-%S}"
    out.mkdir(parents=True, exist_ok=True)
    log = open(out / "verify.log", "w", encoding="utf-8")

    def note(*parts):
        msg = " ".join(str(p) for p in parts)
        print(msg)
        log.write(msg + "\n")
        log.flush()

    if "--no-deploy" not in sys.argv:
        deploy()
        note("已部署 jade.hmod 到", GAME / "horizon-mods")

    mcp = Mcp()

    def call(name, args=None):
        err, text, _ = mcp.call(name, args)
        note(f"[{name}] {json.dumps(args or {}, ensure_ascii=False)} -> {'ERROR ' if err else ''}{text[:300]}")
        return err, text

    def cmd(c):
        return call("game_command", {"command": c})

    try:
        err, _ = call("play", {"world": WORLD, "creative": True, "offscreen": True})
        if err:
            sys.exit("启动失败")
        call("game_wait_ready", {"seconds": 180})
        for c in [
            "gamerule doDaylightCycle false", "gamerule doMobSpawning false", "time set day",
            "weather clear", "gamemode creative",
            f"fill -6 {Y} -6 6 {Y + 4} 6 air", f"fill -6 {Y} -6 6 {Y} 6 stone",
            f"setblock 0 {Y} 3 minecraft:farmland",
            "kill @e[type=!player]",
            f"tp @s 0.5 {Y + 1} 0.5",
        ]:
            cmd(c)
        for _, (x, y, z), spec in BLOCKS:
            cmd(f"setblock {x} {y} {z} {spec} replace")
        for _, kind, (x, y, z), nbt in ENTITIES:
            cmd(f"summon {kind} {x + 0.5} {y} {z + 0.5} {nbt}")
        time.sleep(2)

        for name, (x, y, z), _ in BLOCKS:
            call("game_look_at_block", {"x": x, "y": y, "z": z})
            time.sleep(1.5)  # 等服务端数据回来（每 5 tick 请求一次）
            call("game_looking_at")
            note(name, "截图:", mcp.screenshot(out / f"{name}.png"))

        _, text = call("game_entities", {"radius": 10})
        try:
            found = json.loads(text).get("entities", [])
        except Exception:
            found = []
        for name, kind, _, _ in ENTITIES:
            ent = next((e for e in found if str(e.get("type", e.get("kind", ""))).endswith(kind.split(":")[1])), None)
            if ent is None:
                note(name, "未找到实体")
                continue
            call("game_look_at_entity", {"id": ent["id"]})
            time.sleep(1.5)
            call("game_looking_at")
            note(name, "截图:", mcp.screenshot(out / f"{name}.png"))

        _, logs = call("game_logs", {"lines": 400})
        relevant = [l for l in logs.splitlines() if "jade" in l.lower() or "panic" in l.lower() or "wasm" in l.lower()]
        (out / "game-log-jade.txt").write_text("\n".join(relevant), encoding="utf-8")
        note(f"游戏日志中与 jade/wasm/panic 相关的行：{len(relevant)}")
    finally:
        if "--keep-open" not in sys.argv:
            try:
                call("game_quit")
            except Exception as e:
                note("退出失败:", e)
            # 游戏收到 quit 后还要保存、关闭几秒，期间占着启动器的 run.lock；
            # 等它真正退出，免得紧接着手动启动时报 "lock acquisition failed"。
            note("等待游戏进程退出:", wait_game_exit(60))
        mcp.close()
        note("输出目录:", out)
        log.close()


if __name__ == "__main__":
    main()
