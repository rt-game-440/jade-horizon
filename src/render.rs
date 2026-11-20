//! 把 [`Tooltip`] 画到 HUD 顶部正中，外观仿 Jade 默认（原版 tooltip 风格）。
use std::sync::atomic::{AtomicUsize, Ordering};

use mod_api::DrawContext;

use crate::client::{Row, Tooltip};

const PAD: f32 = 4.0;
const LINE: f32 = 10.0;
const SLOT: f32 = 18.0;
const ICON_W: f32 = 20.0;

const TITLE: u32 = 0xFFFF_FFFF;
const MOD_NAME: u32 = 0xFF55_55FF;
const BG: u32 = 0xF010_0010;
const BORDER_TOP: u32 = 0x5050_00FF;
const BORDER_BOTTOM: u32 = 0x5028_007F;

fn row_height(row: &Row) -> f32 {
    match row {
        Row::Line(..) | Row::Health(..) => LINE,
        Row::Items(_) | Row::Progress { .. } => SLOT,
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
        Row::Items(items) => items.len() as f32 * SLOT,
        Row::Progress { inputs, outputs, .. } => (inputs.len() + outputs.len()) as f32 * SLOT + 26.0,
    }
}

fn health_text(hp: f32, max: f32) -> String {
    format!("❤ {:.1} / {:.1}", hp, max)
}

pub fn draw(draw: &mut dyn DrawContext, tip: &Tooltip) {
    let [screen_w, _] = draw.size();
    let icon_w = if tip.icon.is_some() { ICON_W } else { 0.0 };
    let mut width = draw.text_width(&tip.title).max(draw.text_width(&tip.mod_name));
    let mut height = LINE * 2.0;
    for row in &tip.rows {
        width = width.max(row_width(draw, row));
        height += row_height(row);
    }
    let content_h = height.max(if tip.icon.is_some() { 16.0 } else { 0.0 });
    let w = (width + icon_w + PAD * 2.0).ceil();
    let h = (content_h + PAD * 2.0 - 2.0).ceil();
    let x = ((screen_w - w) / 2.0).floor();
    let y = 4.0;
    background(draw, [x, y, w, h]);

    if let Some(icon) = tip.icon {
        let iy = y + (h - 16.0) / 2.0;
        draw.item(icon, [x + PAD, iy.floor()], 16.0);
    }
    let tx = x + PAD + icon_w;
    let mut ty = y + PAD;
    draw.text(&tip.title, [tx, ty], TITLE, true);
    ty += LINE;
    for row in &tip.rows {
        draw_row(draw, row, tx, ty);
        ty += row_height(row);
    }
    draw.text(&tip.mod_name, [tx, ty], MOD_NAME, true);
}

/// 原版 tooltip 的背景：深色底加紫色渐变边框。
fn background(draw: &mut dyn DrawContext, [x, y, w, h]: [f32; 4]) {
    draw.fill([x - 1.0, y, w + 2.0, h], BG);
    draw.fill([x, y - 1.0, w, 1.0], BG);
    draw.fill([x, y + h, w, 1.0], BG);
    draw.fill_gradient([x, y + 1.0, 1.0, h - 2.0], BORDER_TOP, BORDER_BOTTOM);
    draw.fill_gradient([x + w - 1.0, y + 1.0, 1.0, h - 2.0], BORDER_TOP, BORDER_BOTTOM);
    draw.fill([x, y, w, 1.0], BORDER_TOP);
    draw.fill([x, y + h - 1.0, w, 1.0], BORDER_BOTTOM);
}

fn draw_row(draw: &mut dyn DrawContext, row: &Row, x: f32, y: f32) {
    match row {
        Row::Line(text, color) => draw.text(text, [x, y], *color, true),
        Row::Health(hp, max) => health(draw, *hp, *max, x, y),
        Row::Items(items) => {
            for (i, (item, count)) in items.iter().enumerate() {
                slot(draw, *item, *count, x + i as f32 * SLOT, y);
            }
        }
        Row::Progress { inputs, outputs, fraction } => {
            let mut cx = x;
            for (item, count) in inputs {
                slot(draw, *item, *count, cx, y);
                cx += SLOT;
            }
            arrow(draw, cx + 2.0, y + 4.0, *fraction);
            cx += 26.0;
            for (item, count) in outputs {
                slot(draw, *item, *count, cx, y);
                cx += SLOT;
            }
        }
    }
}

fn slot(draw: &mut dyn DrawContext, item: mod_api::ItemKey, count: i32, x: f32, y: f32) {
    draw.item(item, [x + 1.0, y + 1.0], 16.0);
    if count > 1 {
        let text = short_count(count);
        let tw = draw.text_width(&text);
        draw.text(&text, [x + 17.0 - tw, y + 9.0], 0xFFFF_FFFF, true);
    }
}

fn short_count(count: i32) -> String {
    if count >= 10_000 { format!("{}k", count / 1000) } else { count.to_string() }
}

/// 进度箭头：逐列填色画出 22×11 的箭头，已完成部分白色、其余灰色。
/// （原版熔炉箭头贴图在 HUD 上会带出容器底色，故自绘。）
fn arrow(draw: &mut dyn DrawContext, x: f32, y: f32, fraction: f32) {
    const W: i32 = 22;
    const HEAD: i32 = 6; // 箭头头部宽度
    let mid = y + 4.5; // 箭杆竖直中心，使各列上下沿落在整像素
    let done = (W as f32 * fraction.clamp(0.0, 1.0)).round() as i32;
    for c in 0..W {
        let half = if c < W - HEAD { 1.5 } else { (W - c) as f32 - 0.5 };
        let color = if c < done { 0xFFFF_FFFF } else { 0xFF55_5555 };
        draw.fill([x + c as f32, mid - half, 1.0, half * 2.0], color);
    }
}

fn health(draw: &mut dyn DrawContext, hp: f32, max: f32, x: f32, y: f32) {
    if max > 20.0 {
        draw.text(&health_text(hp, max), [x, y], 0xFFFF_5555, true);
        return;
    }
    let hearts = (max / 2.0).ceil() as i32;
    for i in 0..hearts.min(10) {
        let hx = x + i as f32 * 8.0;
        let dst = [hx, y, 9.0, 9.0];
        if !gui_sprite(draw, "hud/heart/container", [0.0, 0.0, 9.0, 9.0], dst, None) {
            // 没有心形贴图时退回文字。
            draw.text(&health_text(hp, max), [x, y], 0xFFFF_5555, true);
            return;
        }
        let filled = hp - i as f32 * 2.0;
        if filled >= 2.0 {
            gui_sprite(draw, "hud/heart/full", [0.0, 0.0, 9.0, 9.0], dst, None);
        } else if filled > 0.0 {
            gui_sprite(draw, "hud/heart/half", [0.0, 0.0, 9.0, 9.0], dst, None);
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
