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

/// 拼接出来的键：音符盒乐器、比较器模式等。
fn dynamic_keys() -> Vec<String> {
    let instruments = [
        "harp", "basedrum", "snare", "hat", "bass", "flute", "bell", "guitar", "chime", "xylophone",
        "iron_xylophone", "cow_bell", "didgeridoo", "bit", "banjo", "pling", "zombie", "skeleton", "creeper",
        "dragon", "wither_skeleton", "piglin", "custom_head",
    ];
    instruments.iter().map(|i| format!("jade.instrument.{i}")).collect()
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
