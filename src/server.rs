//! 服务端：读出玩家看着的方块实体或实体的数据，整理成 [`ServerData`]。
//!
//! 对应 Jade 中需要服务端的 provider：ItemStorage（容器内容）、Furnace、BrewingStand、
//! Campfire、Beehive、MobSpawner、Jukebox、Lectern、MobGrowth、MobBreeding 等。
use mod_api::{Compound, PlayerRef, ServerHost, Tag};

use crate::net::{KIND_BLOCK, KIND_ENTITY, NetItem, NetLine, NetProgress, NetTarget, ServerData};

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
            let be = host.block_entity(level, pos)?;
            let kind = be.kind(&*host).map(|k| k.id(&*host)).unwrap_or_default();
            let nbt = be.data(&*host).unwrap_or_default();
            block_entity(host, level, pos, &kind, &nbt, &mut data);
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
                data.lines.push(line("jade.spawner", [format!("@block.{}", kind.replace(':', ".")), format!("@{}", entity_key(&id))]));
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
            out.push(NetItem { id, count: (*amount).min(i32::MAX as u64) as i32 });
        }
        match next {
            Ok(Some(n)) => cursor = n,
            _ => break,
        }
    }
    out
}

fn entity_data(kind: &str, nbt: &Compound, data: &mut ServerData) {
    // 幼年生物的成长时间，成年动物的繁殖冷却（Age 字段，单位 tick）。
    if let Some(age) = cget(nbt, "Age").and_then(tag_int) {
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
        _ => {}
    }
    // 驯服生物的主人：只给出 UUID 是否存在，名字由客户端显示不了，Jade 也只在服务端解析名字。
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
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("entity.{ns}.{path}")
}

fn line<const N: usize>(key: &str, args: [String; N]) -> NetLine {
    NetLine { key: key.to_owned(), args: args.into() }
}

/// tick 转成 Jade 的时间文本参数，客户端按 `jade.seconds` 等键格式化。
fn ticks_text(ticks: i32) -> String {
    format!("#ticks:{ticks}")
}
