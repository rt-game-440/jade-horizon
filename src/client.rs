//! 客户端：每 tick 根据准星目标组装提示框内容，并按需向服务端请求数据。
//!
//! 对应 Jade 的 CorePlugin（ObjectName、ModName）以及纯客户端的 VanillaPlugin provider：
//! EntityHealthAndArmor、CropProgress、Redstone、NoteBlock、Beehive 蜂蜜等级、TNT 等。
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use mod_api::{BlockState, ClientHost, CrosshairTarget, Identifier, ItemKey};

use crate::config::{Config, Feature};
use crate::net::{KIND_BLOCK, KIND_ENTITY, NetLine, NetTarget, Request, ServerData};

/// 文字的语气：具体颜色由主题决定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Warn,
}

/// 提示框里的一行。
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// 一行文字；颜色由主题按语气决定。
    Line(String, Tone),
    /// 生命值：当前、最大。
    Health(f32, f32),
    /// 盔甲值：每 2 点一个图标。
    Armor(f32),
    /// 一排物品图标与数量。
    Items(Vec<(ItemKey, i32)>),
    /// 输入物品 → 进度箭头 → 输出物品。
    Progress { inputs: Vec<(ItemKey, i32)>, outputs: Vec<(ItemKey, i32)>, fraction: f32 },
    /// 需要的工具与能不能采集：工具图标 + ✔/✘。
    Harvest { tool: Option<ItemKey>, ok: bool },
}

/// 组装中的一行：内容、精简模式下是否隐藏（服务端行键）、属于哪个分项开关。
#[derive(Clone, Debug, PartialEq)]
struct Entry<'a> {
    row: Row,
    detail: Option<&'a str>,
    feature: Feature,
}

fn entry(row: Row, feature: Feature) -> Entry<'static> {
    Entry { row, detail: None, feature }
}

/// 当前要画的提示框。
#[derive(Clone, Debug, Default)]
pub struct Tooltip {
    pub icon: Option<ItemKey>,
    pub title: String,
    pub rows: Vec<Row>,
    pub mod_name: String,
    /// 正在挖这个方块时的破坏进度 0..1，画成提示框底部的一条进度条。
    pub breaking: Option<f32>,
}

#[derive(Default)]
struct State {
    /// 客户端 tick 计数，用来判断挖掘记录是不是最新的。
    now: u32,
    /// 最近一次挖掘预测。
    mining: Mining,
    target: Option<NetTarget>,
    /// 当前目标收到过的服务端数据。
    server: Option<ServerData>,
    ticks_since_request: u32,
    tooltip: Option<Tooltip>,
    /// 玩家用按键临时隐藏了提示框；不写回设置，重启后恢复显示。
    overlay_hidden: bool,
    /// 玩家用按键临时关掉了流体显示。
    liquid_hidden: bool,
}

/// 按键 id（`lib.rs` 里注册）。
pub const KEY_SHOW_OVERLAY: &str = "show_overlay";
pub const KEY_TOGGLE_LIQUID: &str = "toggle_liquid";

/// 本 tick 内这个按键被按了几次。
fn clicks(host: &mut dyn ClientHost, id: &str) -> u32 {
    let mut n = 0;
    // 设上限：宿主一直返回 true 也不会死循环。
    while n < 16 && host.consume_click("jade", id) {
        n += 1;
    }
    n
}

/// 按了 `clicks` 次之后开关的状态。
fn toggled(state: bool, clicks: u32) -> bool {
    state ^ (clicks % 2 == 1)
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
    crate::stats::record_reply(data.wire_size());
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

/// 最近一次客户端挖掘预测：正在挖哪个方块、进度多少、记在第几 tick。
#[derive(Clone, Copy, Debug, Default)]
struct Mining {
    pos: Option<[i32; 3]>,
    progress: f32,
    /// 记录这次预测时的客户端 tick（`State.now`）。
    recorded_at: u32,
}

/// 挖掘预测只在这个 tick 数内算“正在挖”；松手后残留的记录会被丢掉。
const MINING_STALE_TICKS: u32 = 2;

/// 客户端每 tick 的挖掘预测（`client_mining_predict` 钩子调用）。
pub fn record_mining(prediction: &mod_api::ClientMiningPrediction) {
    let mut st = lock(state());
    st.mining = Mining { pos: Some(prediction.pos), progress: prediction.progress, recorded_at: st.now };
}

/// 每个客户端 tick 调用：更新目标，返回需要发送的请求。
pub fn tick(host: &mut dyn ClientHost) -> Option<Request> {
    let cfg = crate::config::refresh(host);
    // 按键每 tick 都要读（否则按下次数会攒到隐藏结束后一起生效）。
    let overlay_clicks = clicks(host, KEY_SHOW_OVERLAY);
    let liquid_clicks = clicks(host, KEY_TOGGLE_LIQUID);
    let (hidden, liquid_off) = {
        let mut st = lock(state());
        st.overlay_hidden = toggled(st.overlay_hidden, overlay_clicks);
        st.liquid_hidden = toggled(st.liquid_hidden, liquid_clicks);
        (st.overlay_hidden, st.liquid_hidden)
    };
    // 隐藏（或设置里关掉）时什么也不做：不组装提示框，也不向服务端发请求。
    if hidden || !cfg.enabled {
        let mut st = lock(state());
        st.tooltip = None;
        // 忘掉旧目标和旧数据：重新显示时当作新目标，立即向服务端要新数据。
        st.target = None;
        st.server = None;
        return None;
    }
    // 掉落物原版瞄不到，自己做射线；它比准星命中的东西近时，当作准星命中的实体。
    let dropped = if cfg.has(Feature::Dropped) { crate::dropped::target(host) } else { None };
    let fluid = if dropped.is_some() || !cfg.has(Feature::Fluid) || liquid_off { None } else { fluid_target(host) };
    // 显示流体时不向服务端要数据（流体信息全在客户端算）。
    let target = if fluid.is_some() { None } else { dropped.or_else(|| host.crosshair_target()) };
    let net = target.as_ref().and_then(net_target);
    // 先在锁内更新目标、取出服务端数据，放锁后再读宿主组装提示框。
    let (server, request, mining, tick) = {
        let mut st = lock(state());
        st.now += 1;
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
        (st.server.clone(), request, st.mining, st.now)
    };
    let detail = shift_down(host);
    let mut tooltip = match &fluid {
        Some(fluid) => Some(fluid_tooltip(host, fluid)),
        None => target.as_ref().and_then(|t| build(host, t, server.as_ref(), detail, &cfg)),
    };
    if let Some(tip) = tooltip.as_mut() {
        if cfg.has(Feature::Mining) {
            tip.breaking = breaking(target_pos(&target), &mining, tick);
        }
        if !cfg.has(Feature::ModName) {
            tip.mod_name.clear();
        }
    }
    lock(state()).tooltip = tooltip;
    request
}

/// 准星命中的方块坐标；对准实体或什么也没有时为 None。
fn target_pos(target: &Option<CrosshairTarget>) -> Option<[i32; 3]> {
    match target {
        Some(CrosshairTarget::Block { pos, .. }) => Some(block_pos(pos)),
        _ => None,
    }
}

/// 正在挖准星对着的方块时给出破坏进度；换目标或松手后（记录过期）就不给了。
fn breaking(target: Option<[i32; 3]>, mining: &Mining, now: u32) -> Option<f32> {
    if target? != mining.pos? {
        return None;
    }
    if now.saturating_sub(mining.recorded_at) > MINING_STALE_TICKS {
        return None;
    }
    (mining.progress > 0.0 && mining.progress < 1.0).then_some(mining.progress)
}

/// 视线上的流体；准星命中的实体比流体近时不显示流体。
fn fluid_target(host: &dyn ClientHost) -> Option<crate::fluid::Fluid> {
    let fluid = crate::fluid::target(host)?;
    let closer_entity = match host.crosshair_target() {
        Some(CrosshairTarget::Entity { point, .. }) => {
            crate::ray::eye(host).is_some_and(|eye| crate::ray::distance(eye, point) <= fluid.distance)
        }
        // 方块命中格本身不参与流体判定，能看到的流体一定在它之前。
        _ => false,
    };
    (!closer_entity).then_some(fluid)
}

fn fluid_tooltip(host: &dyn ClientHost, fluid: &crate::fluid::Fluid) -> Tooltip {
    let namespace = fluid.id.split(':').next().unwrap_or("minecraft").to_owned();
    Tooltip {
        icon: host.fluid_bucket(&fluid.id),
        title: block_name(host, &fluid.id),
        rows: Vec::new(),
        mod_name: mod_name(host, &namespace),
        breaking: None,
    }
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
    "jade.owner",
];

const COMPACT_ITEM_ROWS: usize = 1;
const DETAIL_ITEM_ROWS: usize = 6;

fn build(host: &dyn ClientHost, target: &CrosshairTarget, server: Option<&ServerData>, detail: bool, cfg: &Config) -> Option<Tooltip> {
    let mut tip = Tooltip::default();
    let mut rows: Vec<Entry> = Vec::new();
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
            if let Some(harvest) = crate::harvest::of(host, *state) {
                rows.push(entry(Row::Harvest { tool: harvest.tool, ok: harvest.ok }, Feature::Harvest));
            }
            block_rows(host, &id, &props, &mut rows);
        }
        CrosshairTarget::Entity { entity, .. } => {
            let kind = entity.kind(host).map(|k| k.id(host)).unwrap_or_default();
            namespace = kind.split(':').next().unwrap_or("minecraft").to_owned();
            tip.title = entity.name(host).unwrap_or_else(|| translated(host, &key_of("entity", &kind)));
            if let Some(living) = host.living(*entity) {
                if let (Some(hp), Some(max)) = (living.health(host), living.max_health(host)) {
                    rows.push(entry(Row::Health(hp, max), Feature::Health));
                }
            }
        }
        _ => return None,
    }
    if let Some(data) = server {
        server_rows(host, data, &mut rows);
    }
    let (mut visible, hidden, truncated) = visible_rows(rows, detail, cfg);
    if hidden {
        visible.push(Row::Line(tr(host, "jade.horizon.shift_for_more", &[]), Tone::Normal));
    }
    if truncated {
        visible.push(Row::Line(tr(host, "jade.horizon.more_items", &[]), Tone::Normal));
    }
    tip.rows = visible;
    tip.mod_name = mod_name(host, &namespace);
    Some(tip)
}

/// 按分项开关与精简/详细模式过滤行，返回 `(显示的行, 精简模式隐藏了内容, 详细模式也放不下)`。
fn visible_rows(rows: Vec<Entry>, detail: bool, cfg: &Config) -> (Vec<Row>, bool, bool) {
    let item_limit = if detail { DETAIL_ITEM_ROWS } else { COMPACT_ITEM_ROWS };
    let mut item_rows = 0;
    let mut out = Vec::with_capacity(rows.len());
    let mut hidden = false;
    let mut truncated = false;
    for Entry { row, detail: key, feature } in rows {
        if !cfg.has(feature) {
            continue;
        }
        if matches!(row, Row::Items(_)) {
            item_rows += 1;
            if item_rows > item_limit {
                if !detail {
                    hidden = true;
                }
                if item_rows > DETAIL_ITEM_ROWS {
                    truncated = true;
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
    (out, hidden, truncated)
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

fn block_rows(host: &dyn ClientHost, id: &str, props: &HashMap<String, String>, rows: &mut Vec<Entry>) {
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
            rows.push(entry(Row::Line(text, Tone::Normal), Feature::Crop));
        }
    }
    // 红石（RedstoneProvider）。
    match path {
        "repeater" => {
            if let Some(d) = prop("delay") {
                let ticks = d.parse::<u32>().unwrap_or(1);
                let text = ticks_text(host, ticks);
                rows.push(entry(Row::Line(tr(host, "tooltip.jade.delay", &[text]), Tone::Normal), Feature::Redstone));
            }
        }
        "comparator" => {
            if let Some(m) = prop("mode") {
                let key = if m == "subtract" { "tooltip.jade.mode_subtractor" } else { "tooltip.jade.mode_comparator" };
                let mode = tr(host, key, &[]);
                rows.push(entry(Row::Line(tr(host, "tooltip.jade.mode", &[mode]), Tone::Normal), Feature::Redstone));
            }
        }
        "lever" => {
            if let Some(p) = prop("powered") {
                let on = tr(host, if p == "true" { "tooltip.jade.state_on" } else { "tooltip.jade.state_off" }, &[]);
                rows.push(entry(Row::Line(tr(host, "tooltip.jade.state", &[on]), Tone::Normal), Feature::Redstone));
            }
        }
        "beehive" | "bee_nest" => {
            if let Some(h) = prop("honey_level") {
                let amount = tr(host, "jade.fraction", &[h.to_owned(), HONEY_MAX.to_string()]);
                rows.push(entry(Row::Line(tr(host, "jade.beehive.honey", &[amount]), Tone::Normal), Feature::Always));
            }
        }
        "note_block" => {
            if let (Some(note), Some(inst)) = (prop("note"), prop("instrument")) {
                let inst = tr(host, &format!("jade.instrument.{inst}"), &[]);
                let note = note_name(note.parse().unwrap_or(0)).to_owned();
                let text = tr(host, "jade.horizon.note_block", &[inst, note]);
                rows.push(entry(Row::Line(text, Tone::Normal), Feature::Always));
            }
        }
        "tnt" => {
            if prop("unstable") == Some("true") {
                rows.push(entry(Row::Line(tr(host, "jade.tnt.unstable", &[]), Tone::Warn), Feature::Always));
            }
        }
        _ => {}
    }
    if let Some(power) = prop("power") {
        if power != "0" {
            rows.push(entry(Row::Line(tr(host, "tooltip.jade.power", &[power.to_owned()]), Tone::Normal), Feature::Redstone));
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

fn server_rows<'a>(host: &dyn ClientHost, data: &'a ServerData, rows: &mut Vec<Entry<'a>>) {
    // 客户端读不到生物生命值时（Horizon 客户端未同步部分生物），用服务端的值，放在标题下第一行。
    if data.max_health > 0.0 && !rows.iter().any(|e| matches!(e.row, Row::Health(..))) {
        rows.insert(0, entry(Row::Health(data.health, data.max_health), Feature::Health));
    }
    if data.armor > 0.0 {
        let at = rows.iter().position(|e| matches!(e.row, Row::Health(..))).map_or(0, |i| i + 1);
        rows.insert(at, entry(Row::Armor(data.armor), Feature::Armor));
    }
    for p in &data.progress {
        let items = |v: &[crate::net::NetItem]| v.iter().filter_map(|i| Some((item_key(host, &i.id)?, i.count))).collect();
        let fraction = if p.total > 0 { (p.current as f32 / p.total as f32).clamp(0.0, 1.0) } else { 0.0 };
        rows.push(entry(Row::Progress { inputs: items(&p.inputs), outputs: items(&p.outputs), fraction }, Feature::Furnace));
    }
    if !data.items.is_empty() {
        let items: Vec<_> = data.items.iter().filter_map(|i| Some((item_key(host, &i.id)?, i.count))).collect();
        // 每行最多 9 个，与 Jade 默认的容器显示一致；显示几行由 visible_rows 决定。
        for chunk in items.chunks(9) {
            rows.push(entry(Row::Items(chunk.to_vec()), Feature::Container));
        }
    }
    for line in &data.lines {
        rows.push(Entry { row: Row::Line(net_line(host, line), Tone::Normal), detail: Some(line.key.as_str()), feature: line_feature(&line.key) });
    }
}

/// 服务端行属于哪个分项开关；没有开关的归 `Always`。
fn line_feature(key: &str) -> Feature {
    match key {
        "jade.potion" => Feature::Potion,
        "jade.owner" => Feature::Owner,
        "tooltip.jade.villager_profession" => Feature::Villager,
        k if k.starts_with("jade.horseStat.") => Feature::Horse,
        _ => Feature::Always,
    }
}

fn item_key(host: &dyn ClientHost, id: &str) -> Option<ItemKey> {
    host.item_key(&Identifier::parse_namespaced(id)?)
}

fn net_line(host: &dyn ClientHost, line: &NetLine) -> String {
    let args: Vec<String> = line.args.iter().map(|a| arg(host, a)).collect();
    tr(host, &line.key, &args)
}

/// 服务端行参数：`#lit:` 原样文本；`#ticks:N` 时间；`#tr:` 后是以 U+001F 分隔的片段，
/// `@键` 片段翻译、其余原样，用空格连接；没有标签的一律原样显示（不会被当翻译键）。
fn arg(host: &dyn ClientHost, a: &str) -> String {
    if let Some(text) = a.strip_prefix("#lit:") {
        text.to_owned()
    } else if let Some(t) = a.strip_prefix("#ticks:") {
        duration(host, t.parse().unwrap_or(0))
    } else if let Some(parts) = a.strip_prefix("#tr:") {
        parts
            .split('\u{1f}')
            .map(|part| part.strip_prefix('@').map_or_else(|| part.to_owned(), |key| translated(host, key)))
            .collect::<Vec<_>>()
            .join(" ")
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
    fn breaking_follows_the_crosshair_and_fresh_predictions() {
        let mining = Mining { pos: Some([0, 150, 3]), progress: 0.4, recorded_at: 10 };
        assert_eq!(breaking(Some([0, 150, 3]), &mining, 10), Some(0.4));
        // 记录两 tick 内仍算在挖。
        assert_eq!(breaking(Some([0, 150, 3]), &mining, 12), Some(0.4));
        // 过期（松手后）就不显示了。
        assert_eq!(breaking(Some([0, 150, 3]), &mining, 13), None);
        // 准星换到别的方块也不显示。
        assert_eq!(breaking(Some([0, 150, 4]), &mining, 10), None);
        // 没对准任何东西同理。
        assert_eq!(breaking(None, &mining, 10), None);
    }

    #[test]
    fn breaking_matches_block_pos_conversion() {
        // 准星命中经 block_pos 换算，与预测的 [i32; 3] 坐标比较；负坐标同样成立。
        for p in [[-1, 64, -30], [0, -60, 0], [123, 319, -7]] {
            let pos = mod_api::BlockPos { x: p[0], y: p[1], z: p[2] };
            assert_eq!(block_pos(&pos), p);
            let mining = Mining { pos: Some(p), progress: 0.5, recorded_at: 1 };
            assert_eq!(breaking(Some(block_pos(&pos)), &mining, 1), Some(0.5));
            let other = mod_api::BlockPos { x: p[0] + 1, ..pos };
            assert_eq!(breaking(Some(block_pos(&other)), &mining, 1), None);
        }
    }

    #[test]
    fn breaking_hides_zero_and_complete() {
        for progress in [0.0, 1.0] {
            let mining = Mining { pos: Some([0, 150, 3]), progress, recorded_at: 10 };
            assert_eq!(breaking(Some([0, 150, 3]), &mining, 10), None, "progress={progress}");
        }
        let mining = Mining { pos: Some([0, 150, 3]), progress: 0.99, recorded_at: 10 };
        assert_eq!(breaking(Some([0, 150, 3]), &mining, 10), Some(0.99));
    }

    #[test]
    fn formats_java_style() {
        let args = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(format_args_java("%s / %s", &args), "a / b");
        assert_eq!(format_args_java("%2$s-%1$s", &args), "b-a");
        assert_eq!(format_args_java("100%%", &[]), "100%");
        assert_eq!(format_args_java("§oWhat?", &[]), "What?");
    }

    fn items(n: usize) -> Vec<Entry<'static>> {
        (0..n).map(|_| entry(Row::Items(vec![]), Feature::Container)).collect()
    }

    fn line(text: &str, key: &'static str) -> Entry<'static> {
        Entry { row: Row::Line(text.into(), Tone::Normal), detail: Some(key), feature: line_feature(key) }
    }

    #[test]
    fn clicks_flip_the_switch() {
        assert!(toggled(false, 1));
        assert!(!toggled(true, 1));
        assert!(!toggled(false, 2));
        assert!(toggled(false, 3));
        assert!(toggled(true, 0));
    }

    #[test]
    fn compact_limits_item_rows_to_one() {
        let cfg = Config::default();
        assert_eq!(visible_rows(items(1), false, &cfg), (vec![Row::Items(vec![])], false, false));
        assert_eq!(visible_rows(items(1), true, &cfg), (vec![Row::Items(vec![])], false, false));

        let (compact, hidden, truncated) = visible_rows(items(7), false, &cfg);
        assert_eq!(compact.len(), 1);
        assert!(hidden);
        assert!(truncated);

        let (detail, hidden, truncated) = visible_rows(items(7), true, &cfg);
        assert_eq!(detail.len(), 6);
        assert!(!hidden);
        assert!(truncated);

        // 恰好 6 行：详细模式放得下，不提示截断。
        let (_, hidden, truncated) = visible_rows(items(6), true, &cfg);
        assert!(!hidden && !truncated);
    }

    #[test]
    fn compact_hides_detail_keys_only() {
        let cfg = Config::default();
        let rows = vec![
            entry(Row::Health(10.0, 10.0), Feature::Health),
            line("Bees: 3", "jade.beehive.bees"),
            line("Growing time: 10m", "jade.mobgrowth.time"),
            line("Breeding CD: 2m", "jade.mobbreeding.time"),
            line("Breeding CD: In Love", "jade.mobbreeding.fed"),
            line("Next egg: 5m", "jade.nextEgg"),
        ];
        let (compact, hidden, _) = visible_rows(rows.clone(), false, &cfg);
        assert_eq!(compact, vec![Row::Health(10.0, 10.0), Row::Line("Bees: 3".into(), Tone::Normal)]);
        assert!(hidden);

        let (detail, hidden, _) = visible_rows(rows, true, &cfg);
        assert_eq!(detail.len(), 6);
        assert!(!hidden);
    }

    #[test]
    fn feature_switches_filter_rows() {
        let rows = vec![
            entry(Row::Health(10.0, 10.0), Feature::Health),
            entry(Row::Armor(8.0), Feature::Armor),
            line("Speed (0:30)", "jade.potion"),
            line("Owner: Alice", "jade.owner"),
            line("Bees: 3", "jade.beehive.bees"),
        ];
        let mut cfg = Config::default();
        assert_eq!(visible_rows(rows.clone(), true, &cfg).0.len(), 5);
        cfg.set(Feature::Armor, false);
        cfg.set(Feature::Potion, false);
        let (out, hidden, _) = visible_rows(rows.clone(), false, &cfg);
        // 被开关关掉的行不算“精简模式隐藏了内容”：Owner 属详细行，关掉的 Potion 不触发提示。
        assert_eq!(out, vec![Row::Health(10.0, 10.0), Row::Line("Bees: 3".into(), Tone::Normal)]);
        assert!(hidden);
        cfg.set(Feature::Owner, false);
        assert!(!visible_rows(rows, false, &cfg).1);
    }

    #[test]
    fn server_lines_map_to_features() {
        assert_eq!(line_feature("jade.potion"), Feature::Potion);
        assert_eq!(line_feature("jade.horseStat.jump"), Feature::Horse);
        assert_eq!(line_feature("jade.horseStat.speed"), Feature::Horse);
        assert_eq!(line_feature("tooltip.jade.villager_profession"), Feature::Villager);
        assert_eq!(line_feature("jade.owner"), Feature::Owner);
        assert_eq!(line_feature("jade.nextEgg"), Feature::Always);
    }
}
