//! 客户端：每 tick 根据准星目标组装提示框内容，并按需向服务端请求数据。
//!
//! 对应 Jade 的 CorePlugin（ObjectName、ModName）以及纯客户端的 VanillaPlugin provider：
//! EntityHealthAndArmor、CropProgress、Redstone、NoteBlock、Beehive 蜂蜜等级、TNT 等。
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use mod_api::{BlockState, ClientHost, CrosshairTarget, Identifier, ItemKey};

use crate::net::{KIND_BLOCK, KIND_ENTITY, NetLine, NetTarget, Request, ServerData};

/// 提示框里的一行。
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// 普通文字与颜色。
    Line(String, u32),
    /// 生命值：当前、最大。
    Health(f32, f32),
    /// 一排物品图标与数量。
    Items(Vec<(ItemKey, i32)>),
    /// 输入物品 → 进度箭头 → 输出物品。
    Progress { inputs: Vec<(ItemKey, i32)>, outputs: Vec<(ItemKey, i32)>, fraction: f32 },
}

/// 当前要画的提示框。
#[derive(Clone, Debug, Default)]
pub struct Tooltip {
    pub icon: Option<ItemKey>,
    pub title: String,
    pub rows: Vec<Row>,
    pub mod_name: String,
}

#[derive(Default)]
struct State {
    target: Option<NetTarget>,
    /// 当前目标收到过的服务端数据。
    server: Option<ServerData>,
    ticks_since_request: u32,
    tooltip: Option<Tooltip>,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

/// 取锁；锁中毒（之前持锁时 panic）也继续用里面的数据，避免网络 handler 因此 panic 断线。
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 命名空间 → 模组显示名，注册时由 `lib.rs` 填入。
pub fn mod_names() -> &'static Mutex<HashMap<String, String>> {
    static NAMES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    NAMES.get_or_init(Default::default)
}

pub fn set_mod_name(namespace: &str, name: &str) {
    lock(mod_names()).entry(namespace.to_owned()).or_insert_with(|| name.to_owned());
}

/// 服务端数据到达。
pub fn receive(data: ServerData) {
    let mut st = lock(state());
    if st.target.as_ref() == Some(&data.target) {
        st.server = Some(data);
    }
}

/// 给渲染用的当前提示框。
pub fn current() -> Option<Tooltip> {
    lock(state()).tooltip.clone()
}

/// 同一目标每隔多少 tick 刷新一次服务端数据（进度条、容器内容需要更新）。
/// 换目标时不等这个间隔，立即请求。
pub const REQUEST_INTERVAL: u32 = 10;

/// 每个客户端 tick 调用：更新目标，返回需要发送的请求。
pub fn tick(host: &mut dyn ClientHost) -> Option<Request> {
    let target = host.crosshair_target();
    let net = target.as_ref().and_then(net_target);
    // 先在锁内更新目标、取出服务端数据，放锁后再读宿主组装提示框。
    let (server, request) = {
        let mut st = lock(state());
        let changed = st.target != net;
        if changed {
            st.target = net.clone();
            st.server = None;
        }
        let request = net.and_then(|target| {
            let (send, ticks) = schedule(changed, st.ticks_since_request);
            st.ticks_since_request = ticks;
            send.then_some(Request { target, fresh: changed })
        });
        (st.server.clone(), request)
    };
    let detail = shift_down(host);
    let tooltip = target.as_ref().and_then(|t| build(host, t, server.as_ref(), detail));
    lock(state()).tooltip = tooltip;
    request
}

fn shift_down(host: &dyn ClientHost) -> bool {
    host.is_key_down("key.keyboard.left.shift") || host.is_key_down("key.keyboard.right.shift")
}

/// 本 tick 是否发请求，以及更新后的计数：换目标立即发，否则每 [`REQUEST_INTERVAL`] tick 一次。
fn schedule(changed: bool, ticks_since_request: u32) -> (bool, u32) {
    if changed || ticks_since_request + 1 >= REQUEST_INTERVAL {
        (true, 0)
    } else {
        (false, ticks_since_request + 1)
    }
}

fn net_target(target: &CrosshairTarget) -> Option<NetTarget> {
    Some(match target {
        CrosshairTarget::Block { pos, .. } => {
            let [x, y, z] = block_pos(pos);
            NetTarget { kind: KIND_BLOCK, x, y, z, entity: 0 }
        }
        CrosshairTarget::Entity { entity, .. } => {
            NetTarget { kind: KIND_ENTITY, entity: entity.id() as i32, ..Default::default() }
        }
        _ => return None,
    })
}

fn block_pos(pos: &mod_api::BlockPos) -> [i32; 3] {
    [pos.x, pos.y, pos.z]
}

const GRAY: u32 = 0xFFAA_AAAA;
const RED: u32 = 0xFFFF_5555;

/// 蜂巢最多 5 级蜂蜜。
const HONEY_MAX: u32 = 5;

/// “3 ticks”/“1 tick”，中文两处都是“3刻”。
fn ticks_text(host: &dyn ClientHost, ticks: u32) -> String {
    let key = if ticks == 1 { "jade.horizon.tick" } else { "jade.horizon.ticks" };
    tr(host, key, &[ticks.to_string()])
}

/// 精简模式下隐藏的服务端行键；按住 Shift 才显示。
const DETAIL_KEYS: &[&str] = &[
    "jade.mobgrowth.time",
    "jade.mobbreeding.time",
    "jade.mobbreeding.fed",
    "jade.nextEgg",
];

const COMPACT_ITEM_ROWS: usize = 1;
const DETAIL_ITEM_ROWS: usize = 6;

fn build(host: &dyn ClientHost, target: &CrosshairTarget, server: Option<&ServerData>, detail: bool) -> Option<Tooltip> {
    let mut tip = Tooltip::default();
    let mut rows: Vec<(Row, Option<&str>)> = Vec::new();
    let namespace;
    match target {
        CrosshairTarget::Block { state, .. } => {
            let block = state.block();
            let id = block.id(host);
            if id == "minecraft:air" {
                return None;
            }
            namespace = id.split(':').next().unwrap_or("minecraft").to_owned();
            tip.icon = host.block_item(block).filter(|item| item.id(host) != "minecraft:air");
            tip.title = block_name(host, &id);
            let props = properties(host, *state);
            let mut plain = Vec::new();
            block_rows(host, &id, &props, &mut plain);
            rows.extend(plain.into_iter().map(|r| (r, None)));
        }
        CrosshairTarget::Entity { entity, .. } => {
            let kind = entity.kind(host).map(|k| k.id(host)).unwrap_or_default();
            namespace = kind.split(':').next().unwrap_or("minecraft").to_owned();
            tip.title = entity.name(host).unwrap_or_else(|| translated(host, &key_of("entity", &kind)));
            if let Some(living) = host.living(*entity) {
                if let (Some(hp), Some(max)) = (living.health(host), living.max_health(host)) {
                    rows.push((Row::Health(hp, max), None));
                }
            }
        }
        _ => return None,
    }
    if let Some(data) = server {
        server_rows(host, data, &mut rows);
    }
    let (mut visible, hidden) = visible_rows(rows, detail);
    if hidden {
        visible.push(Row::Line(tr(host, "jade.horizon.shift_for_more", &[]), GRAY));
    }
    tip.rows = visible;
    tip.mod_name = mod_name(host, &namespace);
    Some(tip)
}

/// 按精简/详细模式过滤行，返回 `(显示的行, 是否有内容被精简模式隐藏)`。
fn visible_rows(rows: Vec<(Row, Option<&str>)>, detail: bool) -> (Vec<Row>, bool) {
    let item_limit = if detail { DETAIL_ITEM_ROWS } else { COMPACT_ITEM_ROWS };
    let mut item_rows = 0;
    let mut out = Vec::with_capacity(rows.len());
    let mut hidden = false;
    for (row, key) in rows {
        if matches!(row, Row::Items(_)) {
            item_rows += 1;
            if item_rows > item_limit {
                if !detail && item_rows <= DETAIL_ITEM_ROWS {
                    hidden = true;
                }
                continue;
            }
        }
        if !detail && key.is_some_and(|k| DETAIL_KEYS.contains(&k)) {
            hidden = true;
            continue;
        }
        out.push(row);
    }
    (out, hidden)
}

/// `minecraft:wheat[age=7]` 的属性表。
fn properties(host: &dyn ClientHost, state: BlockState) -> HashMap<String, String> {
    let text = host.block_state_string(state).unwrap_or_default();
    let mut out = HashMap::new();
    if let Some(start) = text.find('[') {
        for pair in text[start + 1..text.len().saturating_sub(1)].split(',') {
            if let Some((k, v)) = pair.split_once('=') {
                out.insert(k.to_owned(), v.to_owned());
            }
        }
    }
    out
}

fn block_rows(host: &dyn ClientHost, id: &str, props: &HashMap<String, String>, rows: &mut Vec<Row>) {
    let prop = |k: &str| props.get(k).map(String::as_str);
    let path = id.split(':').nth(1).unwrap_or(id);
    // 作物生长（CropProgressProvider）。
    if let Some(age) = prop("age").and_then(|v| v.parse::<u32>().ok()) {
        if let Some(max) = crop_max_age(path) {
            let text = if age >= max {
                tr(host, "tooltip.jade.crop_mature", &[])
            } else {
                let pct = format!("{:.0}%", age as f32 * 100.0 / max as f32);
                tr(host, "tooltip.jade.crop_growth", &[pct])
            };
            rows.push(Row::Line(text, GRAY));
        }
    }
    // 红石（RedstoneProvider）。
    match path {
        "repeater" => {
            if let Some(d) = prop("delay") {
                let ticks = d.parse::<u32>().unwrap_or(1);
                let text = ticks_text(host, ticks);
                rows.push(Row::Line(tr(host, "tooltip.jade.delay", &[text]), GRAY));
            }
        }
        "comparator" => {
            if let Some(m) = prop("mode") {
                let key = if m == "subtract" { "tooltip.jade.mode_subtractor" } else { "tooltip.jade.mode_comparator" };
                let mode = tr(host, key, &[]);
                rows.push(Row::Line(tr(host, "tooltip.jade.mode", &[mode]), GRAY));
            }
        }
        "lever" => {
            if let Some(p) = prop("powered") {
                let on = tr(host, if p == "true" { "tooltip.jade.state_on" } else { "tooltip.jade.state_off" }, &[]);
                rows.push(Row::Line(tr(host, "tooltip.jade.state", &[on]), GRAY));
            }
        }
        "beehive" | "bee_nest" => {
            if let Some(h) = prop("honey_level") {
                let amount = tr(host, "jade.fraction", &[h.to_owned(), HONEY_MAX.to_string()]);
                rows.push(Row::Line(tr(host, "jade.beehive.honey", &[amount]), GRAY));
            }
        }
        "note_block" => {
            if let (Some(note), Some(inst)) = (prop("note"), prop("instrument")) {
                let inst = tr(host, &format!("jade.instrument.{inst}"), &[]);
                let note = note_name(note.parse().unwrap_or(0)).to_owned();
                let text = tr(host, "jade.horizon.note_block", &[inst, note]);
                rows.push(Row::Line(text, GRAY));
            }
        }
        "tnt" => {
            if prop("unstable") == Some("true") {
                rows.push(Row::Line(tr(host, "jade.tnt.unstable", &[]), RED));
            }
        }
        _ => {}
    }
    if let Some(power) = prop("power") {
        if power != "0" {
            rows.push(Row::Line(tr(host, "tooltip.jade.power", &[power.to_owned()]), GRAY));
        }
    }
}

fn crop_max_age(path: &str) -> Option<u32> {
    Some(match path {
        "wheat" | "carrots" | "potatoes" | "pumpkin_stem" | "melon_stem" => 7,
        "beetroots" | "nether_wart" | "sweet_berry_bush" => 3,
        "cocoa" => 2,
        "torchflower_crop" => 1,
        "pitcher_crop" => 4,
        _ => return None,
    })
}

fn note_name(note: u32) -> &'static str {
    const NAMES: [&str; 12] = ["F♯", "G", "G♯", "A", "A♯", "B", "C", "C♯", "D", "D♯", "E", "F"];
    NAMES[(note % 12) as usize]
}

fn server_rows<'a>(host: &dyn ClientHost, data: &'a ServerData, rows: &mut Vec<(Row, Option<&'a str>)>) {
    // 客户端读不到生物生命值时（Horizon 客户端未同步部分生物），用服务端的值，放在标题下第一行。
    if data.max_health > 0.0 && !rows.iter().any(|(r, _)| matches!(r, Row::Health(..))) {
        rows.insert(0, (Row::Health(data.health, data.max_health), None));
    }
    for p in &data.progress {
        let items = |v: &[crate::net::NetItem]| v.iter().filter_map(|i| Some((item_key(host, &i.id)?, i.count))).collect();
        let fraction = if p.total > 0 { (p.current as f32 / p.total as f32).clamp(0.0, 1.0) } else { 0.0 };
        rows.push((Row::Progress { inputs: items(&p.inputs), outputs: items(&p.outputs), fraction }, None));
    }
    if !data.items.is_empty() {
        let items: Vec<_> = data.items.iter().filter_map(|i| Some((item_key(host, &i.id)?, i.count))).collect();
        // 每行最多 9 个，与 Jade 默认的容器显示一致；显示几行由 visible_rows 决定。
        for chunk in items.chunks(9) {
            rows.push((Row::Items(chunk.to_vec()), None));
        }
    }
    for line in &data.lines {
        rows.push((Row::Line(net_line(host, line), GRAY), Some(line.key.as_str())));
    }
}

fn item_key(host: &dyn ClientHost, id: &str) -> Option<ItemKey> {
    host.item_key(&Identifier::parse_namespaced(id)?)
}

fn net_line(host: &dyn ClientHost, line: &NetLine) -> String {
    let args: Vec<String> = line.args.iter().map(|a| arg(host, a)).collect();
    tr(host, &line.key, &args)
}

fn arg(host: &dyn ClientHost, a: &str) -> String {
    if let Some(key) = a.strip_prefix('@') {
        translated(host, key)
    } else if let Some(t) = a.strip_prefix("#ticks:") {
        duration(host, t.parse().unwrap_or(0))
    } else {
        a.to_owned()
    }
}

/// Jade 的时间格式：`jade.seconds` / `jade.minutes_seconds` / `jade.hours_minutes`。
fn duration(host: &dyn ClientHost, ticks: i32) -> String {
    let secs = ticks.max(0) / 20;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let (key, a, b) = if h > 0 {
        ("jade.hours_minutes", h, m)
    } else if m > 0 {
        if s > 0 { ("jade.minutes_seconds", m, s) } else { ("jade.minutes", m, 0) }
    } else {
        ("jade.seconds", s, 0)
    };
    translated(host, key).replace("{0}", &a.to_string()).replace("{1}", &b.to_string())
}

fn block_name(host: &dyn ClientHost, id: &str) -> String {
    let key = key_of("block", id);
    match host.translate(&key) {
        Some(name) => name,
        None => host.translate(&key_of("item", id)).unwrap_or_else(|| id.to_owned()),
    }
}

fn key_of(kind: &str, id: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("{kind}.{ns}.{}", path.replace('/', "."))
}

fn mod_name(host: &dyn ClientHost, namespace: &str) -> String {
    if namespace == "minecraft" {
        return "Minecraft".to_owned();
    }
    if let Some(name) = lock(mod_names()).get(namespace) {
        return name.clone();
    }
    let _ = host;
    // 找不到时把 `my_mod` 变成 `My Mod`。
    namespace
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn translated(host: &dyn ClientHost, key: &str) -> String {
    host.translate(key).unwrap_or_else(|| key.to_owned())
}

/// 翻译并依次替换 `%s`（也支持 `%1$s` 形式）。
pub fn tr(host: &dyn ClientHost, key: &str, args: &[String]) -> String {
    format_args_java(&translated(host, key), args)
}

pub fn format_args_java(pattern: &str, args: &[String]) -> String {
    let mut out = String::new();
    let mut next = 0;
    let mut rest = pattern;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        if let Some(t) = tail.strip_prefix('s') {
            out.push_str(args.get(next).map(String::as_str).unwrap_or(""));
            next += 1;
            rest = t;
        } else if let Some(t) = tail.strip_prefix('%') {
            out.push('%');
            rest = t;
        } else if let Some((n, t)) = tail.split_once("$s") {
            match n.parse::<usize>() {
                Ok(n) => {
                    out.push_str(args.get(n.wrapping_sub(1)).map(String::as_str).unwrap_or(""));
                    rest = t;
                }
                Err(_) => {
                    out.push('%');
                    rest = tail;
                }
            }
        } else {
            out.push('%');
            rest = tail;
        }
    }
    out.push_str(rest);
    // 去掉 Minecraft 的 § 格式码，DrawContext::text 画的是纯文本。
    strip_formatting(&out)
}

fn strip_formatting(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_on_change_then_every_interval() {
        // 换目标立即发；之后同一目标每 REQUEST_INTERVAL tick 发一次。
        let (mut sent, mut ticks) = (vec![], 0);
        for t in 0..30u32 {
            let (send, n) = schedule(t == 0, ticks);
            ticks = n;
            if send {
                sent.push(t);
            }
        }
        assert_eq!(sent, vec![0, 10, 20]);
        // 中途换目标立即发并重新计时。
        assert_eq!(schedule(true, 3), (true, 0));
    }

    #[test]
    fn formats_java_style() {
        let args = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(format_args_java("%s / %s", &args), "a / b");
        assert_eq!(format_args_java("%2$s-%1$s", &args), "b-a");
        assert_eq!(format_args_java("100%%", &[]), "100%");
        assert_eq!(format_args_java("§oWhat?", &[]), "What?");
    }

    #[test]
    fn compact_limits_item_rows_to_one() {
        let one = vec![(Row::Items(vec![]), None)];
        assert_eq!(visible_rows(one.clone(), false), (vec![Row::Items(vec![])], false));
        assert_eq!(visible_rows(one, true), (vec![Row::Items(vec![])], false));

        let seven: Vec<_> = (0..7).map(|_| (Row::Items(vec![]), None)).collect();
        let (compact, hidden) = visible_rows(seven.clone(), false);
        assert_eq!(compact.len(), 1);
        assert!(hidden);

        let (detail, hidden) = visible_rows(seven, true);
        assert_eq!(detail.len(), 6);
        assert!(!hidden);
    }

    #[test]
    fn compact_hides_detail_keys_only() {
        let rows = vec![
            (Row::Health(10.0, 10.0), None),
            (Row::Line("Bees: 3".into(), GRAY), Some("jade.beehive.bees")),
            (Row::Line("Growing time: 10m".into(), GRAY), Some("jade.mobgrowth.time")),
            (Row::Line("Breeding CD: 2m".into(), GRAY), Some("jade.mobbreeding.time")),
            (Row::Line("Breeding CD: In Love".into(), GRAY), Some("jade.mobbreeding.fed")),
            (Row::Line("Next egg: 5m".into(), GRAY), Some("jade.nextEgg")),
        ];
        let (compact, hidden) = visible_rows(rows.clone(), false);
        assert_eq!(
            compact,
            vec![Row::Health(10.0, 10.0), Row::Line("Bees: 3".into(), GRAY)]
        );
        assert!(hidden);

        let (detail, hidden) = visible_rows(rows, true);
        assert_eq!(detail.len(), 6);
        assert!(!hidden);
    }
}
