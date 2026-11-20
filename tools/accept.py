"""Jade 验收：一个专用服务端 + 一个客户端，逐场景截图，与基准图逐像素比对。

用法：
  python -I tools/accept.py                     比对全部场景
  python -I tools/accept.py --only 名称          只跑指定场景（可重复给）
  python -I tools/accept.py --update            重写全部基准
  python -I tools/accept.py --update --only 名称  只重写受影响场景
  python -I tools/accept.py --keep              结束后保留服务端和客户端

一个装了 jade 的专用服务端加一个离屏客户端（Alice），所以服务端数据链路也会被覆盖。
窗口固定 1280×720；截图前 /tick freeze，截完立即 /tick unfreeze，免得场景里的
计时（熔炉进度、生物成长时间）在截图时还在走。

截图画面上只取顶部居中的 Jade 提示框（见 tools/hud.py），所以世界背景的细微差别
不影响比对。逐像素比较：任一通道相差超过 32 的像素超过 1% 判为失败；尺寸不同也判失败。
失败时在 logs/accept-<时间>/ 下写 <场景>-actual.png、<场景>-diff.png（差异标红）和 report.txt。
基准图在 tests/baselines/<场景>.png。
"""
import json
import math
import shutil
import sys
import time
from datetime import datetime
from pathlib import Path

from PIL import Image, ImageChops

sys.path.insert(0, str(Path(__file__).resolve().parent))
from e2e_mp import HMOD, Run, find_game  # noqa: E402
from hud import crop_tooltip  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
PORT = 25598
Y = 150
CLIENT = "Alice"
WORK = ROOT / "target" / "accept"
BASELINES = ROOT / "tests" / "baselines"
# 任一通道相差超过它的像素算“不同”，占提示框的比例超过 TOLERANCE 判为失败。
# 同一场景重复截图的差异实测为 0.000%；单个数字变化约 0.23%，所以 0.1% 就能抓到。
PIXEL_DELTA = 32
TOLERANCE = 0.001

FLOOR = f"fill -6 {Y} -6 6 {Y} 6 stone"
CLEAR = f"fill -6 {Y + 1} -6 6 {Y + 4} 6 air"
# 站在 y+1 时的眼睛高度（原版玩家为 1.62）。
EYE_Y = Y + 1 + 1.62


def block(x, y, z):
    return ("block", x, y, z)


def entity(kind):
    return ("entity", kind)


def point(x, y, z):
    """看向一个方块的几何中心（用 look 而不是 look-at-block：流体不是方块目标）。"""
    return ("point", x, y, z)


def yaw_pitch(x, y, z):
    """从固定站位看向方块中心的 yaw/pitch（原版：pitch 正数向下）。"""
    dx, dy, dz = x, y + 0.5 - EYE_Y, z
    return math.degrees(math.atan2(-dx, dz)), math.degrees(math.atan2(-dy, math.hypot(dx, dz)))


def scene(name, setup=(), look=None, actions=(), frozen=(), wait=1.5):
    """一个场景：搭建命令、看的目标、截图前的输入动作、冻结后固定动态值的命令。"""
    return {
        "name": name,
        "setup": list(setup),
        "look": look,
        "actions": list(actions),
        "frozen": list(frozen),
        "wait": wait,
    }


FURNACE_NBT = (
    '{Items:[{Slot:0b,id:"minecraft:iron_ore",count:5},{Slot:1b,id:"minecraft:coal",count:3},'
    '{Slot:2b,id:"minecraft:iron_ingot",count:2}],cooking_time_spent:80s,cooking_total_time:200s,'
    "lit_time_remaining:1000s,lit_total_time:1600s}"
)
CHEST_NBT = (
    '{Items:[{Slot:0b,id:"minecraft:diamond",count:12},{Slot:1b,id:"minecraft:oak_log",count:64},'
    '{Slot:2b,id:"minecraft:oak_log",count:30},{Slot:5b,id:"minecraft:apple",count:1}]}'
)
CHEST_MORE_NBT = (
    "{Items:["
    + ",".join(
        f'{{Slot:{i}b,id:"minecraft:{item}",count:{count}}}'
        for i, (item, count) in enumerate(
            [
                ("diamond", 12),
                ("iron_ingot", 32),
                ("gold_ingot", 16),
                ("emerald", 8),
                ("coal", 64),
                ("redstone", 45),
                ("lapis_lazuli", 20),
                ("quartz", 15),
                ("copper_ingot", 24),
                ("apple", 5),
                ("bread", 9),
            ]
        )
    )
    + "]}"
)
BREWING_NBT = (
    '{Items:[{Slot:0b,id:"minecraft:potion",count:1},{Slot:3b,id:"minecraft:nether_wart",count:4},'
    '{Slot:4b,id:"minecraft:blaze_powder",count:2}],BrewTime:200s,Fuel:15b}'
)

SCENES = [
    scene(
        "furnace",
        setup=[f"setblock 3 {Y + 1} 0 minecraft:furnace[facing=west,lit=true]{FURNACE_NBT}"],
        look=block(3, Y + 1, 0),
        frozen=[f"data merge block 3 {Y + 1} 0 {{cooking_time_spent:80s,cooking_total_time:200s}}"],
    ),
    scene(
        "chest",
        setup=[f"setblock -3 {Y + 1} 0 minecraft:chest[facing=east]{CHEST_NBT}"],
        look=block(-3, Y + 1, 0),
    ),
    scene(
        "chest_more",
        setup=[f"setblock -3 {Y + 1} 0 minecraft:chest[facing=east]{CHEST_MORE_NBT}"],
        look=block(-3, Y + 1, 0),
    ),
    scene(
        "chest_more_shift",
        setup=[f"setblock -3 {Y + 1} 0 minecraft:chest[facing=east]{CHEST_MORE_NBT}"],
        look=block(-3, Y + 1, 0),
        actions=["key shift down"],
    ),
    scene(
        "wheat",
        setup=[f"setblock 0 {Y} 3 minecraft:farmland", f"setblock 0 {Y + 1} 3 minecraft:wheat[age=3]"],
        look=block(0, Y + 1, 3),
    ),
    scene(
        "comparator",
        setup=[f"setblock 0 {Y + 1} -3 minecraft:comparator[facing=north,mode=subtract]"],
        look=block(0, Y + 1, -3),
    ),
    scene(
        "repeater",
        setup=[f"setblock 1 {Y + 1} 1 minecraft:repeater[facing=north,delay=3]"],
        look=block(1, Y + 1, 1),
    ),
    scene(
        "note_block",
        setup=[f"setblock 2 {Y + 1} 2 minecraft:note_block[instrument=harp,note=7]"],
        look=block(2, Y + 1, 2),
    ),
    scene(
        "beehive",
        setup=[f"setblock -1 {Y + 1} 1 minecraft:beehive[facing=north,honey_level=3]"],
        look=block(-1, Y + 1, 1),
    ),
    scene(
        "brewing_stand",
        setup=[f"setblock -2 {Y + 1} 2 minecraft:brewing_stand{BREWING_NBT}"],
        look=block(-2, Y + 1, 2),
        frozen=[f"data merge block -2 {Y + 1} 2 {{BrewTime:200s,Fuel:15b}}"],
    ),
    scene(
        "baby_pig",
        setup=[f"summon minecraft:pig -1.5 {Y + 1} -2.5 {{NoAI:1b,Age:-12000}}"],
        look=entity("minecraft:pig"),
        frozen=["data merge entity @e[type=minecraft:pig,limit=1] {Age:-12000}"],
    ),
    scene(
        "baby_pig_shift",
        setup=[f"summon minecraft:pig -1.5 {Y + 1} -2.5 {{NoAI:1b,Age:-12000}}"],
        look=entity("minecraft:pig"),
        actions=["key shift down"],
        frozen=["data merge entity @e[type=minecraft:pig,limit=1] {Age:-12000}"],
    ),
    scene(
        "husk",
        setup=[f"summon minecraft:husk 2.5 {Y + 1} -2.5 {{NoAI:1b,Health:13f,PersistenceRequired:1b}}"],
        look=entity("minecraft:husk"),
    ),
    # 挖掘信息：石头上用铁镐（能采集），黑曜石用木镐（需要钻石镐，采不下来），
    # 泥土不需要正确工具（这一行不显示）。
    scene(
        "harvest_stone",
        setup=[f"setblock 0 {Y + 1} 3 minecraft:stone"],
        look=block(0, Y + 1, 3),
        actions=["give minecraft:iron_pickaxe", "select minecraft:iron_pickaxe", "wait 0.5"],
    ),
    scene(
        "harvest_obsidian",
        setup=[f"setblock 0 {Y + 1} 3 minecraft:obsidian"],
        look=block(0, Y + 1, 3),
        actions=["give minecraft:wooden_pickaxe", "select minecraft:wooden_pickaxe", "wait 0.5"],
    ),
    scene(
        "harvest_dirt",
        setup=[f"setblock 0 {Y + 1} 3 minecraft:dirt"],
        look=block(0, Y + 1, 3),
        actions=["give minecraft:wooden_shovel", "select minecraft:wooden_shovel", "wait 0.5"],
    ),
    # 挖掘进度：生存模式按住左键挖黑曜石（木镐要 37 秒），进度条稳定在很低的百分比。
    scene(
        "mining",
        setup=[f"setblock 0 {Y + 1} 3 minecraft:obsidian", "gamemode survival Alice"],
        look=block(0, Y + 1, 3),
        actions=[
            "give minecraft:wooden_pickaxe",
            "select minecraft:wooden_pickaxe",
            "mouse left down",
            "wait 3.0",
        ],
    ),
    # 松开之后进度条要消失。
    scene(
        "mining_stopped",
        setup=[f"setblock 0 {Y + 1} 3 minecraft:obsidian", "gamemode survival Alice"],
        look=block(0, Y + 1, 3),
        actions=[
            "give minecraft:wooden_pickaxe",
            "select minecraft:wooden_pickaxe",
            "mouse left down",
            "wait 0.6",
            "mouse left up",
            "wait 0.4",
        ],
    ),
    # 流体：在地板上挖一格做水池，四周都是石头，液体不会流走。
    scene("water", setup=[f"setblock 0 {Y} 2 water"], look=point(0, Y, 2)),
    scene("lava", setup=[f"setblock 0 {Y} 2 lava"], look=point(0, Y, 2)),
    # 海草：合水的方块仍然显示方块信息，而不是“水”。
    scene(
        "seagrass",
        setup=[f"setblock 0 {Y} 2 water", f"setblock 0 {Y} 2 minecraft:seagrass"],
        look=point(0, Y, 2),
    ),
    # 实体信息（#6）：盔甲值、药水效果、马类、村民、主人、掉落物、盔甲架、画。
    scene(
        "armored_zombie",
        setup=[
            f"summon minecraft:zombie -1.5 {Y + 1} -2.5 "
            '{NoAI:1b,Silent:1b,PersistenceRequired:1b,active_effects:[{id:"minecraft:speed",amplifier:1b,duration:-1}],'
            'equipment:{head:{id:"minecraft:iron_helmet",count:1},chest:{id:"minecraft:iron_chestplate",count:1}}}'
        ],
        look=entity("minecraft:zombie"),
    ),
    scene(
        "horse",
        setup=[
            f"summon minecraft:horse -1.5 {Y + 1} -2.5 "
            "{NoAI:1b,Silent:1b,attributes:[{id:\"minecraft:jump_strength\",base:0.7d},{id:\"minecraft:movement_speed\",base:0.225d}]}"
        ],
        look=entity("minecraft:horse"),
    ),
    scene(
        "villager",
        setup=[
            f"summon minecraft:villager -1.5 {Y + 1} -2.5 "
            '{NoAI:1b,Silent:1b,VillagerData:{profession:"minecraft:librarian",level:2,type:"minecraft:plains"}}'
        ],
        look=entity("minecraft:villager"),
    ),
    scene(
        "dropped_item",
        setup=[
            f"summon minecraft:item -1.5 {Y + 1.2} -2.5 "
            '{Item:{id:"minecraft:diamond",count:3},PickupDelay:32767s,Age:-32768s,NoGravity:1b}'
        ],
        look=point(-2, Y + 0.82, -3),
    ),
    scene(
        "armor_stand",
        setup=[
            f"summon minecraft:armor_stand -1.5 {Y + 1} -2.5 "
            '{equipment:{head:{id:"minecraft:iron_helmet",count:1},mainhand:{id:"minecraft:diamond_sword",count:1}}}'
        ],
        look=point(-1.5, Y + 1, -2.5),
    ),
    scene(
        "painting",
        setup=[
            f"setblock 0 {Y + 1} -5 minecraft:stone",
            f"summon minecraft:painting 0 {Y + 1} -4 "
            '{variant:"minecraft:kebab",facing:0b}',
        ],
        look=point(0, Y + 1, -4.4),
    ),
]

# 截图前一刻 /tick freeze，截完立即 /tick unfreeze。
FREEZE_COMMAND = "tick freeze"
UNFREEZE_COMMAND = "tick unfreeze"


def freeze_supported(srv):
    """核实 /tick freeze 可用，并看冻结期间服务端的游戏时间停没停。

    返回 (可用, 冻结前, 冻结中第一次, 冻结中第二次)。冻结会让限流窗口不再前进，
    所以只该在截图前后短暂冻结，不能整段运行都冻着。
    """
    before = server_time(srv)
    code, text = srv("cmd", FREEZE_COMMAND)
    time.sleep(0.3)
    first = server_time(srv)
    time.sleep(0.5)
    second = server_time(srv)
    srv("cmd", UNFREEZE_COMMAND)
    return code == 0 and "frozen" in text.lower(), before, first, second


def server_time(srv):
    """服务端的游戏时间（tick），用来确认冻结有没有停住它。"""
    _, text = srv("cmd", "time query gametime", timeout=30)
    digits = "".join(ch for ch in text if ch.isdigit())
    return int(digits) if digits else None


def launch(ctl, srv, resources, offscreen=True):
    """建服务端目录、装 jade、起服务端，再离屏起一个客户端进服。"""
    sdir = WORK / "server"
    sdir.mkdir(parents=True, exist_ok=True)
    if not (sdir / "server.toml").exists():
        srv(
            "init",
            "--eula",
            "--world",
            "accept",
            "--port",
            PORT,
            "--no-whitelist",
            "--offline",
            "--op",
            CLIENT,
            "--yes",
            timeout=300,
        )
    srv(
        "config",
        "set",
        f"engine.resources={resources}",
        "server.online_mode=false",
        f"server.port={PORT}",
        "world.difficulty=normal",
    )
    (sdir / "horizon-mods").mkdir(exist_ok=True)
    shutil.copy2(HMOD, sdir / "horizon-mods" / "jade.hmod")
    (sdir / "horizon-mods.txt").write_text("mod.jade:true\n", encoding="utf-8")
    srv("op", CLIENT)

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

    gdir = WORK / "alice"
    gdir.mkdir(parents=True, exist_ok=True)
    prepare_options(gdir)
    ctl(
        "launch",
        *(["--offscreen"] if offscreen else []),
        "--wait",
        "--server",
        f"127.0.0.1:{PORT}",
        "--name",
        CLIENT,
        "--game-dir",
        gdir,
        "--home",
        gdir,
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
    for command in [
        # 26.3 的 gamerule 名是下划线形式：doDaylightCycle → advance_time，doMobSpawning → spawn_mobs。
        "gamerule advance_time false",
        "gamerule spawn_mobs false",
        "gamerule spawn_monsters false",
        "time set 6000",
        "weather clear",
    ]:
        srv("cmd", command)
    srv("cmd", FLOOR)
    srv("cmd", CLEAR)
    srv("cmd", "gamemode creative Alice")
    srv("cmd", f"tp Alice 0.5 {Y + 1} 0.5")
    time.sleep(1)


def prepare_options(gdir):
    """关掉新手提示和聊天框：它们画在屏幕上方，会压住提示框或混进顶层截图。

    tutorialStep=none 决定加入世界时还给不给“Move with WASD / Jump with Space”提示；
    chatVisibility=2 隐藏聊天框（服务端命令的回显也在里面）。
    """
    path = gdir / "options.txt"
    lines = path.read_text(encoding="utf-8").splitlines() if path.exists() else []
    lines = [l for l in lines if not l.startswith(("tutorialStep:", "chatVisibility:"))]
    lines += ["tutorialStep:none", "chatVisibility:2"]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def reset(srv):
    """每个场景开始前清场：物品、生物、玩家位置和视角都回到同一状态。"""
    srv("cmd", "gamemode creative Alice")
    srv("cmd", "time set 6000")
    srv("cmd", "weather clear")
    srv("cmd", "kill @e[type=!player]")
    # 手上的工具也清掉：挖掘信息的场景会往快捷栏放镐子。
    srv("cmd", f"clear {CLIENT}")
    # 地面也重铺一次：上一个场景可能把地板换成了耕地之类。
    srv("cmd", FLOOR)
    srv("cmd", CLEAR)
    srv("cmd", f"tp Alice 0.5 {Y + 1} 0.5")


def look_at(ctl, target, r):
    """让玩家看向场景目标：方块用坐标，实体先按类型找到最近的 id。"""
    if target is None:
        return False
    kind = target[0]
    if kind == "point":
        _, x, y, z = target
        yaw, pitch = yaw_pitch(x, y, z)
        ctl("look", f"{yaw:.3f}", f"{pitch:.3f}", timeout=30)
        return True
    if kind == "block":
        _, x, y, z = target
        ctl("look-at-block", x, y, z, timeout=30)
        return True
    _, text = ctl("entities", "10", target[1], timeout=30)
    try:
        found = [e for e in json.loads(text).get("entities", []) if e.get("health", 1) > 0]
    except Exception:
        found = []
    if not found:
        r.note(f"没找到实体 {target[1]}")
        return False
    ctl("look-at-entity", found[0]["id"], timeout=30)
    return True


def capture(ctl, srv, sc, out, r, freeze):
    """跑一个场景并截图，返回裁剪出的提示框图；没看到提示框返回 None。"""
    name = sc["name"]
    reset(srv)
    for command in sc["setup"]:
        srv("cmd", command)
    time.sleep(0.6)
    if not look_at(ctl, sc["look"], r):
        return None
    time.sleep(sc["wait"])

    for action in sc["actions"]:
        if action.startswith("wait "):
            time.sleep(float(action.split(None, 1)[1]))
        elif action.startswith("command "):
            ctl("command", action.split(None, 1)[1], timeout=30)
        else:
            ctl(*action.split(), timeout=30)
    # 按住 Shift 会潜行、视角变低，要重新对准一次。
    if any("shift" in a for a in sc["actions"]):
        look_at(ctl, sc["look"], r)
        time.sleep(0.5)

    if freeze:
        srv("cmd", FREEZE_COMMAND)
    for command in sc["frozen"]:
        srv("cmd", command)
    if freeze and sc["frozen"]:
        # 冻结后固定动态值，等客户端下一次请求（每 10 tick 一次）拿到新数据。
        time.sleep(0.8)
    shot = out / f"{name}-screen.png"
    ctl("screenshot", shot, timeout=60)
    if freeze:
        srv("cmd", UNFREEZE_COMMAND)
    ctl("release", timeout=30)

    if not shot.exists():
        r.note(f"{name}: 截图没生成")
        return None
    return crop_tooltip(Image.open(shot))


def compare(actual, baseline):
    """逐像素比较，返回 (是否通过, 不同像素占比, 差异图)。"""
    if actual.size != baseline.size:
        return False, 1.0, None
    a, b = actual.convert("RGB"), baseline.convert("RGB")
    diff = ImageChops.difference(a, b)
    hot = None
    for channel in diff.split():
        marked = channel.point(lambda v: 255 if v > PIXEL_DELTA else 0)
        hot = marked if hot is None else ImageChops.lighter(hot, marked)
    total = actual.size[0] * actual.size[1]
    ratio = hot.histogram()[255] / total
    gray = a.convert("L").point(lambda v: v // 3)
    diff_image = Image.composite(Image.new("RGB", actual.size, (255, 0, 0)), Image.merge("RGB", (gray, gray, gray)), hot)
    return ratio <= TOLERANCE, ratio, diff_image


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    update = "--update" in sys.argv
    keep = "--keep" in sys.argv
    only = [sys.argv[i + 1] for i, a in enumerate(sys.argv) if a == "--only" and i + 1 < len(sys.argv)]
    scenes = [s for s in SCENES if not only or s["name"] in only]
    missing = sorted(set(only) - {s["name"] for s in SCENES})
    if missing:
        sys.exit(f"没有这些场景：{missing}")
    if not scenes:
        sys.exit("没有要跑的场景")
    if not HMOD.exists():
        sys.exit("缺少 target/horizon-mod/jade.hmod，先运行 cargo horizon-mod build --wasm")

    out = ROOT / "logs" / f"accept-{datetime.now():%Y-%m-%d %H-%M-%S}"
    out.mkdir(parents=True, exist_ok=True)
    BASELINES.mkdir(parents=True, exist_ok=True)
    r = Run(out, "accept.log")
    game, resources = find_game()
    server_exe, ctl_exe = game / "horizon-server.exe", game / "horizon-ctl.exe"
    r.note("游戏目录:", game)

    srv = lambda *args, **kw: r.run([server_exe, "--dir", WORK / "server", *args], **kw)  # noqa: E731
    ctl = lambda *args, **kw: r.run([ctl_exe, "--socket", WORK / "alice.sock", *args], **kw)  # noqa: E731

    results, freeze = {}, False
    try:
        launch(ctl, srv, resources)
        freeze, before, first, second = freeze_supported(srv)
        stopped = "停住" if first == second else "仍在走"
        r.note(f"/tick freeze {'可用' if freeze else '不可用'}；冻结期间服务端游戏时间 {first} -> {second}（{stopped}）")
        for sc in scenes:
            name = sc["name"]
            r.note(f"场景 {name} …")
            actual = capture(ctl, srv, sc, out, r, freeze)
            if actual is None:
                results[name] = "没有 Jade 提示框"
                r.note(f"{name}: 没有 Jade 提示框")
                continue
            actual.save(out / f"{name}.png")
            baseline_path = BASELINES / f"{name}.png"
            if update or not baseline_path.exists():
                actual.save(baseline_path)
                results[name] = "已写基准" if update else "已写基准（原来没有）"
                r.note(f"{name}: 基准 -> {baseline_path}")
                continue
            ok, ratio, diff_image = compare(actual, Image.open(baseline_path))
            results[name] = "通过" if ok else f"失败（{ratio:.1%} 不同）"
            r.note(f"{name}: {'通过' if ok else '失败'}，不同像素 {ratio:.3%}")
            if not ok:
                actual.save(out / f"{name}-actual.png")
                if diff_image is not None:
                    diff_image.save(out / f"{name}-diff.png")
    except Exception as e:
        r.note("出错:", e)
        results["__error__"] = str(e)
    finally:
        if not keep:
            try:
                ctl("quit", timeout=60)
            except Exception as e:
                r.note("客户端退出失败:", e)
            try:
                srv("stop", timeout=120)
            except Exception as e:
                r.note("服务端停止失败:", e)
        else:
            r.note("保留服务端和客户端（--keep）：", WORK)

    lines = [f"accept {datetime.now():%Y-%m-%d %H:%M:%S}", f"freeze: {freeze}"]
    for name in [s["name"] for s in scenes] + [k for k in results if k == "__error__"]:
        if name in results:
            lines.append(f"{name}: {results[name]}")
    (out / "report.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
    r.note("结果:", json.dumps(results, ensure_ascii=False))
    r.note("输出目录:", out)

    failed = [n for n, v in results.items() if not (v == "通过" or v.startswith("已写基准"))]
    sys.exit(0 if not failed else 1)


if __name__ == "__main__":
    main()
