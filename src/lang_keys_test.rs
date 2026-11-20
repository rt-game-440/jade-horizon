//! 自动验收：代码里写死的每个 Jade 翻译键都必须在 en_us.json 与 zh_cn.json 里存在，
//! 否则游戏里会直接显示原始键名。
use std::collections::HashSet;
use std::fs;

fn keys_in(lang: &str) -> HashSet<String> {
    let text = fs::read_to_string(format!("{}/assets/jade/lang/{lang}.json", env!("CARGO_MANIFEST_DIR"))).unwrap();
    // 语言文件是扁平的 "key": "value"，按行取键即可，不引入 JSON 依赖。
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix('"')?;
            let end = rest.find("\":")?;
            Some(rest[..end].to_owned())
        })
        .collect()
}

/// 源码中所有形如 "jade.xxx" / "tooltip.jade.xxx" / "narration.jade.xxx" 的字面量。
fn keys_in_source() -> HashSet<String> {
    let mut out = HashSet::new();
    for entry in fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().is_some_and(|n| n == "lang_keys_test.rs") {
            continue;
        }
        let text = fs::read_to_string(path).unwrap();
        for part in text.split('"').skip(1).step_by(2) {
            let is_key = part.starts_with("jade.") || part.starts_with("tooltip.jade.") || part.starts_with("narration.jade.");
            if is_key && !part.ends_with('.') && !part.contains(['{', ' ']) {
                out.insert(part.to_owned());
            }
        }
    }
    out
}

/// 拼接出来的 Jade 键：音符盒乐器。
/// 新增的 `jade.horizon.*` 都是字面量，源码扫描已经覆盖。
fn dynamic_keys() -> Vec<String> {
    let instruments = [
        "harp", "basedrum", "snare", "hat", "bass", "flute", "bell", "guitar", "chime", "xylophone",
        "iron_xylophone", "cow_bell", "didgeridoo", "bit", "banjo", "pling", "zombie", "skeleton", "creeper",
        "dragon", "wither_skeleton", "piglin", "custom_head",
    ];
    instruments.iter().map(|i| format!("jade.instrument.{i}")).collect()
}

/// 代码里拼接、但由**游戏自己的**语言文件提供的键：药水效果、附魔等级、村民职业与等级、
/// 画、流体。这些不能抄进 Jade 的语言文件，否则写死的旧译文会遮蔽游戏随版本更新的翻译；
/// 这里只确认 Jade 没有把它们抄进来。
fn game_keys() -> Vec<String> {
    [
        "effect.minecraft.speed",
        "enchantment.level.1",
        "entity.minecraft.villager.librarian",
        "merchant.level.1",
        "painting.minecraft.kebab.title",
        "block.minecraft.water",
    ]
    .iter()
    .map(|key| (*key).to_owned())
    .collect()
}

#[test]
fn every_used_key_exists() {
    let used: Vec<String> = keys_in_source().into_iter().chain(dynamic_keys()).collect();
    assert!(used.len() > 15, "源码扫描到的键太少，扫描逻辑可能坏了：{used:?}");
    for lang in ["en_us", "zh_cn"] {
        let have = keys_in(lang);
        let mut missing: Vec<_> = used.iter().filter(|k| !have.contains(*k)).collect();
        missing.sort();
        assert!(missing.is_empty(), "{lang}.json 缺少键：{missing:?}");
    }
}

#[test]
fn game_keys_are_not_shadowed() {
    for lang in ["en_us", "zh_cn"] {
        let have = keys_in(lang);
        let mut shadowed: Vec<_> = game_keys().into_iter().filter(|k| have.contains(k)).collect();
        shadowed.sort();
        assert!(shadowed.is_empty(), "{lang}.json 不该抄游戏提供的键：{shadowed:?}");
    }
}
