"""多人 e2e 验收：一个装了 jade 的专用服务端 + 两个客户端（Alice、Bob）互相瞄准，截图看 Jade 提示框。

用法：python -I tools/e2e_mp.py [--keep]   （--keep：结束后不关服务端和客户端）

做的事：
1. 用启动器已安装版本里的 horizon-server.exe 在 target/e2e/server 建离线服务端（端口 25599，无白名单），
   把 target/horizon-mod/jade.hmod 放进它的 horizon-mods/ 并启用；
2. 用 horizon-ctl 离屏启动两个客户端，各用独立游戏目录（target/e2e/<名字>）与控制 socket，
   `--dev-mod` 加载同一个 jade.hmod，`--server` 直接进服；
3. 服务端搭平台，把两人传送到相距 3 格、面对面；
4. 各自对准对方，记录 looking-at，截图；
5. 收集双方客户端与服务端日志里和 jade/wasm/panic 有关的行；最后退出客户端、停服务端。

输出：logs/e2e-<时间>/（alice.png、bob.png、e2e.log、*-jade.txt）。
"""
import json
import os
import shutil
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LAUNCHER_DATA = Path(os.path.expandvars(r"%APPDATA%\horizon-launcher"))
PORT = 25599
Y = 150
PLAYERS = {"Alice": (0.5, Y + 1, 0.5), "Bob": (3.5, Y + 1, 0.5)}
E2E = ROOT / "target" / "e2e"
HMOD = ROOT / "target" / "horizon-mod" / "jade.hmod"

sys.path.insert(0, str(Path(__file__).resolve().parent))
from hud import hud_visible  # noqa: E402


def find_game():
    """启动器当前运行目录（run/<hash>/game）里的 horizon-server/ctl，以及对应版本的预处理资源。"""
    runs = sorted((LAUNCHER_DATA / "run").glob("*/game/horizon-server.exe"), key=lambda p: p.stat().st_mtime)
    if not runs:
        sys.exit("没找到启动器安装的游戏（%APPDATA%\\horizon-launcher\\run\\*\\game），先在启动器里玩一次")
    game = runs[-1].parent
    source = json.loads((game.parent / "source.json").read_text(encoding="utf-8"))
    version_dir = Path(source.replace("\\\\?\\", ""))
    resources = version_dir / ".local" / "horizon-client" / "26.3"
    if not resources.exists():
        sys.exit(f"没找到预处理资源：{resources}")
    return game, resources


class Run:
    def __init__(self, out, log_name="e2e.log"):
        self.out = out
        self.log = open(out / log_name, "w", encoding="utf-8")

    def note(self, *parts):
        msg = " ".join(str(p) for p in parts)
        print(msg, flush=True)
        self.log.write(msg + "\n")
        self.log.flush()

    def run(self, args, timeout=120, check=False):
        # 输出写临时文件而不是管道：`server start`、`ctl launch` 会留下后台进程，
        # 它们继承管道后 subprocess 会一直等管道关闭而卡死。
        import tempfile
        with tempfile.TemporaryFile() as buf:
            try:
                code = subprocess.run([str(a) for a in args], stdin=subprocess.DEVNULL, stdout=buf,
                                      stderr=subprocess.STDOUT, timeout=timeout).returncode
            except subprocess.TimeoutExpired:
                code = -1
            buf.seek(0)
            text = buf.read().decode("utf-8", errors="replace").strip()
        self.note(f"$ {' '.join(str(a) for a in args[1:])}\n  -> [{code}] {text[:400]}")
        if check and code != 0:
            raise RuntimeError(f"命令失败：{args}")
        return code, text


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    if not HMOD.exists():
        sys.exit("缺少 target/horizon-mod/jade.hmod，先运行 cargo horizon-mod build --wasm")
    out = ROOT / "logs" / f"e2e-{datetime.now():%Y-%m-%d %H-%M-%S}"
    out.mkdir(parents=True, exist_ok=True)
    r = Run(out)
    game, resources = find_game()
    server_exe, ctl = game / "horizon-server.exe", game / "horizon-ctl.exe"
    sdir = E2E / "server"
    r.note("游戏目录:", game)

    def srv(*args, **kw):
        return r.run([server_exe, "--dir", sdir, *args], **kw)

    def ctl_of(name):
        sock = E2E / f"{name.lower()}.sock"
        return lambda *args, **kw: r.run([ctl, "--socket", sock, *args], **kw)

    # ---- 服务端 ----
    sdir.mkdir(parents=True, exist_ok=True)
    if not (sdir / "server.toml").exists():
        srv("init", "--eula", "--world", "e2e", "--port", PORT, "--no-whitelist", "--offline",
            "--op", "Alice", "--op", "Bob", "--yes", timeout=300)
    srv("config", "set", f"engine.resources={resources}", "server.online_mode=false",
        f"server.port={PORT}", "world.difficulty=peaceful")
    (sdir / "horizon-mods").mkdir(exist_ok=True)
    shutil.copy2(HMOD, sdir / "horizon-mods" / "jade.hmod")
    (sdir / "horizon-mods.txt").write_text("mod.jade:true\n", encoding="utf-8")
    for name in PLAYERS:
        srv("op", name)

    clients = {name: ctl_of(name) for name in PLAYERS}
    results, ok = {}, False
    try:
        srv("start", timeout=120, check=True)
        for _ in range(180):
            code, text = srv("status", "--json", timeout=30)
            try:
                phase = json.loads(text.splitlines()[-1]).get("phase", "")
            except Exception:
                phase = ""
            if code == 0 and phase == "running":
                break
            time.sleep(1)
        else:
            raise RuntimeError("服务端 180 秒内没起来")
        for c in ["gamerule doDaylightCycle false", "gamerule doMobSpawning false", "time set day",
                  "weather clear"]:
            srv("cmd", c)

        # ---- 两个客户端 ----
        for name, ctl_run in clients.items():
            gdir = E2E / name.lower()
            gdir.mkdir(parents=True, exist_ok=True)
            ctl_run("launch", "--offscreen", "--wait", "--server", f"127.0.0.1:{PORT}", "--name", name,
                    "--game-dir", gdir, "--home", gdir, "--resources", resources, "--dev-mod", HMOD, "--width", "1280", "--height", "720",
                    "--render-distance", "4", timeout=300, check=True)
        for name, ctl_run in clients.items():
            ctl_run("wait-ready", "120", timeout=150)

        # 两人进服后由 Alice（op）搭平台、传送：无人在线时区块未加载，服务端 fill 会落空，
        # 玩家会掉进海里漂动，准星对不稳。
        alice = clients["Alice"]
        alice("command", f"fill -4 {Y} -4 8 {Y + 4} 4 air", timeout=30)
        alice("command", f"fill -4 {Y} -4 8 {Y} 4 stone", timeout=30)
        alice("command", f"kill @e[type=!player,x=2,y={Y},z=0,distance=..12]", timeout=30)
        for name, ctl_run in clients.items():
            x, y, z = PLAYERS[name]
            ctl_run("command", "gamemode creative", timeout=30)
            alice("command", f"tp {name} {x} {y} {z}", timeout=30)
        time.sleep(3)
        for name, ctl_run in clients.items():
            _, text = ctl_run("state", timeout=30)
            (out / f"{name.lower()}-state.json").write_text(text, encoding="utf-8")
            try:
                p = json.loads(text).get("player", {})
                r.note(f"{name} 位置:", p.get("pos") or p.get("position") or [p.get(k) for k in ("x", "y", "z")])
            except Exception:
                pass

        # ---- 互相瞄准并截图 ----
        for name, ctl_run in clients.items():
            other = next(n for n in PLAYERS if n != name)
            _, text = ctl_run("entities", "10", timeout=30)
            target = None
            try:
                for e in json.loads(text).get("entities", []):
                    if "player" in str(e.get("type", "")):
                        target = e["id"]
                        break
            except Exception:
                pass
            if target is None:
                r.note(f"{name}: 附近没找到另一名玩家")
                results[name] = False
                continue
            ctl_run("look-at-entity", target, timeout=30)
            time.sleep(2)  # 等 Jade 请求服务端数据并回包
            _, looking = ctl_run("looking-at", timeout=30)
            ctl_run("screenshot", out / f"{name.lower()}.png", timeout=60)
            shot = out / f"{name.lower()}.png"
            results[name] = '"entity"' in looking and shot.exists() and hud_visible(shot)
            r.note(f"{name} 看向 {other}: 准星命中实体 = {results[name]}")

        # ---- 对照：Alice 看方块与普通生物，区分"多人下 HUD 全不显示"与"只有玩家目标不显示" ----
        alice("command", f'setblock 0 {Y + 1} 3 minecraft:chest{{Items:[{{Slot:0b,id:"minecraft:diamond",count:7}}]}}', timeout=30)
        time.sleep(1)
        alice("look-at-block", 0, Y + 1, 3, timeout=30)
        time.sleep(2)
        alice("looking-at", timeout=30)
        alice("screenshot", out / "alice-chest.png", timeout=60)
        results["Alice→箱子"] = hud_visible(out / "alice-chest.png")
        alice("command", f"summon minecraft:pig 0.5 {Y + 1} -2.5 {{NoAI:1b,Age:-6000}}", timeout=30)
        time.sleep(2)
        _, text = alice("entities", "10", "minecraft:pig", timeout=30)
        try:
            pig = json.loads(text)["entities"][0]["id"]
            alice("look-at-entity", pig, timeout=30)
            time.sleep(2)
            alice("looking-at", timeout=30)
            alice("screenshot", out / "alice-pig.png", timeout=60)
            results["Alice→猪"] = hud_visible(out / "alice-pig.png")
        except Exception as e:
            r.note("对照：没找到猪", e)
            results["Alice→猪"] = False

        # ---- 日志 ----
        keys = ("jade", "wasm", "panic", "payload")
        for name, ctl_run in clients.items():
            _, text = ctl_run("logs", "--lines", "600", timeout=30)
            lines = [l for l in text.splitlines() if any(k in l.lower() for k in keys)]
            (out / f"{name.lower()}-jade.txt").write_text("\n".join(lines), encoding="utf-8")
        _, text = srv("logs", "-n", "600", timeout=30)
        lines = [l for l in text.splitlines() if any(k in l.lower() for k in keys)]
        (out / "server-jade.txt").write_text("\n".join(lines), encoding="utf-8")
        ok = bool(results) and all(results.values())
        r.note("结果:", json.dumps(results, ensure_ascii=False), "通过" if ok else "失败")
    except Exception as e:
        ok = False
        r.note("出错:", e)
    finally:
        if "--keep" not in sys.argv:
            for name, ctl_run in clients.items():
                try:
                    ctl_run("quit", timeout=60)
                except Exception as e:
                    r.note(f"{name} 退出失败:", e)
            try:
                srv("stop", timeout=120)
            except Exception as e:
                r.note("服务端停止失败:", e)
        r.note("输出目录:", out)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
