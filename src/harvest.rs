//! 挖掘信息：这个方块要什么工具、手上的工具能不能采下掉落物（HarvestToolProvider）。
//!
//! 全部在客户端用方块标签与玩家背包判定：需要的工具类型看 `mineable/*` 标签，
//! 等级看 `needs_*_tool` 标签，能不能采集用 `Player.hasCorrectToolForDrops`。
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use mod_api::{BlockKey, BlockState, ClientHost, Hand, HostBlockStates, Identifier, ItemKey};

/// 一个方块的挖掘要求。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Harvest {
    /// 需要的工具；None 表示认不出该用什么工具，只显示能不能采集。
    pub tool: Option<ItemKey>,
    /// 玩家手上的工具能否采下掉落物。
    pub ok: bool,
}

/// 能挖掘方块的工具种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Pickaxe,
    Axe,
    Shovel,
    Hoe,
    Shears,
}

impl Kind {
    /// 物品 id 里的工具名：`minecraft:<等级>_<工具名>`。
    fn name(self) -> &'static str {
        match self {
            Kind::Pickaxe => "pickaxe",
            Kind::Axe => "axe",
            Kind::Shovel => "shovel",
            Kind::Hoe => "hoe",
            Kind::Shears => "shears",
        }
    }
}

/// 工具等级，决定图标用木、石、铁还是钻石。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tier {
    Wooden,
    Stone,
    Iron,
    Diamond,
}

impl Tier {
    fn name(self) -> &'static str {
        match self {
            Tier::Wooden => "wooden",
            Tier::Stone => "stone",
            Tier::Iron => "iron",
            Tier::Diamond => "diamond",
        }
    }
}

/// 工具物品的 id；剪刀不分等级，是唯一的例外。
fn tool_id(kind: Kind, tier: Tier) -> String {
    match kind {
        Kind::Shears => "minecraft:shears".to_owned(),
        _ => format!("minecraft:{}_{}", tier.name(), kind.name()),
    }
}

const KINDS: [(Kind, &str); 5] = [
    (Kind::Pickaxe, "minecraft:mineable/pickaxe"),
    (Kind::Axe, "minecraft:mineable/axe"),
    (Kind::Shovel, "minecraft:mineable/shovel"),
    (Kind::Hoe, "minecraft:mineable/hoe"),
    (Kind::Shears, "minecraft:mineable/shears"),
];

const TIERS: [(Tier, &str); 3] = [
    (Tier::Diamond, "minecraft:needs_diamond_tool"),
    (Tier::Iron, "minecraft:needs_iron_tool"),
    (Tier::Stone, "minecraft:needs_stone_tool"),
];

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 方块 id → 需要的工具。标签查询每次都要跨进宿主，按方块缓存。
fn cached_tools() -> &'static Mutex<HashMap<String, Option<ItemKey>>> {
    static TOOLS: OnceLock<Mutex<HashMap<String, Option<ItemKey>>>> = OnceLock::new();
    TOOLS.get_or_init(Default::default)
}

/// 准星方块的挖掘要求；不需要正确工具（泥土、木头、树叶等）时返回 None，不显示这一行。
///
/// 这是有意的取舍：这类方块空手也能采下掉落物，“能否采集”永远是“能”，
/// 再显示“最适合的工具”会让准星扫过的大多数方块都多一行，信息量很小。
/// 需要正确工具的方块（石头、矿石、黑曜石等）才显示工具图标和能否采集。
pub fn of(host: &dyn ClientHost, state: BlockState) -> Option<Harvest> {
    let blocks = host.block_states();
    if !blocks.requires_correct_tool(state)? {
        return None;
    }
    let ok = can_harvest(host, state);
    Some(Harvest { tool: tool(host, state.block()), ok })
}

/// 玩家手上的主手工具能不能采下掉落物。
///
/// 先按手持物品自己判（`ItemStack.isCorrectToolForDrops`），读不到手时
/// 退回 `Player.hasCorrectToolForDrops`；两边都读不到就算不能采集。
fn can_harvest(host: &dyn ClientHost, state: BlockState) -> bool {
    let Some(player) = host.local_player() else {
        return false;
    };
    if let Some(stack) = host.living(player.entity()).and_then(|living| living.hand_item(host, Hand::Main)) {
        return host.block_states().is_correct_tool_for_drops(state, &stack).unwrap_or(false);
    }
    player.has_correct_tool_for_drops(host, state).unwrap_or(false)
}

fn tool(host: &dyn ClientHost, block: BlockKey) -> Option<ItemKey> {
    let id = block.id(host);
    if let Some(cached) = lock(cached_tools()).get(&id) {
        return *cached;
    }
    let tool = resolve(host, block);
    lock(cached_tools()).insert(id, tool);
    tool
}

/// 按标签推出该用什么工具：类型看 `mineable/*`，等级看 `needs_*_tool`（默认木）。
fn resolve(host: &dyn ClientHost, block: BlockKey) -> Option<ItemKey> {
    let kind = KINDS.iter().find(|(_, tag)| in_tag(host, block, tag)).map(|(kind, _)| *kind)?;
    let tier = TIERS
        .iter()
        .find(|(_, tag)| in_tag(host, block, tag))
        .map(|(tier, _)| *tier)
        .unwrap_or(Tier::Wooden);
    host.item_key(&Identifier::parse_namespaced(&tool_id(kind, tier))?)
}

fn in_tag(host: &dyn ClientHost, block: BlockKey, tag: &str) -> bool {
    host.block_in_tag(block, tag) == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_ids_follow_tier_and_kind() {
        assert_eq!(tool_id(Kind::Pickaxe, Tier::Wooden), "minecraft:wooden_pickaxe");
        assert_eq!(tool_id(Kind::Pickaxe, Tier::Diamond), "minecraft:diamond_pickaxe");
        assert_eq!(tool_id(Kind::Axe, Tier::Iron), "minecraft:iron_axe");
        assert_eq!(tool_id(Kind::Shovel, Tier::Stone), "minecraft:stone_shovel");
        assert_eq!(tool_id(Kind::Hoe, Tier::Diamond), "minecraft:diamond_hoe");
        // 剪刀不分等级，永远同一个物品。
        assert_eq!(tool_id(Kind::Shears, Tier::Wooden), "minecraft:shears");
        assert_eq!(tool_id(Kind::Shears, Tier::Diamond), "minecraft:shears");
    }
}
