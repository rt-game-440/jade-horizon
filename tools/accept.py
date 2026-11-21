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


def scene(name, setup=(), look=None, actions=(), frozen=(), wait=1.5, settings=None, install="both",
          theme="classic", anchor="top_center", expect="visible", after=(), check=None):
    """一个场景：搭建命令、看的目标、截图前的输入动作、冻结后固定动态值的命令。"""
    return {
        "name": name,
        "setup": list(setup),
        "look": look,
        "actions": list(actions),
        "frozen": list(frozen),
        "wait": wait,
        # 启动客户端前写入的设置（键 -> 值）；设置相同的场景放在同一批里共用一次启动。
        "settings": dict(settings or {}),
        # mod 安装组合：both（默认）、client_only（服务端不装）、server_only（客户端不装）。
        "install": install,
        # 提示框的配色与锚点，决定在截图里怎么把它找出来。
        "theme": theme,
        "anchor": anchor,
        # visible：截图里要有提示框并与基准比对；hidden：不该有提示框，不比对。
        "expect": expect,
        # 截完图之后的输入动作（比如把按键开关按回去）。
        "after": list(after),
        # 不截图的场景：check(ctl, srv, r) -> (是否通过, 说明)。
        "check": check,
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

def offline_uuid_ints(name):
    """离线模式玩家的 UUID（md5("OfflinePlayer:名字")），写成 NBT 的 [I;a,b,c,d]。"""
    import hashlib
    import struct

    raw = bytearray(hashlib.md5(f"OfflinePlayer:{name}".encode()).digest())
    raw[6] = raw[6] & 0x0F | 0x30
    raw[8] = raw[8] & 0x3F | 0x80
    return ",".join(str(v) for v in struct.unpack(">4i", bytes(raw)))


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
    # 单端安装（manifest 里 side = "client"，进服时不比对 jade）：
    # 只装客户端也能进服，提示框只有客户端知道的名字和图标，没有箱子内容；
    # 只装服务端时客户端没有 jade，进服正常、不画提示框。
    scene(
        "chest_client_only",
        setup=[f"setblock -3 {Y + 1} 0 minecraft:chest[facing=east]{CHEST_NBT}"],
        look=block(-3, Y + 1, 0),
        install="client_only",
    ),
    scene(
        "chest_server_only",
        setup=[f"setblock -3 {Y + 1} 0 minecraft:chest[facing=east]{CHEST_NBT}"],
        look=block(-3, Y + 1, 0),
        install="server_only",
        expect="hidden",
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
    # ---- 挖掘等级：铁矿要石镐以上，钻石矿要铁镐以上 ----
    scene(
        "harvest_iron_stone",
        setup=[f"setblock 0 {Y + 1} 3 minecraft:iron_ore"],
        look=block(0, Y + 1, 3),
        actions=["give minecraft:stone_pickaxe", "select minecraft:stone_pickaxe", "wait 0.5"],
    ),
    scene(
        "harvest_iron_iron",
        setup=[f"setblock 0 {Y + 1} 3 minecraft:iron_ore"],
        look=block(0, Y + 1, 3),
        actions=["give minecraft:iron_pickaxe", "select minecraft:iron_pickaxe", "wait 0.5"],
    ),
    scene(
        "harvest_diamond_stone",
        setup=[f"setblock 0 {Y + 1} 3 minecraft:diamond_ore"],
        look=block(0, Y + 1, 3),
        actions=["give minecraft:stone_pickaxe", "select minecraft:stone_pickaxe", "wait 0.5"],
    ),
    # ---- 主人（Alice 驯服的宠物）、物品展示框、有限时长的药水效果 ----
    scene(
        "wolf_owner",
        setup=[f"summon minecraft:wolf -1.5 {Y + 1} -2.5 {{NoAI:1b,Silent:1b,Owner:[I;{offline_uuid_ints(CLIENT)}]}}"],
        look=entity("minecraft:wolf"),
        actions=["key shift down"],
    ),
    scene(
        "cat_owner",
        setup=[f"summon minecraft:cat -1.5 {Y + 1} -2.5 {{NoAI:1b,Silent:1b,Owner:[I;{offline_uuid_ints(CLIENT)}]}}"],
        look=entity("minecraft:cat"),
        actions=["key shift down"],
    ),
    scene(
        "item_frame",
        setup=[
            f"setblock 0 {Y + 1} -5 minecraft:stone",
            f"summon minecraft:item_frame 0 {Y + 1} -4 {{Facing:3b,Item:{{id:\"minecraft:diamond\",count:1}}}}",
        ],
        look=point(0, Y + 1, -4.4),
    ),
    scene(
        "potion_timed",
        setup=[
            f"summon minecraft:zombie -1.5 {Y + 1} -2.5 {{NoAI:1b,Silent:1b,PersistenceRequired:1b,"
            'active_effects:[{id:"minecraft:regeneration",amplifier:0b,duration:1200}]}'
        ],
        look=entity("minecraft:zombie"),
        frozen=[
            "data merge entity @e[type=minecraft:zombie,limit=1] "
            '{Health:14f,active_effects:[{id:"minecraft:regeneration",amplifier:0b,duration:1200}]}'
        ],
    ),
    # ---- 按键开关（#12）：按一次消失，再按一次回来 ----
    scene(
        "overlay_key_hidden",
        setup=[f"setblock -3 {Y + 1} 0 minecraft:chest[facing=east]{CHEST_NBT}"],
        look=block(-3, Y + 1, 0),
        actions=["key key.keyboard.keypad.1 tap", "wait 0.8"],
        after=["key key.keyboard.keypad.1 tap"],
        expect="hidden",
    ),
    scene(
        "overlay_key_restored",
        setup=[f"setblock -3 {Y + 1} 0 minecraft:chest[facing=east]{CHEST_NBT}"],
        look=block(-3, Y + 1, 0),
        actions=["key key.keyboard.keypad.1 tap", "wait 0.5", "key key.keyboard.keypad.1 tap", "wait 0.8"],
    ),
    # ---- 设置（#11、#14）：启动前写好设置，同样设置的场景共用一次启动 ----
    scene(
        "setting_disabled",
        setup=[f"setblock -3 {Y + 1} 0 minecraft:chest[facing=east]{CHEST_NBT}"],
        look=block(-3, Y + 1, 0),
        settings={"enabled": False},
        expect="hidden",
    ),
    scene(
        "theme_light",
        setup=[f"setblock -3 {Y + 1} 0 minecraft:chest[facing=east]{CHEST_NBT}"],
        look=block(-3, Y + 1, 0),
        settings={"theme": 2},
        theme="light",
    ),
    scene(
        "theme_high_contrast",
        setup=[f"setblock 3 {Y + 1} 0 minecraft:furnace[facing=west,lit=true]{FURNACE_NBT}"],
        look=block(3, Y + 1, 0),
        frozen=[f"data merge block 3 {Y + 1} 0 {{cooking_time_spent:80s,cooking_total_time:200s}}"],
        settings={"theme": 3},
        theme="high_contrast",
    ),
    scene(
        "anchor_top_left",
        setup=[f"setblock -3 {Y + 1} 0 minecraft:chest[facing=east]{CHEST_NBT}"],
        look=block(-3, Y + 1, 0),
        settings={"anchor": 1},
        anchor="top_left",
    ),
    # mod 安装组合（#19）：试过“只装客户端”“只装服务端”，Horizon 的 mod 握手都直接拒绝进服
    # （Your mods don't match this server's），side = "both" 的 mod 没法做这两种组合，所以没有对应场景。
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


def server_running(srv):
    code, text = srv("status", "--json", timeout=30)
    try:
        phase = json.loads(text.splitlines()[-1]).get("phase", "")
    except Exception:
        phase = ""
    return code == 0 and phase == "running"


def start_server(srv, resources, jade=True):
    """建服务端目录、按需装 jade 并启动。已经按同样方式在跑就什么也不做，否则重启。"""
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
    # 记下模式和 mod 文件的修改时间：mod 重新构建过也要重启服务端才会加载新版。
    mode = f"{'jade' if jade else 'vanilla'}:{HMOD.stat().st_mtime_ns}"
    marker = sdir / ".jade-mode"
    running = server_running(srv)
    if running and marker.exists() and marker.read_text(encoding="utf-8") == mode:
        return
    if running:
        srv("stop", timeout=120)
    srv(
        "config",
        "set",
        f"engine.resources={resources}",
        "server.online_mode=false",
        f"server.port={PORT}",
        "world.difficulty=normal",
    )
    # 只装客户端的批次里世界存档记着 jade、服务端却没装：允许缺 mod 照样开服。
    toml = sdir / "server.toml"
    text = toml.read_text(encoding="utf-8")
    if "[missing_mods]" not in text:
        toml.write_text(text.rstrip() + "\n\n[missing_mods]\nallow = true\n", encoding="utf-8")
    (sdir / "horizon-mods").mkdir(exist_ok=True)
    shutil.copy2(HMOD, sdir / "horizon-mods" / "jade.hmod")
    (sdir / "horizon-mods.txt").write_text(f"mod.jade:{'true' if jade else 'false'}\n", encoding="utf-8")
    marker.write_text(mode, encoding="utf-8")
    srv("op", CLIENT)

    srv("start", timeout=120, check=True)
    for _ in range(180):
        if server_running(srv):
            break
        time.sleep(1)
    else:
        raise RuntimeError("服务端 180 秒内没起来")


def start_client(ctl, resources, offscreen=True, settings=None, with_mod=True, lang="en_us"):
    """起一个客户端进服。`settings` 是启动前写好的 jade 设置；`with_mod` 为 False 时不装 jade。"""
    gdir = WORK / "alice"
    gdir.mkdir(parents=True, exist_ok=True)
    prepare_options(gdir, lang)
    reset_settings(gdir, settings)
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
        *(["--dev-mod", HMOD] if with_mod else []),
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


def setup_world(srv):
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


def launch(ctl, srv, resources, offscreen=True, settings=None, install="both", lang="en_us"):
    """起服务端（按 install 决定装不装 jade），再离屏起一个客户端进服并搭好场地。"""
    start_server(srv, resources, jade=install != "client_only")
    start_client(ctl, resources, offscreen, settings, with_mod=install != "server_only", lang=lang)
    setup_world(srv)


def setting_dir(gdir):
    """游戏把 jade 的设置存成这个目录下每个键一个文本文件（值的写法见 SettingSpec::store）。"""
    return gdir / "horizon-mods" / ".horizon-mod-data" / "jade"


def set_setting(gdir, key, value):
    """写一项设置。必须在启动客户端前写：直接写文件不会触发 setting_changed。"""
    d = setting_dir(gdir)
    d.mkdir(parents=True, exist_ok=True)
    text = str(value).lower() if isinstance(value, bool) else str(value)
    (d / key).write_text(text, encoding="utf-8")


def reset_settings(gdir, settings=None):
    """清掉上次留下的设置，再写入这次要用的（默认全部用默认值）。"""
    shutil.rmtree(setting_dir(gdir), ignore_errors=True)
    for key, value in (settings or {}).items():
        set_setting(gdir, key, value)


def prepare_options(gdir, lang="en_us"):
    """关掉新手提示和聊天框：它们画在屏幕上方，会压住提示框或混进顶层截图。

    tutorialStep=none 决定加入世界时还给不给“Move with WASD / Jump with Space”提示；
    chatVisibility=2 隐藏聊天框（服务端命令的回显也在里面）。
    lang 每次都写死：基准图是英文的，手动在游戏里切过语言也不能影响验收。
    """
    path = gdir / "options.txt"
    lines = path.read_text(encoding="utf-8").splitlines() if path.exists() else []
    lines = [l for l in lines if not l.startswith(("tutorialStep:", "chatVisibility:", "lang:"))]
    lines += ["tutorialStep:none", "chatVisibility:2", f"lang:{lang}"]
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
    """跑一个场景并截图，返回裁剪出的提示框图；没看到提示框返回 None。截图本身存在 out/<场景>-screen.png。"""
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
    for action in sc["after"]:
        ctl(*action.split(), timeout=30)
    ctl("release", timeout=30)

    if not shot.exists():
        r.note(f"{name}: 截图没生成")
        return None
    return crop_tooltip(Image.open(shot), theme=sc["theme"], anchor=sc["anchor"])


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


def check_joined(ctl):
    """客户端已经进服、能正常玩（没装 jade 也一样）。"""
    code, text = ctl("state", timeout=30)
    return code == 0 and '"error"' not in text, text[:200]


def scan_logs():
    """服务端和客户端日志里和 jade 有关的 panic / 错误行。"""
    bad = []
    for path in (WORK / "server" / "logs" / "latest.log", WORK / "alice" / "logs" / "latest.log"):
        if not path.exists():
            continue
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
            low = line.lower()
            if "panicked" in low or "wasm trap" in low or ("jade" in low and any(w in low for w in ("error", "failed", "exception"))):
                bad.append(f"{path.parent.parent.name}: {line.strip()[:200]}")
    return bad


def batches_of(scenes):
    """按（安装组合, 设置）分批：同一批的场景共用一次客户端启动。"""
    out = {}
    for sc in scenes:
        out.setdefault((sc["install"], tuple(sorted(sc["settings"].items()))), []).append(sc)
    return list(out.items())


def run_scene(ctl, srv, sc, out, r, freeze, update, results):
    name = sc["name"]
    r.note(f"场景 {name} …")
    if sc["check"]:
        ok, text = sc["check"](ctl, srv, r)
        results[name] = "通过" if ok else f"失败（{text}）"
        r.note(f"{name}: {results[name]}")
        return
    actual = capture(ctl, srv, sc, out, r, freeze)
    if sc["expect"] == "hidden":
        shot_ok = (out / f"{name}-screen.png").exists()
        results[name] = "通过" if shot_ok and actual is None else "失败（不该有提示框）"
        r.note(f"{name}: {results[name]}")
        return
    if actual is None:
        results[name] = "没有 Jade 提示框"
        r.note(f"{name}: 没有 Jade 提示框")
        return
    actual.save(out / f"{name}.png")
    baseline_path = BASELINES / f"{name}.png"
    if update or not baseline_path.exists():
        actual.save(baseline_path)
        results[name] = "已写基准" if update else "已写基准（原来没有）"
        r.note(f"{name}: 基准 -> {baseline_path}")
        return
    ok, ratio, diff_image = compare(actual, Image.open(baseline_path))
    results[name] = "通过" if ok else f"失败（{ratio:.1%} 不同）"
    r.note(f"{name}: {'通过' if ok else '失败'}，不同像素 {ratio:.3%}")
    if not ok:
        actual.save(out / f"{name}-actual.png")
        if diff_image is not None:
            diff_image.save(out / f"{name}-diff.png")


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

    results, freeze, client_up = {}, None, False
    try:
        for (install, settings), batch in batches_of(scenes):
            label = f"{install}" + (f" {dict(settings)}" if settings else "")
            r.note(f"== 一批场景：{label}，共 {len(batch)} 个 ==")
            if client_up:
                ctl("quit", timeout=60)
                client_up = False
                time.sleep(3)
            launch(ctl, srv, resources, install=install, settings=dict(settings))
            client_up = True
            if freeze is None:
                freeze, before, first, second = freeze_supported(srv)
                stopped = "停住" if first == second else "仍在走"
                r.note(f"/tick freeze {'可用' if freeze else '不可用'}；冻结期间服务端游戏时间 {first} -> {second}（{stopped}）")
            for sc in batch:
                run_scene(ctl, srv, sc, out, r, freeze, update, results)
            bad = scan_logs()
            key = f"日志[{label}]"
            results[key] = "通过" if not bad else f"失败（{len(bad)} 行）"
            for line in bad[:10]:
                r.note(key, line)
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
    for name in [s["name"] for s in scenes] + [k for k in results if k.startswith(("日志[", "__error__"))]:
        if name in results:
            lines.append(f"{name}: {results[name]}")
    (out / "report.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
    r.note("结果:", json.dumps(results, ensure_ascii=False))
    r.note("输出目录:", out)

    failed = [n for n, v in results.items() if not (v == "通过" or v.startswith("已写基准"))]
    sys.exit(0 if not failed else 1)


if __name__ == "__main__":
    main()
