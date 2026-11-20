"""Jade 提示框的定位与检测，供验收脚本共用。

提示框画在屏幕顶部正中，底色是 Jade 的 0xF0100010（深紫，94% 不透明）。
叠加在任意背景上仍然是很暗的紫色；`tooltip_box` 在顶部居中区域里按颜色找出它，
返回包围盒，`crop_tooltip` 直接裁出提示框。
"""
from PIL import Image


# 提示框底色的判色：很暗，红蓝接近，绿色明显更低。
def _is_background(r, g, b):
    return r < 45 and g < 30 and b < 45 and abs(r - b) < 20


def _runs(counts, minimum):
    """`counts` 里所有连续 >= minimum 的段，按先后顺序返回 (start, end)。"""
    out = []
    start = None
    for i, n in enumerate(counts):
        if n >= minimum:
            if start is None:
                start = i
        elif start is not None:
            out.append((start, i - 1))
            start = None
    if start is not None:
        out.append((start, len(counts) - 1))
    return out


def tooltip_box(img, coverage=0.03, pad=2):
    """提示框在截图里的包围盒 (left, top, right, bottom)，找不到返回 None。

    只看顶部居中的区域（横向 20%–80%、纵向 35% 以内），夜里或洞穴里的暗色不参与。
    提示框是一条横带，所以先找出底色像素够多的连续行，再在这几行里找连续列（取包含
    屏幕中心的那一段），免得把世界里的暗处（比如物品展示框的边框）当成提示框。
    """
    img = img.convert("RGB")
    w, h = img.size
    left, top = int(w * 0.2), 0
    right, bottom = int(w * 0.8), max(1, int(h * 0.35))
    region = img.crop((left, top, right, bottom))
    rw, rh = region.size
    data = region.tobytes()

    rows = []
    for y in range(rh):
        base = y * rw * 3
        rows.append(sum(1 for x in range(rw) if _is_background(*data[base + x * 3 : base + x * 3 + 3])))
    for y0, y1 in _runs(rows, rw * coverage):
        if y1 - y0 + 1 < h * 0.012:
            continue
        height = y1 - y0 + 1
        cols = []
        for x in range(rw):
            cols.append(sum(1 for y in range(y0, y1 + 1) if _is_background(*data[(y * rw + x) * 3 : (y * rw + x) * 3 + 3])))
        # 列内的文字与图标会占掉一部分像素，这里只要有一成就够；
        # 提示框总是居中，所以取包含屏幕中心的那一段，左右两侧的暗处（比如展示框）不参与。
        span = next((r for r in _runs(cols, height * 0.1) if r[0] <= rw / 2 <= r[1]), None)
        if span is None:
            continue
        x0, x1 = span
        if x1 - x0 + 1 < w * 0.01:
            continue
        return (
            max(0, left + x0 - pad),
            max(0, top + y0 - pad),
            min(w, left + x1 + 1 + pad),
            min(h, top + y1 + 1 + pad),
        )
    return None


def crop_tooltip(img, pad=2):
    """裁出提示框；截图里没有提示框时返回 None。"""
    box = tooltip_box(img, pad=pad)
    return None if box is None else img.crop(box)


def hud_visible(png, coverage=0.03):
    """截图顶部中间是否有 Jade 提示框。`png` 是路径或已打开的图片。"""
    img = Image.open(png) if isinstance(png, (str, bytes)) or hasattr(png, "__fspath__") else png
    return tooltip_box(img, coverage=coverage) is not None
