//! 玩家在“选项 > 模组 > Jade”里调的设置。
//!
//! 设置表 [`specs`] 是键名、翻译键、默认值的唯一出处；[`load`] 把读到的值整理成 [`Config`]。
//! 读设置的宿主接口只能在世界里调用，所以 `setting_changed` 里只打“待重读”标记，
//! 真正的读取放在下一次客户端 tick（见 [`refresh`]）。
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use mod_api::{ClientHost, SettingKind, SettingSpec, SettingValue};

/// 提示框里一类内容；设置里每个分项开关对应一个。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    /// 没有开关的内容（名称、成长时间之类），一直显示。
    Always,
    Health,
    Armor,
    Potion,
    Horse,
    Villager,
    Owner,
    Container,
    Energy,
    FluidTank,
    Furnace,
    Crop,
    Redstone,
    Harvest,
    Mining,
    Fluid,
    Dropped,
    ModName,
}

/// 注册的按键：id、默认键（与 Jade 一致，都在小键盘）、标签的翻译键。
pub const KEYS: [(&str, &str, &str); 2] = [
    (crate::client::KEY_SHOW_OVERLAY, "key.keyboard.keypad.1", "key.jade.show_overlay"),
    (crate::client::KEY_TOGGLE_LIQUID, "key.keyboard.keypad.2", "key.jade.toggle_liquid"),
];

/// 分项开关：功能、设置键、标签的翻译键。默认全开。
const FEATURES: &[(Feature, &str, &str)] = &[
    (Feature::Health, "show_health", "config.jade.plugin_minecraft.entity_health"),
    (Feature::Armor, "show_armor", "config.jade.plugin_minecraft.entity_armor"),
    (Feature::Potion, "show_potion", "config.jade.plugin_minecraft.potion_effects"),
    (Feature::Horse, "show_horse", "config.jade.plugin_minecraft.horse_stats"),
    (Feature::Villager, "show_villager", "config.jade.plugin_minecraft.profession"),
    (Feature::Owner, "show_owner", "config.jade.plugin_minecraft.animal_owner"),
    (Feature::Container, "show_container", "config.jade.plugin_minecraft.item_storage"),
    (Feature::Energy, "show_energy", "config.jade.plugin_minecraft.energy_storage"),
    (Feature::FluidTank, "show_fluid_tank", "config.jade.plugin_minecraft.fluid_storage"),
    (Feature::Furnace, "show_furnace", "config.jade.plugin_minecraft.furnace"),
    (Feature::Crop, "show_crop", "config.jade.plugin_minecraft.crop_progress"),
    (Feature::Redstone, "show_redstone", "config.jade.plugin_minecraft.redstone"),
    (Feature::Harvest, "show_harvest", "config.jade.plugin_minecraft.harvest_tool"),
    (Feature::Mining, "show_mining", "config.jade.plugin_minecraft.breaking_progress"),
    (Feature::Fluid, "show_fluid", "config.jade.display_fluids"),
    (Feature::Dropped, "show_dropped", "config.jade.horizon.dropped_items"),
    (Feature::ModName, "show_mod_name", "config.jade.plugin_jade.mod_name"),
];

/// 提示框锚在屏幕的哪里。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    TopCenter,
    TopLeft,
    TopRight,
    MiddleLeft,
    MiddleRight,
}

const ANCHORS: [(Anchor, &str); 5] = [
    (Anchor::TopCenter, "config.jade.horizon.anchor.top_center"),
    (Anchor::TopLeft, "config.jade.horizon.anchor.top_left"),
    (Anchor::TopRight, "config.jade.horizon.anchor.top_right"),
    (Anchor::MiddleLeft, "config.jade.horizon.anchor.middle_left"),
    (Anchor::MiddleRight, "config.jade.horizon.anchor.middle_right"),
];

/// 配色方案。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeId {
    Classic,
    Dark,
    Light,
    HighContrast,
}

const THEMES: [(ThemeId, &str); 4] = [
    (ThemeId::Classic, "jade.theme.jade.waila"),
    (ThemeId::Dark, "jade.theme.jade.dark"),
    (ThemeId::Light, "config.jade.horizon.theme.light"),
    (ThemeId::HighContrast, "config.jade.horizon.theme.high_contrast"),
];

/// 一份读好的设置。
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub enabled: bool,
    pub anchor: Anchor,
    pub offset_x: i32,
    pub offset_y: i32,
    /// 背景不透明度 0..1，只作用在背景色的 alpha 通道。
    pub alpha: f32,
    pub theme: ThemeId,
    pub animation: bool,
    /// 调试统计：把网络计数写进 `perf.json`（给 `tools/perf.py` 用）。
    pub debug_stats: bool,
    features: u32,
}

impl Config {
    pub fn has(&self, feature: Feature) -> bool {
        match feature_bit(feature) {
            Some(bit) => self.features & bit != 0,
            None => true,
        }
    }

    #[cfg(test)]
    pub fn set(&mut self, feature: Feature, on: bool) {
        if let Some(bit) = feature_bit(feature) {
            if on {
                self.features |= bit;
            } else {
                self.features &= !bit;
            }
        }
    }
}

fn feature_bit(feature: Feature) -> Option<u32> {
    FEATURES.iter().position(|(f, ..)| *f == feature).map(|i| 1 << i)
}

impl Default for Config {
    fn default() -> Self {
        load(&|_| None)
    }
}

/// 全部设置：键名、标签、选项、范围与默认值都在这里。
pub fn specs() -> Vec<SettingSpec> {
    let mut out = vec![
        SettingSpec::toggle("enabled", "config.jade.display_tooltip", true).tooltip("config.jade.display_tooltip_desc"),
        SettingSpec::choice("anchor", "config.jade.overlay_pos", ANCHORS.iter().map(|(_, key)| *key), 0),
        SettingSpec::range("offset_x", "config.jade.horizon.offset_x", -200.0, 200.0, 1.0, 0.0),
        SettingSpec::range("offset_y", "config.jade.horizon.offset_y", -200.0, 200.0, 1.0, 0.0),
        SettingSpec::range("alpha", "config.jade.overlay_alpha", 0.0, 1.0, 0.01, 0.94),
        SettingSpec::choice("theme", "config.jade.overlay_theme", THEMES.iter().map(|(_, key)| *key), 0),
        SettingSpec::toggle("animation", "config.jade.animation", true).tooltip("config.jade.animation_desc"),
    ];
    out.push(SettingSpec::toggle("debug_stats", "config.jade.debug_mode", false));
    out.extend(FEATURES.iter().map(|(_, key, label)| SettingSpec::toggle(*key, *label, true)));
    out
}

/// 设置与按键里用到的所有翻译键（标签、提示、选项）。
#[cfg(test)]
pub fn label_keys() -> Vec<String> {
    let mut keys: Vec<String> = KEYS.iter().map(|(.., label)| (*label).to_owned()).collect();
    for spec in specs() {
        keys.push(spec.label.clone());
        keys.extend(spec.tooltip.clone());
        if let SettingKind::Choice { options, .. } = &spec.kind {
            keys.extend(options.iter().cloned());
        }
    }
    keys
}

/// 用 `read` 取各设置的值（读不到就用默认值）整理成 [`Config`]。纯函数，测试里注入假的读取。
pub fn load(read: &impl Fn(&str) -> Option<SettingValue>) -> Config {
    let toggle = |key: &str, default: bool| match read(key) {
        Some(SettingValue::Toggle(v)) => v,
        _ => default,
    };
    let range = |key: &str, min: f64, max: f64, default: f64| match read(key) {
        Some(SettingValue::Range(v)) if v.is_finite() => v.clamp(min, max),
        _ => default,
    };
    let choice = |key: &str, len: usize| match read(key) {
        Some(SettingValue::Choice(i)) if i < len => i,
        _ => 0,
    };
    let mut features = 0;
    for (i, (_, key, _)) in FEATURES.iter().enumerate() {
        if toggle(key, true) {
            features |= 1 << i;
        }
    }
    Config {
        enabled: toggle("enabled", true),
        anchor: ANCHORS[choice("anchor", ANCHORS.len())].0,
        offset_x: range("offset_x", -200.0, 200.0, 0.0).round() as i32,
        offset_y: range("offset_y", -200.0, 200.0, 0.0).round() as i32,
        alpha: range("alpha", 0.0, 1.0, 0.94) as f32,
        theme: THEMES[choice("theme", THEMES.len())].0,
        animation: toggle("animation", true),
        debug_stats: toggle("debug_stats", false),
        features,
    }
}

/// 从宿主读全部设置。
fn read_host(host: &dyn ClientHost) -> Config {
    let specs = specs();
    load(&|key| {
        let spec = specs.iter().find(|s| s.key == key)?;
        match spec.kind {
            SettingKind::Toggle { .. } => host.setting_bool(key).map(SettingValue::Toggle),
            SettingKind::Choice { .. } => host.setting_choice(key).map(|i| SettingValue::Choice(i as usize)),
            SettingKind::Range { .. } => host.setting_f64(key).map(SettingValue::Range),
            SettingKind::Color { .. } => host.setting_color(key).map(SettingValue::Color),
            _ => None,
        }
    })
}

static CURRENT: OnceLock<Mutex<Config>> = OnceLock::new();
/// 设置被改过（或还没读过）：下一次 tick 重读。
static STALE: AtomicBool = AtomicBool::new(true);

fn cell() -> &'static Mutex<Config> {
    CURRENT.get_or_init(|| Mutex::new(Config::default()))
}

/// `setting_changed` 钩子里调用：只打标记。
pub fn mark_stale() {
    STALE.store(true, Ordering::Relaxed);
}

/// 每个客户端 tick 调用：有标记就重读，返回当前设置。
pub fn refresh(host: &dyn ClientHost) -> Config {
    if STALE.swap(false, Ordering::Relaxed) {
        let fresh = read_host(host);
        *cell().lock().unwrap_or_else(|e| e.into_inner()) = fresh;
    }
    current()
}

/// 最近一次读到的设置；还没读到时是默认值。
pub fn current() -> Config {
    cell().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_spec_passes_the_sdk_check() {
        let mut keys = HashSet::new();
        for spec in specs() {
            assert_eq!(spec.check(), Ok(()), "{}", spec.key);
            assert!(keys.insert(spec.key.clone()), "重复的键 {}", spec.key);
        }
    }

    #[test]
    fn defaults_match_the_original_look() {
        let c = Config::default();
        assert!(c.enabled && c.animation);
        assert_eq!((c.anchor, c.theme, c.offset_x, c.offset_y), (Anchor::TopCenter, ThemeId::Classic, 0, 0));
        // 默认不透明度要换算成原来的 0xF0。
        assert_eq!((c.alpha * 255.0).round() as u32, 0xF0);
        for (feature, ..) in FEATURES {
            assert!(c.has(*feature));
        }
    }

    #[test]
    fn load_reads_each_setting_and_ignores_bad_values() {
        let read = |key: &str| match key {
            "enabled" => Some(SettingValue::Toggle(false)),
            "anchor" => Some(SettingValue::Choice(2)),
            "offset_x" => Some(SettingValue::Range(-12.4)),
            "alpha" => Some(SettingValue::Range(0.5)),
            "theme" => Some(SettingValue::Choice(99)),
            "show_furnace" => Some(SettingValue::Toggle(false)),
            // 种类不对的值当作没读到。
            "show_health" => Some(SettingValue::Range(0.0)),
            _ => None,
        };
        let c = load(&read);
        assert!(!c.enabled);
        assert_eq!(c.anchor, Anchor::TopRight);
        assert_eq!(c.offset_x, -12);
        assert_eq!(c.alpha, 0.5);
        assert_eq!(c.theme, ThemeId::Classic);
        assert!(!c.has(Feature::Furnace));
        assert!(c.has(Feature::Health));
        assert!(c.has(Feature::Always));
    }

    #[test]
    fn feature_bits_do_not_collide() {
        let mut c = Config::default();
        c.set(Feature::Crop, false);
        for (feature, ..) in FEATURES {
            assert_eq!(c.has(*feature), *feature != Feature::Crop);
        }
        c.set(Feature::Crop, true);
        assert_eq!(c, Config::default());
    }
}
