"""实机体验：起验收用的服务端和一个带窗口的客户端，摆好一圈演示物，然后把键鼠交给你。

用法：python -I tools/play.py           启动（先 cargo horizon-mod build --wasm）
      python -I tools/play.py --stop    关掉服务端和客户端
演示物都在出生点附近：马、村民、带盔甲的僵尸、盔甲架、画、掉落物、水、岩浆、黑曜石等。
"""
import sys
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import accept  # noqa: E402
from accept import CLIENT, HMOD, ROOT, WORK, Y, Run, find_game, launch  # noqa: E402

DEMO = [
    f'summon minecraft:horse -3 {Y + 1} -3 {{NoAI:1b,Silent:1b,attributes:[{{id:"minecraft:jump_strength",base:0.7d}}]}}',
    f'summon minecraft:villager 3 {Y + 1} -3 {{NoAI:1b,Silent:1b,VillagerData:{{profession:"minecraft:librarian",level:3,type:"minecraft:plains"}}}}',
    f'summon minecraft:zombie 0 {Y + 1} -4 {{NoAI:1b,Silent:1b,PersistenceRequired:1b,active_effects:[{{id:"minecraft:speed",amplifier:1b,duration:-1}},{{id:"minecraft:regeneration",amplifier:0b,duration:2400}}],equipment:{{head:{{id:"minecraft:iron_helmet",count:1}},chest:{{id:"minecraft:iron_chestplate",count:1}}}}}}',
    f'summon minecraft:wolf -4 {Y + 1} 1 {{NoAI:1b,Silent:1b,Owner:[I;0,0,0,0]}}',
    f'summon minecraft:armor_stand 4 {Y + 1} 1 {{equipment:{{head:{{id:"minecraft:diamond_helmet",count:1}},mainhand:{{id:"minecraft:diamond_sword",count:1}}}}}}',
    f'summon minecraft:item 2 {Y + 1.2} 3 {{Item:{{id:"minecraft:diamond",count:3}},PickupDelay:32767s,Age:-32768s,NoGravity:1b}}',
    f'setblock 0 {Y + 1} -6 minecraft:stone',
    f'summon minecraft:painting 0 {Y + 1} -5 {{variant:"minecraft:kebab",facing:0b}}',
    f'setblock -2 {Y} 3 water',
    f'setblock 0 {Y} 3 lava',
    f'setblock -2 {Y + 1} 5 minecraft:obsidian',
    f'setblock 3 {Y + 1} 5 minecraft:chest[facing=north]{{Items:[{{Slot:0b,id:"minecraft:diamond",count:12}},{{Slot:1b,id:"minecraft:apple",count:5}}]}}',
    f'summon minecraft:pig -1.5 {Y + 1} 3 {{NoAI:1b,Age:-12000}}',
]


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    out = ROOT / "logs"
    out.mkdir(exist_ok=True)
    r = Run(out, f"play-{datetime.now():%Y-%m-%d %H-%M-%S}.log")
    game, resources = find_game()
    srv = lambda *a, **kw: r.run([game / "horizon-server.exe", "--dir", WORK / "server", *a], **kw)  # noqa: E731
    ctl = lambda *a, **kw: r.run([game / "horizon-ctl.exe", "--socket", WORK / "alice.sock", *a], **kw)  # noqa: E731
    if "--stop" in sys.argv:
        ctl("quit", timeout=30)
        srv("stop", timeout=60)
        return
    if not HMOD.exists():
        sys.exit("缺少 target/horizon-mod/jade.hmod，先运行 cargo horizon-mod build --wasm")
    launch(ctl, srv, resources, offscreen=False)
    srv("cmd", "kill @e[type=!player]")
    for command in DEMO:
        srv("cmd", command)
    srv("cmd", f"give {CLIENT} minecraft:iron_pickaxe")
    r.note("已启动。窗口在前台，创造模式；Shift 看详细信息，切生存模式：horizon-server --dir target/accept/server cmd gamemode survival Alice")


if __name__ == "__main__":
    main()
