//! 服务端：读出玩家看着的方块实体或实体的数据，整理成 [`ServerData`]。
//!
//! 对应 Jade 中需要服务端的 provider：ItemStorage（容器内容）、Furnace、BrewingStand、
//! Campfire、Beehive、MobSpawner、Jukebox、Lectern、MobGrowth、MobBreeding 等。
use std::sync::{Mutex, OnceLock};

use mod_api::{Compound, PlayerRef, ServerHost, Tag};

use crate::limit::Limiter;
use crate::net::{KIND_BLOCK, KIND_ENTITY, NetItem, NetLine, NetProgress, NetTank, NetTarget, Request, ServerData, TANK_ENERGY, TANK_FLUID};

fn limiter() -> std::sync::MutexGuard<'static, Limiter> {
    static LIMITER: OnceLock<Mutex<Limiter>> = OnceLock::new();
    LIMITER.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// 处理一个请求：限流、收集数据、去重，返回需要回给玩家的数据。
pub fn handle(host: &mut dyn ServerHost, player: PlayerRef, request: &Request) -> Option<ServerData> {
    let now = player.level(&*host)?.game_time(&*host)?;
    if !limiter().admit(player.uuid(), now) {
        return None;
    }
    let data = collect(host, player, &request.target)?;
    limiter().should_send(player.uuid(), request.fresh, &data).then_some(data)
}

/// 玩家离开时清掉限流状态。
pub fn forget(player: u128) {
    limiter().forget(player);
}

/// 玩家与目标的最大距离（格）。超出则不回数据，避免被用来探查远处的箱子。
const MAX_DISTANCE: f64 = 16.0;

pub fn collect(host: &mut dyn ServerHost, player: PlayerRef, target: &NetTarget) -> Option<ServerData> {
    let level = player.level(&*host)?;
    let eye = player.entity().position(&*host)?;
    let mut data = ServerData { target: target.clone(), ..Default::default() };
    match target.kind {
        KIND_BLOCK => {
            let pos = [target.x, target.y, target.z];
            let center = [pos[0] as f64 + 0.5, pos[1] as f64 + 0.5, pos[2] as f64 + 0.5];
            if dist2(eye, center) > MAX_DISTANCE * MAX_DISTANCE {
                return None;
            }
            let be = host.block_entity(level, pos);
            if let Some(be) = be {
                let kind = be.kind(&*host).map(|k| k.id(&*host)).unwrap_or_default();
                let nbt = be.data(&*host).unwrap_or_default();
                block_entity(host, level, pos, &kind, &nbt, &mut data);
            }
            // 能量与流体能力按坐标查：炼药锅这类没有方块实体的方块也有流体能力。
            tanks(host, level, pos, &mut data.tanks);
            // 没有方块实体又没有储罐：没什么可回的，不浪费一个网络包。
            if be.is_none() && data.tanks.is_empty() {
                return None;
            }
        }
        KIND_ENTITY => {
            let entity = host.entity(target.entity as u32)?;
            // 只回同一维度的实体，防止借坐标探查其他维度。
            if entity.level(&*host)? != level {
                return None;
            }
            let pos = entity.position(&*host)?;
            if dist2(eye, pos) > MAX_DISTANCE * MAX_DISTANCE {
                return None;
            }
            let kind = entity.kind(&*host).map(|k| k.id(&*host)).unwrap_or_default();
            if let Some(living) = host.living(entity) {
                if let (Some(hp), Some(max)) = (living.health(&*host), living.max_health(&*host)) {
                    data.health = hp;
                    data.max_health = max;
                }
                living_lines(host, entity, living, &kind, &mut data);
            }
            if let Some(item) = host.item_entity(entity).and_then(|e| e.stack(&*host)) {
                data.items.push(NetItem { id: item.item().id(&*host), count: clamp_count(item.count()) });
            }
            let nbt = entity.nbt(host).unwrap_or_default();
            entity_data(&kind, &nbt, &mut data);
        }
        _ => return None,
    }
    Some(data)
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

fn block_entity(
    host: &mut dyn ServerHost,
    level: mod_api::LevelRef,
    pos: [i32; 3],
    kind: &str,
    nbt: &Compound,
    data: &mut ServerData,
) {
    let slots = slot_items(nbt);
    match kind {
        "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => {
            data.progress.push(NetProgress {
                inputs: vec![slot(&slots, 0), slot(&slots, 1)],
                outputs: vec![slot(&slots, 2)],
                current: int_any(nbt, &["cooking_time_spent", "CookTime"]),
                total: int_any(nbt, &["cooking_total_time", "CookTimeTotal"]),
            });
            return;
        }
        "minecraft:brewing_stand" => {
            let brew = int_any(nbt, &["BrewTime"]);
            data.progress.push(NetProgress {
                inputs: vec![slot(&slots, 3), slot(&slots, 4)],
                outputs: vec![slot(&slots, 0), slot(&slots, 1), slot(&slots, 2)],
                current: if brew > 0 { 400 - brew } else { 0 },
                total: 400,
            });
            let fuel = int_any(nbt, &["Fuel"]);
            data.lines.push(line("narration.jade.brewingStand.fuel", [fuel.to_string()]));
            return;
        }
        "minecraft:campfire" | "minecraft:soul_campfire" => {
            let times = int_array(nbt, "CookingTimes");
            let totals = int_array(nbt, "CookingTotalTimes");
            for (i, item) in slots_in_order(nbt, 4).into_iter().enumerate() {
                let Some(item) = item else { continue };
                let total = totals.get(i).copied().unwrap_or(0);
                let current = times.get(i).copied().unwrap_or(0);
                data.progress.push(NetProgress { inputs: vec![item], outputs: vec![], current, total });
            }
            return;
        }
        "minecraft:beehive" | "minecraft:bee_nest" => {
            let bees = list_len(nbt, &["bees", "Bees"]);
            data.lines.push(line("jade.beehive.bees", [bees.to_string()]));
            return;
        }
        "minecraft:mob_spawner" | "minecraft:trial_spawner" => {
            if let Some(id) = cpath(nbt, &["SpawnData", "entity", "id"])
                .or_else(|| cpath(nbt, &["spawn_data", "entity", "id"]))
                .and_then(tag_str)
            {
                data.lines.push(line("jade.spawner", [tr_arg(&[&format!("block.{}", kind.replace(':', "."))]), tr_arg(&[&entity_key(&id)])]));
            }
            let delay = int_any(nbt, &["Delay"]);
            if delay > 0 {
                data.lines.push(line("jade.trial_spawner_cd", [ticks_text(delay)]));
            }
            return;
        }
        "minecraft:jukebox" => {
            if let Some(item) = cget(nbt, "RecordItem").and_then(item_of) {
                data.items.push(item);
            }
            return;
        }
        "minecraft:lectern" => {
            if let Some(item) = cget(nbt, "Book").and_then(item_of) {
                data.items.push(item);
            }
            return;
        }
        _ => {}
    }
    // 通用容器：先读物品能力，读不到再读 NBT 的 Items。
    if cget(nbt, "LootTable").is_some() {
        data.lines.push(line("jade.loot_not_generated", []));
        return;
    }
    if cget(nbt, "lock").is_some() || cget(nbt, "Lock").is_some() {
        data.lines.push(line("jade.locked", []));
        return;
    }
    let mut items = storage_items(host, level, pos);
    if items.is_empty() {
        items = slots.into_iter().map(|(_, item)| item).collect();
    }
    data.items = merge(items);
}

/// 通过 `horizon:storage` 能力列出容器内容：原版容器与注册了该能力的模组容器都适用。
fn storage_items(host: &mut dyn ServerHost, level: mod_api::LevelRef, pos: [i32; 3]) -> Vec<NetItem> {
    let mut out = Vec::new();
    let Some(storage) = level.capability(host, &mod_api::STORAGE, pos, None) else {
        return out;
    };
    let Ok(mut page) = mod_api::StoragePage::new(64) else {
        return out;
    };
    let mut cursor = 0;
    // 最多翻 8 页，避免巨型存储拖慢服务端。
    for _ in 0..8 {
        let next = storage.page(host, cursor, &mut page);
        for (resource, amount) in page.entries() {
            let id = resource.item().id(&*host);
            out.push(NetItem { id, count: clamp_count(*amount) });
        }
        match next {
            Ok(Some(n)) => cursor = n,
            _ => break,
        }
    }
    out
}

/// 一个方块最多回多少个流体储罐。
const MAX_FLUID_TANKS: usize = 8;

/// 通过 `horizon:energy` 与 `horizon:fluids` 能力读出能量缓冲和流体储罐。
fn tanks(host: &mut dyn ServerHost, level: mod_api::LevelRef, pos: [i32; 3], out: &mut Vec<NetTank>) {
    if let Some(energy) = level.capability(host, &mod_api::ENERGY, pos, None) {
        let capacity = energy.capacity(host);
        if capacity > 0 {
            let amount = energy.amount(host);
            out.push(NetTank { kind: TANK_ENERGY, id: String::new(), amount: clamp_count(amount), capacity: clamp_count(capacity) });
        }
    }
    let Some(fluids) = level.capability(host, &mod_api::FLUIDS, pos, None) else {
        return;
    };
    for index in 0..fluids.size(host).min(MAX_FLUID_TANKS) {
        let Some(resource) = fluids.resource(host, index) else {
            continue;
        };
        let amount = fluids.amount(host, index);
        if amount == 0 {
            continue;
        }
        let Some(key) = resource.fluid().key() else {
            continue;
        };
        let capacity = fluids.capacity(host, index, &resource).max(amount);
        out.push(NetTank { kind: TANK_FLUID, id: key.id(&*host), amount: clamp_count(amount), capacity: clamp_count(capacity) });
    }
}

fn entity_data(kind: &str, nbt: &Compound, data: &mut ServerData) {
    // 幼年生物的成长时间，成年动物的繁殖冷却（Age 字段，单位 tick）。
    // 掉落物的 Age 是存在时间，不是成长/繁殖冷却。
    let has_growth_age = kind != "minecraft:item";
    if let Some(age) = cget(nbt, "Age").and_then(tag_int).filter(|_| has_growth_age) {
        if age < 0 {
            data.lines.push(line("jade.mobgrowth.time", [ticks_text(-age)]));
        } else if age > 0 {
            data.lines.push(line("jade.mobbreeding.time", [ticks_text(age)]));
        }
    }
    if cget(nbt, "InLove").and_then(tag_int).is_some_and(|v| v > 0) {
        data.lines.push(line("jade.mobbreeding.fed", []));
    }
    match kind {
        "minecraft:chicken" => {
            if let Some(t) = cget(nbt, "EggLayTime").and_then(tag_int) {
                data.lines.push(line("jade.nextEgg", [ticks_text(t)]));
            }
        }
        "minecraft:item_frame" | "minecraft:glow_item_frame" => {
            if let Some(item) = cget(nbt, "Item").and_then(item_of) {
                data.items.push(item);
            }
        }
        "minecraft:painting" => {
            if let Some(variant) = cget(nbt, "variant").and_then(tag_str) {
                data.lines.push(line("jade.horizon.painting", [tr_arg(&[&painting_key(&variant)])]));
            }
        }
        _ => {}
    }
}

/// 生物专属信息：盔甲值、药水效果、马类属性、村民职业、主人、盔甲架装备。
fn living_lines(host: &mut dyn ServerHost, entity: mod_api::EntityRef, living: mod_api::LivingRef, kind: &str, data: &mut ServerData) {
    data.armor = living.attribute_value(&*host, "minecraft:armor").unwrap_or(0.0) as f32;
    for effect in living.effects(&*host).unwrap_or_default() {
        data.lines.push(potion_line(&effect.id.id(&*host), effect.amplifier, effect.duration));
    }
    if HORSES.contains(&kind) {
        let speed = living.attribute_value(&*host, "minecraft:movement_speed");
        let jump = living.attribute_value(&*host, "minecraft:jump_strength");
        if let Some(jump) = jump {
            data.lines.push(line("jade.horseStat.jump", [format!("{:.2}", horse_jump_blocks(jump))]));
        }
        if let Some(speed) = speed {
            data.lines.push(line("jade.horseStat.speed", [format!("{:.2}", horse_speed_blocks(speed))]));
        }
    }
    if let Some(merchant) = host.merchant(entity) {
        let profession = merchant.profession(&*host);
        let level = merchant.level(&*host);
        if let Some(arg) = profession.as_deref().and_then(|p| profession_arg(p, level)) {
            data.lines.push(line("tooltip.jade.villager_profession", [arg]));
        }
    }
    if let Some(owner) = living.owner(&*host) {
        // 在线就记下名字；主人离线时用最近见过的名字，都没有才退回 UUID 前 8 位。
        let name = match host.player(owner).and_then(|p| p.name(&*host)) {
            Some(name) => {
                remember_name(owner, &name);
                name
            }
            None => known_name(owner).unwrap_or_else(|| short_uuid(owner)),
        };
        data.lines.push(line("jade.owner", [lit(&name)]));
    }
    if kind == "minecraft:armor_stand" {
        let items = mod_api::EquipmentSlot::ALL
            .into_iter()
            .filter_map(|slot| living.equipment(&*host, slot))
            .map(|stack| NetItem { id: stack.item().id(&*host), count: clamp_count(stack.count()) })
            .collect();
        data.items.extend(merge(items));
    }
}

/// 有速度与跳跃力属性、值得显示的坐骑。
const HORSES: &[&str] = &[
    "minecraft:horse",
    "minecraft:donkey",
    "minecraft:mule",
    "minecraft:skeleton_horse",
    "minecraft:zombie_horse",
    "minecraft:llama",
    "minecraft:trader_llama",
    "minecraft:camel",
];

/// 移动速度属性 → 格/秒。
pub fn horse_speed_blocks(attribute: f64) -> f64 {
    attribute * 42.16
}

/// 跳跃力属性 → 跳跃高度（格），Jade 用的拟合公式。
pub fn horse_jump_blocks(strength: f64) -> f64 {
    -0.1817584952 * strength.powi(3) + 3.689713992 * strength.powi(2) + 2.128599134 * strength - 0.343930367
}

/// 药水效果一行：`jade.potion` = `名称 等级` + 剩余时间；等级 1 不写；无限时长写"无限"。
pub fn potion_line(id: &str, amplifier: i32, duration: i32) -> NetLine {
    let (ns, path) = split_id(id);
    let effect = format!("effect.{ns}.{path}");
    let name = if amplifier <= 0 {
        tr_arg(&[&effect])
    } else if amplifier < 9 {
        tr_arg(&[&effect, &format!("enchantment.level.{}", amplifier + 1)])
    } else {
        // 9 级以上没有罗马数字翻译，直接写数字。
        format!("#tr:@{effect}\u{1f}{}", amplifier + 1)
    };
    let time = if duration < 0 { tr_arg(&["effect.duration.infinite"]) } else { ticks_text(duration) };
    line("jade.potion", [name, time])
}

/// 村民职业与等级：`entity.minecraft.villager.<职业> merchant.level.<等级>`。没有职业或无等级信息时按有的显示。
pub fn profession_arg(profession: &str, level: Option<i32>) -> Option<String> {
    let (ns, path) = split_id(profession);
    if path == "none" {
        return None;
    }
    let profession = format!("entity.{ns}.villager.{path}");
    Some(match level.filter(|l| (1..=5).contains(l)) {
        Some(level) => tr_arg(&[&profession, &format!("merchant.level.{level}")]),
        None => tr_arg(&[&profession]),
    })
}

/// 画的翻译键由游戏提供：`painting.<ns>.<path>.title`。
pub fn painting_key(variant: &str) -> String {
    let (ns, path) = split_id(variant);
    format!("painting.{ns}.{path}.title")
}

fn short_uuid(uuid: u128) -> String {
    format!("{:08x}", (uuid >> 96) as u32)
}

// ---- NBT 小工具 ----

fn cget<'a>(nbt: &'a Compound, key: &str) -> Option<&'a Tag> {
    nbt.iter().find(|(k, _)| k.to_string_lossy() == key).map(|(_, v)| v)
}

fn cpath<'a>(nbt: &'a Compound, path: &[&str]) -> Option<&'a Tag> {
    let (first, rest) = path.split_first()?;
    let mut tag = cget(nbt, first)?;
    for key in rest {
        tag = tag.get(key)?;
    }
    Some(tag)
}

fn tag_int(tag: &Tag) -> Option<i32> {
    match tag {
        Tag::Byte(v) => Some(*v as i32),
        Tag::Short(v) => Some(*v as i32),
        Tag::Int(v) => Some(*v),
        Tag::Long(v) => Some(*v as i32),
        _ => None,
    }
}

fn tag_str(tag: &Tag) -> Option<String> {
    match tag {
        Tag::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

fn int_any(nbt: &Compound, keys: &[&str]) -> i32 {
    keys.iter().find_map(|k| cget(nbt, k).and_then(tag_int)).unwrap_or(0)
}

fn int_array(nbt: &Compound, key: &str) -> Vec<i32> {
    match cget(nbt, key) {
        Some(Tag::IntArray(v)) => v.clone(),
        Some(Tag::List(v)) => v.iter().filter_map(tag_int).collect(),
        _ => Vec::new(),
    }
}

fn list_len(nbt: &Compound, keys: &[&str]) -> usize {
    keys.iter()
        .find_map(|k| match cget(nbt, k) {
            Some(Tag::List(v)) => Some(v.len()),
            _ => None,
        })
        .unwrap_or(0)
}

/// 一个物品的 NBT（`{id, count}` 或旧式 `{id, Count}`）。
fn item_of(tag: &Tag) -> Option<NetItem> {
    let id = tag.get("id").and_then(tag_str)?;
    if id == "minecraft:air" {
        return None;
    }
    let count = tag.get("count").or_else(|| tag.get("Count")).and_then(tag_int).unwrap_or(1);
    Some(NetItem { id, count })
}

/// `Items` 列表：每项带 `Slot`。
fn slot_items(nbt: &Compound) -> Vec<(i32, NetItem)> {
    let Some(Tag::List(list)) = cget(nbt, "Items") else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|t| {
            let slot = t.get("Slot").and_then(tag_int).unwrap_or(0);
            Some((slot, item_of(t)?))
        })
        .collect()
}

fn slots_in_order(nbt: &Compound, size: usize) -> Vec<Option<NetItem>> {
    let mut out = vec![None; size];
    for (slot, item) in slot_items(nbt) {
        if let Some(cell) = out.get_mut(slot as usize) {
            *cell = Some(item);
        }
    }
    out
}

fn slot(slots: &[(i32, NetItem)], index: i32) -> NetItem {
    slots.iter().find(|(s, _)| *s == index).map(|(_, i)| i.clone()).unwrap_or_default()
}

fn merge(items: Vec<NetItem>) -> Vec<NetItem> {
    let mut out: Vec<NetItem> = Vec::new();
    for item in items {
        match out.iter_mut().find(|i| i.id == item.id) {
            Some(existing) => existing.count += item.count,
            None => out.push(item),
        }
    }
    out
}

fn entity_key(id: &str) -> String {
    let (ns, path) = split_id(id);
    format!("entity.{ns}.{path}")
}

fn line<const N: usize>(key: &str, args: [String; N]) -> NetLine {
    NetLine { key: key.to_owned(), args: args.into() }
}

/// `minecraft:x` 拆成 `(命名空间, 路径)`；没有命名空间按 minecraft。
fn split_id(id: &str) -> (&str, &str) {
    id.split_once(':').unwrap_or(("minecraft", id))
}

/// 物品数量压进线协议的 i32。
fn clamp_count(count: impl Into<u64>) -> i32 {
    count.into().min(i32::MAX as u64) as i32
}

/// 翻译键参数：各键翻译后用空格连接。参数由服务端明确标记，客户端不会把别的文本当键。
fn tr_arg(keys: &[&str]) -> String {
    let parts: Vec<String> = keys.iter().map(|k| format!("@{k}")).collect();
    format!("#tr:{}", parts.join("\u{1f}"))
}

/// 原样显示的文本（玩家名等不可信内容），客户端不做任何翻译或解析。
fn lit(text: &str) -> String {
    format!("#lit:{text}")
}

/// 见过的玩家名（uuid → 名字），主人离线时用。进程内缓存，重启后重新积累。
fn names() -> std::sync::MutexGuard<'static, std::collections::HashMap<u128, String>> {
    static NAMES: OnceLock<Mutex<std::collections::HashMap<u128, String>>> = OnceLock::new();
    NAMES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

pub fn remember_name(uuid: u128, name: &str) {
    names().insert(uuid, name.to_owned());
}

fn known_name(uuid: u128) -> Option<String> {
    names().get(&uuid).cloned()
}

/// tick 转成 Jade 的时间文本参数，客户端按 `jade.seconds` 等键格式化。
fn ticks_text(ticks: i32) -> String {
    format!("#ticks:{ticks}")
}

#[cfg(test)]
mod living_tests {
    use super::*;

    #[test]
    fn potion_line_formats_level_and_time() {
        assert_eq!(potion_line("minecraft:speed", 0, 600).args, ["#tr:@effect.minecraft.speed", "#ticks:600"]);
        assert_eq!(potion_line("minecraft:speed", 1, 600).args[0], "#tr:@effect.minecraft.speed\u{1f}@enchantment.level.2");
        assert_eq!(potion_line("minecraft:strength", 11, -1).args, ["#tr:@effect.minecraft.strength\u{1f}12", "#tr:@effect.duration.infinite"]);
        assert_eq!(potion_line("mod:x", 0, 20).args[0], "#tr:@effect.mod.x");
    }

    #[test]
    fn potion_args_match_lang_order() {
        // jade.potion = "%s (%s)"：第 1 个是名称与等级，第 2 个是剩余时间。
        let line = potion_line("minecraft:speed", 1, 600);
        assert_eq!(line.key, "jade.potion");
        assert_eq!(line.args.len(), 2);
        assert!(line.args[0].starts_with("#tr:@effect."));
        assert!(line.args[1].starts_with("#ticks:"));
        let lang = include_str!("../assets/jade/lang/en_us.json");
        assert!(lang.contains("\"jade.potion\": \"%s (%s)\""));
    }

    #[test]
    fn owner_names_are_literal_and_cached() {
        assert_eq!(lit("@a #ticks:5"), "#lit:@a #ticks:5");
        remember_name(42, "Alex");
        assert_eq!(known_name(42).as_deref(), Some("Alex"));
        assert_eq!(known_name(43), None);
    }

    #[test]
    fn clamp_and_split() {
        assert_eq!(clamp_count(5u32), 5);
        assert_eq!(clamp_count(u64::MAX), i32::MAX);
        assert_eq!(split_id("mod:x"), ("mod", "x"));
        assert_eq!(split_id("x"), ("minecraft", "x"));
    }

    #[test]
    fn horse_stats_convert_like_jade() {
        assert!((horse_speed_blocks(0.225) - 9.486).abs() < 1e-3);
        // 跳跃力 0.7 约 2.9 格，1.0（上限）约 5.3 格。
        assert!((horse_jump_blocks(0.7) - 2.89).abs() < 0.02);
        assert!((horse_jump_blocks(1.0) - 5.29).abs() < 0.02);
    }

    #[test]
    fn profession_hides_none_and_bad_levels() {
        assert_eq!(profession_arg("minecraft:none", Some(1)), None);
        assert_eq!(profession_arg("minecraft:librarian", Some(2)).unwrap(), "#tr:@entity.minecraft.villager.librarian\u{1f}@merchant.level.2");
        assert_eq!(profession_arg("minecraft:farmer", None).unwrap(), "#tr:@entity.minecraft.villager.farmer");
    }

    #[test]
    fn painting_and_uuid_keys() {
        assert_eq!(painting_key("minecraft:kebab"), "painting.minecraft.kebab.title");
        assert_eq!(short_uuid(0xdeadbeef_u128 << 96), "deadbeef");
    }
}
