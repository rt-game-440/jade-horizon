//! 服务端按玩家限流与回包去重。
//!
//! 正常客户端每 tick 至多发一个请求（目标变化时立即发，同一目标定期刷新），
//! 所以每个窗口放行的数量留有余量；超出的请求直接丢弃，避免改过的客户端刷包拖慢服务端。
//! 同一目标的数据没有变化时不重复回包，除非客户端标明这是新目标的首个请求。
use std::collections::HashMap;

use crate::net::ServerData;

/// 限流窗口长度（游戏 tick）。
pub const WINDOW: i64 = 20;
/// 每个窗口内每名玩家最多处理的请求数。客户端正常上限约为每 tick 一个，这里留出余量。
pub const MAX_PER_WINDOW: u32 = 30;

#[derive(Default)]
struct PlayerState {
    window_start: i64,
    count: u32,
    /// 上次回给该玩家的数据，用于去重。
    last: Option<ServerData>,
}

#[derive(Default)]
pub struct Limiter {
    players: HashMap<u128, PlayerState>,
}

impl Limiter {
    /// 是否处理这个请求。`now` 为当前游戏时间（tick）。
    pub fn admit(&mut self, player: u128, now: i64) -> bool {
        let st = self.players.entry(player).or_default();
        // 时间不应倒退；万一倒退（读到异常值）也开新窗口，免得玩家被一直挡住。
        if now - st.window_start >= WINDOW || now < st.window_start {
            st.window_start = now;
            st.count = 0;
        }
        if st.count >= MAX_PER_WINDOW {
            // 丢弃了请求：清掉去重记录，保证下一个被放行的请求一定回包，客户端不会一直等不到数据。
            st.last = None;
            return false;
        }
        st.count += 1;
        true
    }

    /// 是否需要把 `data` 发给玩家；需要时记下它。`fresh` 表示客户端刚换了目标。
    pub fn should_send(&mut self, player: u128, fresh: bool, data: &ServerData) -> bool {
        let st = self.players.entry(player).or_default();
        if !fresh && st.last.as_ref() == Some(data) {
            return false;
        }
        st.last = Some(data.clone());
        true
    }

    /// 玩家离开时清掉状态。
    pub fn forget(&mut self, player: u128) {
        self.players.remove(&player);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.players.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{KIND_BLOCK, NetItem, NetTarget};

    fn data(x: i32, count: i32) -> ServerData {
        ServerData {
            target: NetTarget { kind: KIND_BLOCK, x, ..Default::default() },
            items: vec![NetItem { id: "minecraft:diamond".into(), count }],
            ..Default::default()
        }
    }

    #[test]
    fn caps_requests_per_window() {
        let mut l = Limiter::default();
        let admitted = (0..100).filter(|_| l.admit(1, 0)).count();
        assert_eq!(admitted, MAX_PER_WINDOW as usize);
        // 其他玩家不受影响。
        assert!(l.admit(2, 0));
        // 下一个窗口恢复。
        assert!(l.admit(1, WINDOW));
    }

    #[test]
    fn normal_client_rate_never_dropped() {
        // 每 tick 一个请求，持续 10 秒。
        let mut l = Limiter::default();
        assert!((0..200).all(|t| l.admit(1, t)));
    }

    #[test]
    fn dedups_unchanged_data_unless_fresh() {
        let mut l = Limiter::default();
        assert!(l.should_send(1, true, &data(0, 7)));
        assert!(!l.should_send(1, false, &data(0, 7)));
        assert!(l.should_send(1, false, &data(0, 8)), "内容变化要发");
        assert!(l.should_send(1, false, &data(5, 8)), "目标变化要发");
        assert!(l.should_send(1, true, &data(5, 8)), "新目标首个请求总要发");
    }

    #[test]
    fn dropped_request_resets_dedup() {
        let mut l = Limiter::default();
        assert!(l.should_send(1, true, &data(0, 7)));
        while l.admit(1, 0) {}
        assert!(l.admit(1, WINDOW));
        assert!(l.should_send(1, false, &data(0, 7)), "丢过请求后下一次必须回包");
    }

    #[test]
    fn forget_clears_player() {
        let mut l = Limiter::default();
        l.admit(1, 0);
        l.forget(1);
        assert_eq!(l.len(), 0);
    }
}
