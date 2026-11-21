//! 隐藏列表：`config/jade/hide.txt` 里列出的方块、实体 id 不显示提示框。
//!
//! 每行一个条目：完整 id（`minecraft:stone`）、整个命名空间（`create:*`）或以 `#` 开头的注释。
//! 文件不存在时写出一份带说明的模板。运行中改文件，几秒内自动重读，不需要重启。
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use mod_api::{ClientHost, ConfigPath};

/// 重读文件的间隔（毫秒）。
const RELOAD_MS: u64 = 5_000;
/// 配置根目录下只有 `jade/` 文件夹与 `jade.*` 文件属于本 mod，所以带上文件夹名。
const FILE: &str = "jade/hide.txt";
const TEMPLATE: &str = "# Jade for Horizon 隐藏列表 / hide list\n\
# 每行一个条目，不显示提示框 / one entry per line, no tooltip is shown for it:\n\
#   minecraft:barrier   完整 id / a full id\n\
#   create:*            整个命名空间 / a whole namespace\n\
# 以 # 开头的行是注释 / lines starting with # are comments\n";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HideList {
    ids: HashSet<String>,
    namespaces: HashSet<String>,
}

impl HideList {
    /// 解析文件内容：忽略空行与注释，大小写不敏感。
    pub fn parse(text: &str) -> HideList {
        let mut list = HideList::default();
        for line in text.lines() {
            let line = line.trim().to_ascii_lowercase();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match line.strip_suffix(":*") {
                Some(ns) if !ns.is_empty() => {
                    list.namespaces.insert(ns.to_owned());
                }
                _ => {
                    // 没写命名空间的按 minecraft 算。
                    let id = if line.contains(':') { line } else { format!("minecraft:{line}") };
                    list.ids.insert(id);
                }
            }
        }
        list
    }

    pub fn hides(&self, id: &str) -> bool {
        if self.ids.contains(id) {
            return true;
        }
        id.split_once(':').is_some_and(|(ns, _)| self.namespaces.contains(ns))
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty() && self.namespaces.is_empty()
    }
}

struct Cache {
    list: HideList,
    loaded_at: Option<u64>,
}

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE
        .get_or_init(|| Mutex::new(Cache { list: HideList::default(), loaded_at: None }))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// 当前隐藏列表；到点了就重读文件。读不到文件时（含第一次）写出模板、按空列表处理。
pub fn current(host: &dyn ClientHost) -> HideList {
    let now = host.millis();
    let mut cache = cache();
    if cache.loaded_at.is_some_and(|t| now.saturating_sub(t) < RELOAD_MS) {
        return cache.list.clone();
    }
    cache.loaded_at = Some(now);
    if let Ok(path) = ConfigPath::new(FILE) {
        let files = host.config_files();
        match files.read(&path) {
            Ok(bytes) => cache.list = HideList::parse(&String::from_utf8_lossy(&bytes)),
            Err(_) => {
                cache.list = HideList::default();
                let _ = files.write(&path, TEMPLATE.as_bytes());
            }
        }
    }
    cache.list.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ids_namespaces_and_comments() {
        let list = HideList::parse("# 注释\n\nMinecraft:Barrier\ncreate:*\n  stone  \n:*\n");
        assert!(list.hides("minecraft:barrier"));
        assert!(list.hides("minecraft:stone"));
        assert!(list.hides("create:shaft"));
        assert!(!list.hides("minecraft:dirt"));
        assert!(!list.hides("other:shaft"));
    }

    #[test]
    fn template_hides_nothing() {
        assert!(HideList::parse(TEMPLATE).is_empty());
    }
}
