//! 网络用量统计：给性能基线（`tools/perf.py`）用。只有设置里打开“调试模式”才会写文件。
use std::sync::atomic::{AtomicU64, Ordering};

use mod_api::{ClientHost, ConfigPath};

static REQUESTS: AtomicU64 = AtomicU64::new(0);
static REPLIES: AtomicU64 = AtomicU64::new(0);
static BYTES_OUT: AtomicU64 = AtomicU64::new(0);
static BYTES_IN: AtomicU64 = AtomicU64::new(0);
static LAST_FLUSH_MS: AtomicU64 = AtomicU64::new(0);

/// 每隔多久写一次文件。
const FLUSH_EVERY_MS: u64 = 10_000;

/// 发出了一个请求。
pub fn record_request(bytes: usize) {
    REQUESTS.fetch_add(1, Ordering::Relaxed);
    BYTES_OUT.fetch_add(bytes as u64, Ordering::Relaxed);
}

/// 收到了一个回包。
pub fn record_reply(bytes: usize) {
    REPLIES.fetch_add(1, Ordering::Relaxed);
    BYTES_IN.fetch_add(bytes as u64, Ordering::Relaxed);
}

/// 计数器写成 JSON 文本（累计值加时间戳，由读的一方自己做差）。
fn snapshot(now_ms: u64) -> String {
    format!(
        "{{\"now_ms\":{now_ms},\"requests\":{},\"replies\":{},\"bytes_out\":{},\"bytes_in\":{}}}\n",
        REQUESTS.load(Ordering::Relaxed),
        REPLIES.load(Ordering::Relaxed),
        BYTES_OUT.load(Ordering::Relaxed),
        BYTES_IN.load(Ordering::Relaxed),
    )
}

/// 每个客户端 tick 调用：打开了调试统计且到点了，就把计数器写进 `horizon-mod-data/jade/perf.json`。
pub fn flush(host: &dyn ClientHost, enabled: bool) {
    if !enabled {
        return;
    }
    let now = host.millis();
    let last = LAST_FLUSH_MS.load(Ordering::Relaxed);
    if now.saturating_sub(last) < FLUSH_EVERY_MS {
        return;
    }
    LAST_FLUSH_MS.store(now, Ordering::Relaxed);
    if let Ok(path) = ConfigPath::new("perf.json") {
        let _ = host.data_files().write(&path, snapshot(now).as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_is_json_with_all_counters() {
        record_request(18);
        record_reply(100);
        let text = snapshot(5);
        assert!(text.starts_with("{\"now_ms\":5,\"requests\":"));
        for key in ["replies", "bytes_out", "bytes_in"] {
            assert!(text.contains(&format!("\"{key}\":")), "{key}");
        }
        assert!(text.ends_with("}\n"));
    }
}
