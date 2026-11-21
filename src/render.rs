//! 把 [`Tooltip`] 画到 HUD 上，外观仿 Jade 默认（原版 tooltip 风格）；配色、位置与动画见 [`Theme`]、[`Anim`]。
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use mod_api::DrawContext;

use crate::client::{Row, TankKind, Tone, Tooltip};
use crate::config::{Anchor, Config, CustomColors, ThemeId};

const PAD: f32 = 4.0;
const LINE: f32 = 10.0;
const SLOT: f32 = 18.0;
const ICON_W: f32 = 20.0;

/// 一套配色。`bg` 只取 RGB，透明度由设置的“背景不透明度”决定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub bg: u32,
    pub border_top: u32,
    pub border_bottom: u32,
    pub title: u32,
    pub mod_name: u32,
    pub text: u32,
    pub warn: u32,
    /// ✔ 与进度已完成部分。
    pub ok: u32,
    /// 进度条的空槽、箭头未完成部分。
    pub dim: u32,
    /// 文字是否带阴影（浅色底上带阴影反而发糊）。
    pub shadow: bool,
}

impl Theme {
    /// 当前设置对应的配色：自定义主题用设置里的五个颜色。
    pub fn from_config(cfg: &Config) -> Theme {
        match cfg.theme {
            ThemeId::Custom => Theme::custom(cfg.custom),
            id => Theme::of(id),
        }
    }

    /// 自定义主题：背景、边框、标题、正文、模组名由玩家定，警告、完成、空槽沿用深色主题的值。
    pub fn custom(c: CustomColors) -> Theme {
        let dark = Theme::of(ThemeId::Dark);
        let opaque = |rgb: u32| 0xFF00_0000 | (rgb & 0x00FF_FFFF);
        // 边框下沿比上沿暗一档，和内置主题的渐变一致。
        let darker = |rgb: u32| {
            let ch = |shift: u32| ((rgb >> shift & 0xFF) * 3 / 5) << shift;
            0xFF00_0000 | ch(16) | ch(8) | ch(0)
        };
        Theme {
            bg: 0xF000_0000 | (c.bg & 0x00FF_FFFF),
            border_top: opaque(c.border),
            border_bottom: darker(c.border),
            title: opaque(c.title),
            mod_name: opaque(c.accent),
            text: opaque(c.text),
            ..dark
        }
    }

    pub fn of(id: ThemeId) -> Theme {
        match id {
            // 原版 tooltip 风：深紫底、紫色渐变边框。
            ThemeId::Classic => Theme {
                bg: 0xF010_0010,
                border_top: 0x5050_00FF,
                border_bottom: 0x5028_007F,
                title: 0xFFFF_FFFF,
                mod_name: 0xFF55_55FF,
                text: 0xFFAA_AAAA,
                warn: 0xFFFF_5555,
                ok: 0xFF55_FF55,
                dim: 0xFF55_5555,
                shadow: true,
            },
            ThemeId::Dark => Theme {
                bg: 0xF011_1111,
                border_top: 0xFF44_4444,
                border_bottom: 0xFF2A_2A2A,
                title: 0xFFFF_FFFF,
                mod_name: 0xFF8F_A8FF,
                text: 0xFFCC_CCCC,
                warn: 0xFFFF_6B6B,
                ok: 0xFF55_FF55,
                dim: 0xFF55_5555,
                shadow: true,
            },
            ThemeId::Light => Theme {
                bg: 0xF0F2_F2F2,
                border_top: 0xFF9A_9A9A,
                border_bottom: 0xFF7A_7A7A,
                title: 0xFF11_1111,
                mod_name: 0xFF2A_3FA0,
                text: 0xFF33_3333,
                warn: 0xFFB0_0020,
                ok: 0xFF1B_7A1B,
                dim: 0xFFB5_B5B5,
                shadow: false,
            },
            ThemeId::HighContrast => Theme {
                bg: 0xFF00_0000,
                border_top: 0xFFFF_FF00,
                border_bottom: 0xFFFF_FF00,
                title: 0xFFFF_FFFF,
                mod_name: 0xFF8C_D2FF,
                text: 0xFFFF_FFFF,
                warn: 0xFFFF_6B6B,
                ok: 0xFF55_FF55,
                dim: 0xFF80_8080,
                shadow: false,
            },
            // 没有设置时的自定义主题；真正的颜色走 `Theme::custom`。
            ThemeId::Custom => Theme::custom(CustomColors::default()),
        }
    }

    fn tone(&self, tone: Tone) -> u32 {
        match tone {
            Tone::Normal => self.text,
            Tone::Warn => self.warn,
        }
    }

    /// 设置里的不透明度换成背景色（只改 alpha 通道）。
    fn background(&self, alpha: f32) -> u32 {
        ((alpha.clamp(0.0, 1.0) * 255.0).round() as u32) << 24 | (self.bg & 0x00FF_FFFF)
    }
}

/// 画一帧要用的东西：主题、整体透明度（动画）。
#[derive(Clone, Copy)]
struct Style {
    theme: Theme,
    /// 0..1，乘到所有颜色的 alpha 上。
    fade: f32,
}

impl Style {
    /// 颜色乘上整体透明度；完全不透明时原样返回。
    fn c(&self, color: u32) -> u32 {
        fade(color, self.fade)
    }

    fn items(&self) -> bool {
        // draw.item 不支持透明度：淡入淡出到一半以下就不画图标。
        self.fade >= 0.5
    }

    fn tint(&self) -> Option<u32> {
        (self.fade < 1.0).then(|| fade(0xFFFF_FFFF, self.fade))
    }
}

/// 颜色的 alpha 通道乘上 `a`（0..1）。
fn fade(color: u32, a: f32) -> u32 {
    if a >= 1.0 {
        return color;
    }
    let alpha = ((color >> 24) as f32 * a.clamp(0.0, 1.0)).round() as u32;
    alpha << 24 | (color & 0x00FF_FFFF)
}

fn row_height(row: &Row) -> f32 {
    match row {
        Row::Line(..) | Row::Health(..) | Row::Armor(..) => LINE,
        Row::Items(_) | Row::Progress { .. } | Row::Harvest { .. } => SLOT,
        Row::Tank { .. } => TANK_H + 2.0,
    }
}

fn row_width(draw: &dyn DrawContext, row: &Row) -> f32 {
    match row {
        Row::Line(t, _) => draw.text_width(t),
        Row::Health(_, max) => {
            if *max <= 20.0 {
                (max / 2.0).ceil().min(10.0) * 8.0 + 1.0
            } else {
                draw.text_width(&health_text(0.0, *max)) + 2.0
            }
        }
        Row::Armor(armor) => (armor / 2.0).ceil().min(10.0) * 8.0 + 1.0,
        Row::Items(items) => items.len() as f32 * SLOT,
        Row::Progress { inputs, outputs, .. } => (inputs.len() + outputs.len()) as f32 * SLOT + 26.0,
        Row::Harvest { tool, .. } => (if tool.is_some() { SLOT } else { 0.0 }) + MARK_W,
        Row::Tank { text, .. } => draw.text_width(text) + 8.0,
    }
}

fn health_text(hp: f32, max: f32) -> String {
    format!("❤ {:.1} / {:.1}", hp, max)
}

/// 提示框左上角：按锚点贴边（边距 4），再加玩家调的偏移。
fn place(anchor: Anchor, [screen_w, screen_h]: [f32; 2], [w, h]: [f32; 2], offset: [i32; 2]) -> [f32; 2] {
    let (x, y) = match anchor {
        Anchor::TopCenter => (((screen_w - w) / 2.0).floor(), 4.0),
        Anchor::TopLeft => (4.0, 4.0),
        Anchor::TopRight => (screen_w - w - 4.0, 4.0),
        Anchor::MiddleLeft => (4.0, ((screen_h - h) / 2.0).floor()),
        Anchor::MiddleRight => (screen_w - w - 4.0, ((screen_h - h) / 2.0).floor()),
    };
    [x + offset[0] as f32, y + offset[1] as f32]
}

/// 提示框排版出来的尺寸。
struct Layout {
    w: f32,
    h: f32,
    icon_w: f32,
    /// 底部为破坏进度条多留的高度。
    extra: f32,
}

fn layout(draw: &dyn DrawContext, tip: &Tooltip) -> Layout {
    let icon_w = if tip.icon.is_some() { ICON_W } else { 0.0 };
    let mut width = draw.text_width(&tip.title).max(draw.text_width(&tip.mod_name));
    let mut height = if tip.mod_name.is_empty() { LINE } else { LINE * 2.0 };
    for row in &tip.rows {
        width = width.max(row_width(draw, row));
        height += row_height(row);
    }
    let content_h = height.max(if tip.icon.is_some() { 16.0 } else { 0.0 });
    // 有破坏进度时底部多留 2px 画进度条。
    let extra = if tip.breaking.is_some() { BREAK_H } else { 0.0 };
    Layout {
        w: (width + icon_w + PAD * 2.0).ceil(),
        h: (content_h + PAD * 2.0 - 2.0 + extra).ceil(),
        icon_w,
        extra,
    }
}

/// 动画状态：整体透明度与当前（平滑中的）宽高。
#[derive(Clone, Debug, Default)]
pub struct Anim {
    alpha: f32,
    w: f32,
    h: f32,
    last_ms: Option<u64>,
    /// 目标消失后用它画完淡出。
    last_tip: Option<Tooltip>,
}

/// 淡入、淡出的时长，宽高指数平滑的时间常数（毫秒）。
const FADE_IN_MS: f32 = 100.0;
const FADE_OUT_MS: f32 = 150.0;
const SIZE_TAU_MS: f32 = 60.0;
/// 离目标不足这么多像素就直接取目标值。
const SNAP_PX: f32 = 0.5;

/// 一帧的动画结果。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub alpha: f32,
    pub w: f32,
    pub h: f32,
    /// 宽高还没到目标：内容要裁在当前框内。
    pub morphing: bool,
}

impl Anim {
    /// 推进到 `now_ms`。`target` 是目标尺寸（没有提示框时为 None）；返回这一帧要画的状态，
    /// 完全淡出后返回 None。`animate` 为 false 时直接取目标值。
    pub fn step(&mut self, target: Option<[f32; 2]>, now_ms: u64, animate: bool) -> Option<Frame> {
        let dt = self.last_ms.map_or(0.0, |last| now_ms.saturating_sub(last) as f32).min(250.0);
        self.last_ms = Some(now_ms);
        if !animate {
            return match target {
                Some([w, h]) => {
                    self.alpha = 1.0;
                    self.w = w;
                    self.h = h;
                    Some(Frame { alpha: 1.0, w, h, morphing: false })
                }
                None => {
                    self.alpha = 0.0;
                    self.last_tip = None;
                    None
                }
            };
        }
        match target {
            Some([tw, th]) => {
                if self.alpha <= 0.0 {
                    // 刚出现：直接以目标尺寸淡入，不从零长大。
                    self.w = tw;
                    self.h = th;
                }
                self.alpha = (self.alpha + dt / FADE_IN_MS).min(1.0);
                let k = 1.0 - (-dt / SIZE_TAU_MS).exp();
                self.w = approach(self.w, tw, k);
                self.h = approach(self.h, th, k);
                Some(Frame { alpha: self.alpha, w: self.w, h: self.h, morphing: self.w != tw || self.h != th })
            }
            None => {
                self.alpha = (self.alpha - dt / FADE_OUT_MS).max(0.0);
                if self.alpha <= 0.0 {
                    self.last_tip = None;
                    return None;
                }
                Some(Frame { alpha: self.alpha, w: self.w, h: self.h, morphing: false })
            }
        }
    }
}

fn approach(current: f32, target: f32, k: f32) -> f32 {
    let next = current + (target - current) * k;
    if (target - next).abs() < SNAP_PX { target } else { next }
}

fn anim() -> &'static Mutex<Anim> {
    static ANIM: OnceLock<Mutex<Anim>> = OnceLock::new();
    ANIM.get_or_init(Default::default)
}

/// 每帧调用：`tip` 是当前要显示的提示框（没有时传 None，让淡出画完）。
pub fn draw(draw: &mut dyn DrawContext, tip: Option<&Tooltip>, cfg: &Config, now_ms: u64) {
    let mut anim = anim().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(tip) = tip {
        anim.last_tip = Some(tip.clone());
    }
    let Some(shown) = anim.last_tip.clone() else {
        return;
    };
    let lay = layout(draw, &shown);
    let target = tip.map(|_| [lay.w, lay.h]);
    let Some(frame) = anim.step(target, now_ms, cfg.animation) else {
        return;
    };
    drop(anim);
    let style = Style { theme: Theme::from_config(cfg), fade: frame.alpha };
    let [x, y] = place(cfg.anchor, draw.size(), [frame.w, frame.h], [cfg.offset_x, cfg.offset_y]);
    draw_box(draw, &shown, &lay, &style, cfg.alpha, [x, y, frame.w, frame.h], frame.morphing);
}

fn draw_box(draw: &mut dyn DrawContext, tip: &Tooltip, lay: &Layout, st: &Style, alpha: f32, [x, y, w, h]: [f32; 4], clip: bool) {
    background(draw, st, [x, y, w, h], st.theme.background(alpha));
    // 尺寸过渡中内容可能比当前框大：裁在框内，画完一定要关掉。
    let clipped = clip && draw.push_clip([x, y, w, h]).is_ok();
    if let Some(icon) = tip.icon {
        if st.items() {
            let iy = y + (h - lay.extra - 16.0) / 2.0;
            draw.item(icon, [x + PAD, iy.floor()], 16.0);
        }
    }
    let tx = x + PAD + lay.icon_w;
    let mut ty = y + PAD;
    text(draw, st, &tip.title, [tx, ty], st.theme.title);
    ty += LINE;
    for row in &tip.rows {
        draw_row(draw, st, row, tx, ty);
        ty += row_height(row);
    }
    if !tip.mod_name.is_empty() {
        text(draw, st, &tip.mod_name, [tx, ty], st.theme.mod_name);
    }
    if let Some(progress) = tip.breaking {
        break_bar(draw, st, [x, y, w, h], progress);
    }
    if clipped {
        let _ = draw.pop_clip();
    }
}

fn text(draw: &mut dyn DrawContext, st: &Style, text: &str, at: [f32; 2], color: u32) {
    draw.text(text, at, st.c(color), st.theme.shadow);
}

/// 破坏进度条的高度。
const BREAK_H: f32 = 2.0;
const BREAK_FILL: u32 = 0xFFFF_FFFF;

/// 提示框底部的破坏进度：1px 的槽 + 1px 的已完成部分，左右各留 1px 不压边框。
fn break_bar(draw: &mut dyn DrawContext, st: &Style, [x, y, w, h]: [f32; 4], progress: f32) {
    let width = (w - 2.0).max(0.0);
    let top = y + h - BREAK_H;
    draw.fill([x + 1.0, top, width, 1.0], st.c(st.theme.dim));
    let filled = (width * progress.clamp(0.0, 1.0)).round().min(width);
    if filled > 0.0 {
        draw.fill([x + 1.0, top, filled, 1.0], st.c(if st.theme.shadow { BREAK_FILL } else { st.theme.title }));
    }
}

/// 背景：底色加渐变边框。
fn background(draw: &mut dyn DrawContext, st: &Style, [x, y, w, h]: [f32; 4], bg: u32) {
    let (bg, top, bottom) = (st.c(bg), st.c(st.theme.border_top), st.c(st.theme.border_bottom));
    draw.fill([x - 1.0, y, w + 2.0, h], bg);
    draw.fill([x, y - 1.0, w, 1.0], bg);
    draw.fill([x, y + h, w, 1.0], bg);
    draw.fill_gradient([x, y + 1.0, 1.0, h - 2.0], top, bottom);
    draw.fill_gradient([x + w - 1.0, y + 1.0, 1.0, h - 2.0], top, bottom);
    draw.fill([x, y, w, 1.0], top);
    draw.fill([x, y + h - 1.0, w, 1.0], bottom);
}

fn draw_row(draw: &mut dyn DrawContext, st: &Style, row: &Row, x: f32, y: f32) {
    match row {
        Row::Line(line, tone) => text(draw, st, line, [x, y], st.theme.tone(*tone)),
        Row::Health(hp, max) => health(draw, st, *hp, *max, x, y),
        Row::Armor(armor) => armor_row(draw, st, *armor, x, y),
        Row::Items(items) => {
            for (i, (item, count)) in items.iter().enumerate() {
                slot(draw, st, *item, *count, x + i as f32 * SLOT, y);
            }
        }
        Row::Progress { inputs, outputs, fraction } => {
            let mut cx = x;
            for (item, count) in inputs {
                slot(draw, st, *item, *count, cx, y);
                cx += SLOT;
            }
            arrow(draw, st, cx + 2.0, y + 4.0, *fraction);
            cx += 26.0;
            for (item, count) in outputs {
                slot(draw, st, *item, *count, cx, y);
                cx += SLOT;
            }
        }
        Row::Harvest { tool, ok } => harvest(draw, st, *tool, *ok, x, y),
        Row::Tank { kind, fraction, text } => tank(draw, st, *kind, *fraction, text, x, y),
    }
}

/// 储罐条的高度。
const TANK_H: f32 = 10.0;

fn tank_color(kind: TankKind) -> u32 {
    match kind {
        TankKind::Energy => 0xFFD0_2A2A,
        TankKind::Water => 0xFF3F_76E4,
        TankKind::Lava => 0xFFE0_7A1F,
        TankKind::Other => 0xFF8A_8A8A,
    }
}

/// 能量/流体条：空槽打底，已有的部分按种类着色，文字叠在条上。
fn tank(draw: &mut dyn DrawContext, st: &Style, kind: TankKind, fraction: f32, label: &str, x: f32, y: f32) {
    let width = draw.text_width(label) + 8.0;
    let top = y + 1.0;
    draw.fill([x, top, width, TANK_H], st.c(st.theme.dim));
    let filled = (width * fraction.clamp(0.0, 1.0)).round();
    if filled > 0.0 {
        draw.fill([x, top, filled, TANK_H], st.c(tank_color(kind)));
    }
    draw.text(label, [x + 4.0, top + 1.0], st.c(0xFFFF_FFFF), true);
}

const MARK_W: f32 = 9.0;

/// 需要的工具：先画工具图标，再画绿色的✔（能采集）或红色的✘（不能）。
/// 对勾与叉用像素点自绘，不依赖字体里有没有这些字形。
fn harvest(draw: &mut dyn DrawContext, st: &Style, tool: Option<mod_api::ItemKey>, ok: bool, x: f32, y: f32) {
    let mut cx = x;
    if let Some(item) = tool {
        if st.items() {
            draw.item(item, [x + 1.0, y + 1.0], 16.0);
        }
        cx += SLOT;
    }
    mark(draw, st, ok, cx + 1.0, y + 5.0);
}

/// ✔ 的像素：从左上降到底部再升到右上。
const CHECK: [(i32, i32); 7] = [(0, 3), (1, 4), (2, 5), (3, 4), (4, 3), (5, 2), (6, 1)];
/// ✘ 的两条对角线。
const CROSS: [(i32, i32); 12] =
    [(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (5, 0), (4, 1), (3, 2), (2, 3), (1, 4), (0, 5)];

fn mark(draw: &mut dyn DrawContext, st: &Style, ok: bool, x: f32, y: f32) {
    let (pixels, color) = if ok { (CHECK.as_slice(), st.theme.ok) } else { (CROSS.as_slice(), st.theme.warn) };
    for (c, r) in pixels {
        draw.fill([x + *c as f32, y + *r as f32, 1.0, 1.0], st.c(color));
    }
}

fn slot(draw: &mut dyn DrawContext, st: &Style, item: mod_api::ItemKey, count: i32, x: f32, y: f32) {
    if st.items() {
        draw.item(item, [x + 1.0, y + 1.0], 16.0);
    }
    if count > 1 {
        let label = short_count(count);
        let tw = draw.text_width(&label);
        text(draw, st, &label, [x + 17.0 - tw, y + 9.0], st.theme.title);
    }
}

fn short_count(count: i32) -> String {
    if count >= 10_000 { format!("{}k", count / 1000) } else { count.to_string() }
}

/// 进度箭头：逐列填色画出 22×11 的箭头，已完成部分白色、其余灰色。
/// （原版熔炉箭头贴图在 HUD 上会带出容器底色，故自绘。）
fn arrow(draw: &mut dyn DrawContext, st: &Style, x: f32, y: f32, fraction: f32) {
    const W: i32 = 22;
    const HEAD: i32 = 6; // 箭头头部宽度
    let mid = y + 4.5; // 箭杆竖直中心，使各列上下沿落在整像素
    let done = (W as f32 * fraction.clamp(0.0, 1.0)).round() as i32;
    for c in 0..W {
        let half = if c < W - HEAD { 1.5 } else { (W - c) as f32 - 0.5 };
        let color = if c < done { st.theme.title } else { st.theme.dim };
        draw.fill([x + c as f32, mid - half, 1.0, half * 2.0], st.c(color));
    }
}

fn health(draw: &mut dyn DrawContext, st: &Style, hp: f32, max: f32, x: f32, y: f32) {
    if max > 20.0 {
        text(draw, st, &health_text(hp, max), [x, y], st.theme.warn);
        return;
    }
    let hearts = (max / 2.0).ceil() as i32;
    for i in 0..hearts.min(10) {
        let hx = x + i as f32 * 8.0;
        let dst = [hx, y, 9.0, 9.0];
        if !gui_sprite(draw, "hud/heart/container", [0.0, 0.0, 9.0, 9.0], dst, st.tint()) {
            // 没有心形贴图时退回文字。
            text(draw, st, &health_text(hp, max), [x, y], st.theme.warn);
            return;
        }
        let filled = hp - i as f32 * 2.0;
        if filled >= 2.0 {
            gui_sprite(draw, "hud/heart/full", [0.0, 0.0, 9.0, 9.0], dst, st.tint());
        } else if filled > 0.0 {
            gui_sprite(draw, "hud/heart/half", [0.0, 0.0, 9.0, 9.0], dst, st.tint());
        }
    }
}

/// 盔甲值：满、半图标，每图标 2 点；超过 20 点画满 10 个。
fn armor_row(draw: &mut dyn DrawContext, st: &Style, armor: f32, x: f32, y: f32) {
    let icons = (armor / 2.0).ceil().min(10.0) as i32;
    for i in 0..icons {
        let full = armor - i as f32 * 2.0 >= 2.0;
        let path = if full { "hud/armor_full" } else { "hud/armor_half" };
        if !gui_sprite(draw, path, [0.0, 0.0, 9.0, 9.0], [x + i as f32 * 8.0, y, 9.0, 9.0], st.tint()) {
            text(draw, st, &format!("🛡 {armor:.0}"), [x, y], st.theme.text);
            return;
        }
    }
}

/// GUI 图集里贴图名的几种写法；第一个画成功的写法会被记住，以后只用它。
const SPRITE_FORMS: [&str; 4] = ["minecraft:gui/sprites/", "minecraft:", "gui/sprites/", "minecraft:gui/"];
static SPRITE_FORM: AtomicUsize = AtomicUsize::new(usize::MAX);

/// 画原版 GUI 贴图（`path` 如 `hud/heart/full`），找不到时返回 false。
fn gui_sprite(draw: &mut dyn DrawContext, path: &str, src: [f32; 4], dst: [f32; 4], tint: Option<u32>) -> bool {
    let known = SPRITE_FORM.load(Ordering::Relaxed);
    if known < SPRITE_FORMS.len() {
        return draw.sprite(&format!("{}{path}", SPRITE_FORMS[known]), src, dst, tint);
    }
    for (i, form) in SPRITE_FORMS.iter().enumerate() {
        if draw.sprite(&format!("{form}{path}"), src, dst, tint) {
            SPRITE_FORM.store(i, Ordering::Relaxed);
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 相对亮度与对比度。
    fn luminance(rgb: u32) -> f64 {
        let channel = |shift: u32| {
            let c = ((rgb >> shift) & 0xFF) as f64 / 255.0;
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// 背景按自己的 alpha 混合到黑底上。
    fn over_black(argb: u32) -> u32 {
        let a = (argb >> 24) as f64 / 255.0;
        let mix = |shift: u32| ((((argb >> shift) & 0xFF) as f64 * a).round() as u32) << shift;
        mix(16) | mix(8) | mix(0)
    }

    #[test]
    fn classic_theme_keeps_the_original_colors() {
        let t = Theme::of(ThemeId::Classic);
        assert_eq!(t.bg, 0xF010_0010);
        assert_eq!((t.border_top, t.border_bottom), (0x5050_00FF, 0x5028_007F));
        assert_eq!((t.title, t.mod_name, t.text, t.warn), (0xFFFF_FFFF, 0xFF55_55FF, 0xFFAA_AAAA, 0xFFFF_5555));
        assert_eq!((t.ok, t.dim), (0xFF55_FF55, 0xFF55_5555));
        assert!(t.shadow);
        // 默认不透明度正好是原来的 0xF0。
        assert_eq!(t.background(0.94), 0xF010_0010);
    }

    #[test]
    fn background_alpha_only_touches_the_alpha_channel() {
        let t = Theme::of(ThemeId::Light);
        assert_eq!(t.background(0.0), 0x00F2_F2F2);
        assert_eq!(t.background(1.0), 0xFFF2_F2F2);
        assert_eq!(t.background(7.0), 0xFFF2_F2F2);
    }

    #[test]
    fn high_contrast_text_is_readable() {
        let t = Theme::of(ThemeId::HighContrast);
        let bg = over_black(t.background(1.0));
        for (name, color) in [("title", t.title), ("mod_name", t.mod_name), ("text", t.text), ("warn", t.warn), ("ok", t.ok)] {
            let ratio = contrast(color & 0xFF_FFFF, bg);
            assert!(ratio >= 4.5, "{name} 对比度只有 {ratio:.2}");
        }
    }

    #[test]
    fn custom_theme_uses_given_colors_and_keeps_text_readable_fields() {
        let c = CustomColors { bg: 0x112233, border: 0xFFFFFF, title: 0xAABBCC, text: 0x010203, accent: 0x445566 };
        let t = Theme::custom(c);
        assert_eq!(t.bg & 0x00FF_FFFF, 0x112233);
        assert_eq!(t.border_top, 0xFFFF_FFFF);
        assert_eq!(t.border_bottom, 0xFF99_9999);
        assert_eq!((t.title, t.text, t.mod_name), (0xFFAA_BBCC, 0xFF01_0203, 0xFF44_5566));
        assert_eq!(t.warn, Theme::of(ThemeId::Dark).warn);
    }

    #[test]
    fn light_theme_text_is_readable() {
        let t = Theme::of(ThemeId::Light);
        let bg = over_black(t.background(0.94));
        // 浅色底按 94% 混黑后仍然很浅；文字必须明显更深。
        for (name, color) in [("title", t.title), ("mod_name", t.mod_name), ("text", t.text), ("warn", t.warn)] {
            assert!(contrast(color & 0xFF_FFFF, bg) >= 4.5, "{name}");
        }
    }

    #[test]
    fn fade_scales_only_alpha() {
        assert_eq!(fade(0xFF11_2233, 1.0), 0xFF11_2233);
        assert_eq!(fade(0xFF11_2233, 0.5), 0x8011_2233);
        assert_eq!(fade(0x8011_2233, 0.0), 0x0011_2233);
    }

    #[test]
    fn anchors_place_the_box() {
        let screen = [640.0, 360.0];
        let size = [100.0, 40.0];
        assert_eq!(place(Anchor::TopCenter, screen, size, [0, 0]), [270.0, 4.0]);
        assert_eq!(place(Anchor::TopLeft, screen, size, [0, 0]), [4.0, 4.0]);
        assert_eq!(place(Anchor::TopRight, screen, size, [0, 0]), [536.0, 4.0]);
        assert_eq!(place(Anchor::MiddleLeft, screen, size, [0, 0]), [4.0, 160.0]);
        assert_eq!(place(Anchor::MiddleRight, screen, size, [0, 0]), [536.0, 160.0]);
        assert_eq!(place(Anchor::TopCenter, screen, size, [-10, 20]), [260.0, 24.0]);
    }

    fn frames(anim: &mut Anim, target: impl Fn(u64) -> Option<[f32; 2]>, times: &[u64]) -> Vec<Option<Frame>> {
        times.iter().map(|t| anim.step(target(*t), *t, true)).collect()
    }

    #[test]
    fn fade_in_is_monotonic_and_converges() {
        let mut anim = Anim::default();
        let out = frames(&mut anim, |_| Some([100.0, 40.0]), &[0, 50, 100, 200]);
        let alphas: Vec<f32> = out.iter().map(|f| f.unwrap().alpha).collect();
        assert_eq!(alphas[0], 0.0);
        assert!(alphas.windows(2).all(|w| w[0] < w[1] || w[1] == 1.0), "{alphas:?}");
        assert_eq!(alphas[2], 1.0);
        assert_eq!(alphas[3], 1.0);
        // 刚出现时直接是目标尺寸，不从零长大。
        assert!(out.iter().all(|f| f.unwrap().w == 100.0 && !f.unwrap().morphing));
    }

    #[test]
    fn size_eases_toward_the_new_target() {
        let mut anim = Anim::default();
        frames(&mut anim, |_| Some([100.0, 40.0]), &[0, 100, 200]);
        let target = |_| Some([160.0, 70.0]);
        let out = frames(&mut anim, target, &[250, 300, 350, 600, 1000]);
        let widths: Vec<f32> = out.iter().map(|f| f.unwrap().w).collect();
        assert!(widths.windows(2).all(|w| w[0] <= w[1]), "{widths:?}");
        assert!(widths[0] > 100.0 && widths[0] < 160.0, "{widths:?}");
        assert!(out[0].unwrap().morphing);
        let last = out[4].unwrap();
        assert_eq!((last.w, last.h, last.morphing), (160.0, 70.0, false));
    }

    #[test]
    fn disabled_animation_jumps_to_the_target() {
        let mut anim = Anim::default();
        let f = anim.step(Some([100.0, 40.0]), 0, false).unwrap();
        assert_eq!((f.alpha, f.w, f.h, f.morphing), (1.0, 100.0, 40.0, false));
        let f = anim.step(Some([30.0, 20.0]), 1, false).unwrap();
        assert_eq!((f.alpha, f.w, f.h), (1.0, 30.0, 20.0));
        assert_eq!(anim.step(None, 2, false), None);
    }

    #[test]
    fn fade_out_finishes_before_returning_none() {
        let mut anim = Anim::default();
        frames(&mut anim, |_| Some([100.0, 40.0]), &[0, 100, 200]);
        let out = frames(&mut anim, |_| None, &[200, 250, 300, 349, 350, 400]);
        let alphas: Vec<f32> = out.iter().take(4).map(|f| f.unwrap().alpha).collect();
        assert!(alphas.windows(2).all(|w| w[0] > w[1]), "{alphas:?}");
        assert!(out[3].is_some());
        assert!(out[4].is_none() && out[5].is_none());
        // 淡出结束后再出现：从零重新淡入（距上一帧 50ms，淡入走一半），尺寸直接取目标。
        let f = anim.step(Some([50.0, 20.0]), 450, true).unwrap();
        assert_eq!((f.alpha, f.w), (0.5, 50.0));
    }

    #[test]
    fn long_frame_gaps_are_capped() {
        let mut anim = Anim::default();
        anim.step(Some([100.0, 40.0]), 0, true);
        // 卡了 10 秒（例如暂停菜单）：不会一帧内把动画跳完以外的状态弄乱。
        let f = anim.step(Some([100.0, 40.0]), 10_000, true).unwrap();
        assert_eq!(f.alpha, 1.0);
    }
}
