"""实机检查界面：离屏起服务端和客户端，截取 Jade 设置页、按键绑定页，并连拍提示框淡入淡出。

用法：python -I tools/ui_check.py          （先 cargo horizon-mod build --wasm）
      python -I tools/ui_check.py --keep   检查完不关游戏，便于继续手动用 horizon-ctl 操作
输出：logs/ui-<时间>/
  config-1..4.png  Jade 设置页从上到下滚动的四屏；config-ui.txt 为第一屏控件列表
  keys.png         按键绑定页底部（Jade 的两个按键）
  anim-sheet.png   连拍联系表：前两行淡入，后两行淡出（顶部居中裁剪）
这些图需要目视检查：文字是否都已翻译、动画有无闪烁。脚本只检查控件是否存在。
"""
import argparse
import json
import sys
import time
from datetime import datetime
from pathlib import Path

from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
from accept import HMOD, ROOT, WORK, Y, Run, find_game, launch  # noqa: E402

CHEST = (
    f'setblock -3 {Y + 1} 0 minecraft:chest[facing=east]'
    '{Items:[{Slot:0b,id:"minecraft:diamond",count:12},{Slot:1b,id:"minecraft:apple",count:5}]}'
)
# 这些控件 id 来自游戏中文界面的标签（ctl ui 用标签生成 id），换语言后会变。
OPTIONS_MODS = "模组"
MOD_CONFIG = "config"
KEY_BINDS = ["按键控制", "按键绑定"]
CLOSE = ["完成", "回到游戏"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", action="store_true", help="结束后不关游戏")
    args = ap.parse_args()
    sys.stdout.reconfigure(encoding="utf-8")
    if not HMOD.exists():
        sys.exit("缺少 target/horizon-mod/jade.hmod，先运行 cargo horizon-mod build --wasm")
    out = ROOT / "logs" / f"ui-{datetime.now():%Y-%m-%d %H-%M-%S}"
    out.mkdir(parents=True)
    r = Run(out, "ui.log")
    game, resources = find_game()
    srv = lambda *a, **kw: r.run([game / "horizon-server.exe", "--dir", WORK / "server", *a], **kw)  # noqa: E731
    ctl = lambda *a, **kw: r.run([game / "horizon-ctl.exe", "--socket", WORK / "alice.sock", *a], **kw)  # noqa: E731
    problems = []

    def shot(name):
        ctl("screenshot", out / f"{name}.png", timeout=60)
        return out / f"{name}.png"

    def widgets():
        _, text = ctl("ui", timeout=30)
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            return {"widgets": []}

    def click(widget, wait=1.5):
        ctl("click", widget, timeout=30)
        time.sleep(wait)

    def close_screens():
        # 一路点“完成”“回到游戏”，直到没有界面。esc 在子页面里只回上一级，不可靠。
        for _ in range(6):
            ids = {w["id"] for w in widgets().get("widgets", [])}
            hit = next((c for c in CLOSE if c in ids), None)
            if hit is None:
                return
            click(hit, 1)

    try:
        launch(ctl, srv, resources, lang="zh_cn")
        # ---- Jade 设置页：选项 > 模组 > 搜 jade > 选中 > Config ----
        ctl("options", timeout=30)
        time.sleep(1.5)
        click(OPTIONS_MODS)
        click("field-0", 0.5)
        ctl("type", "jade", timeout=30)
        ctl("key", "tab", "tap", timeout=30)
        ctl("key", "down", "tap", timeout=30)
        time.sleep(0.5)
        click(MOD_CONFIG)
        page = widgets()
        (out / "config-ui.txt").write_text(json.dumps(page, ensure_ascii=False, indent=1), encoding="utf-8")
        labels = [w.get("label", "") for w in page.get("widgets", [])]
        if not any(l.startswith("启用") for l in labels):
            problems.append(f"没进到 Jade 设置页，控件：{labels}")
        if any("config.jade" in l for l in labels):
            problems.append(f"设置页有未翻译的键：{labels}")
        shot("config-1")
        for i in range(2, 5):
            ctl("scroll-at", "213", "120", "-5", timeout=30)
            time.sleep(0.6)
            shot(f"config-{i}")
        close_screens()
        # ---- 按键绑定页：滚到底，Jade 的分组在最后 ----
        ctl("options", timeout=30)
        time.sleep(1.5)
        for widget in KEY_BINDS:
            click(widget)
        for _ in range(12):
            ctl("scroll-at", "213", "120", "-10", timeout=30)
        time.sleep(0.8)
        keys = [w.get("label", "") for w in widgets().get("widgets", [])]
        if not {"小键盘 1", "小键盘 2"} <= set(keys):
            problems.append(f"按键绑定页没找到 Jade 的两个按键：{keys[-12:]}")
        shot("keys")
        close_screens()
        # ---- 动画连拍：看天 -> 转向箱子（淡入）-> 再看天（淡出）----
        srv("cmd", CHEST)
        frames = []
        for phase, look in (("in", ("look-at-block", "-3", str(Y + 1), "0")), ("out", ("look", "90", "-80"))):
            if phase == "in":
                ctl("look", "90", "-80", timeout=30)
                time.sleep(1)
            else:
                time.sleep(1)
            ctl(*look, timeout=30)
            frames += [shot(f"anim-{phase}-{i}") for i in range(8)]
        crops = [Image.open(f).crop((880, 0, 1680, 220)).resize((400, 110)) for f in frames]
        sheet = Image.new("RGB", (1600, 440), "white")
        for k, c in enumerate(crops):
            sheet.paste(c, ((k % 4) * 400, (k // 4) * 110))
        sheet.save(out / "anim-sheet.png")
    finally:
        if not args.keep:
            ctl("quit", timeout=30)
            srv("stop", timeout=60)
    for p in problems:
        r.note("问题：", p)
    print(json.dumps({"out": str(out), "problems": problems}, ensure_ascii=False))
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
