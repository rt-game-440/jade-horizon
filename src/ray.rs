//! 视线射线与几何小工具：眼睛位置、视线方向、触及距离，以及沿视线逐格前进（Amanatides & Woo）。
//!
//! 原版准星忽略流体和掉落物，`fluid`、`dropped` 用这里的函数自己做射线检测。
use mod_api::ClientHost;

/// 射线最多检查多少格：原版触及距离不到 6 格，留足余量防止死循环。
const MAX_CELLS: usize = 64;

/// 读不到 `minecraft:block_interaction_range` 时的触及距离。
const DEFAULT_REACH: f64 = 4.5;

/// 眼睛位置：优先用相机实体的眼睛，读不到时退回相机位置。
pub fn eye(host: &dyn ClientHost) -> Option<[f64; 3]> {
    if let Some(position) = host.camera_entity().and_then(|entity| entity.eye_position(host)) {
        return Some(position);
    }
    host.camera().map(|camera| camera.position)
}

pub fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// 视线方向单位向量（Minecraft 约定：yaw 0 朝 +Z，pitch 为负向上看）。
pub fn direction(yaw: f32, pitch: f32) -> [f64; 3] {
    let (yaw, pitch) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    let horizontal = pitch.cos();
    [-yaw.sin() * horizontal, -pitch.sin(), yaw.cos() * horizontal]
}

/// 玩家的方块触及距离（格）。
pub fn reach(host: &dyn ClientHost) -> f64 {
    host.local_player()
        .and_then(|player| host.living(player.entity()))
        .and_then(|living| living.attribute_value(host, "minecraft:block_interaction_range"))
        .unwrap_or(DEFAULT_REACH)
}

/// 射线依次经过的格子与进入每格的射线距离（第一格的距离为 0）。
/// 走到 `until` 那格就停（该格仍会出现在结果里）；超过 `max` 或 `MAX_CELLS` 也停。
pub fn cells(origin: [f64; 3], dir: [f64; 3], max: f64, until: Option<[i32; 3]>) -> Vec<([i32; 3], f64)> {
    let mut out = Vec::new();
    let mut pos = [origin[0].floor() as i32, origin[1].floor() as i32, origin[2].floor() as i32];
    let mut step = [0; 3];
    let mut next = [f64::INFINITY; 3];
    let mut delta = [f64::INFINITY; 3];
    for axis in 0..3 {
        if dir[axis] > 0.0 {
            step[axis] = 1;
            next[axis] = ((f64::from(pos[axis]) + 1.0) - origin[axis]) / dir[axis];
            delta[axis] = 1.0 / dir[axis];
        } else if dir[axis] < 0.0 {
            step[axis] = -1;
            next[axis] = (f64::from(pos[axis]) - origin[axis]) / dir[axis];
            delta[axis] = -1.0 / dir[axis];
        }
    }
    let mut distance = 0.0;
    loop {
        out.push((pos, distance));
        if until == Some(pos) || out.len() >= MAX_CELLS {
            break;
        }
        let axis = if next[0] <= next[1] && next[0] <= next[2] {
            0
        } else if next[1] <= next[2] {
            1
        } else {
            2
        };
        if next[axis] > max {
            break;
        }
        distance = next[axis];
        next[axis] += delta[axis];
        pos[axis] += step[axis];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn positions(cells: &[([i32; 3], f64)]) -> Vec<[i32; 3]> {
        cells.iter().map(|(pos, _)| *pos).collect()
    }

    #[test]
    fn steps_through_one_axis() {
        let hit = cells([0.5, 0.5, 0.5], [0.0, 0.0, 1.0], 2.5, None);
        assert_eq!(
            positions(&hit),
            vec![[0, 0, 0], [0, 0, 1], [0, 0, 2], [0, 0, 3]]
        );
        // 进入每格的距离就是到该格近面的距离。
        assert_eq!(hit[0].1, 0.0);
        assert!((hit[2].1 - 1.5).abs() < 1e-9);
        // 恰好在 max 上的那一格仍会被检查，再远的不再走。
        assert!((hit[3].1 - 2.5).abs() < 1e-9);
    }

    #[test]
    fn stops_at_the_crosshair_block() {
        let hit = cells([0.5, 0.5, 0.5], [0.0, 0.0, -1.0], 5.0, Some([0, 0, -1]));
        assert_eq!(positions(&hit), vec![[0, 0, 0], [0, 0, -1]]);
    }

    #[test]
    fn steps_diagonally_axis_by_axis() {
        // 45° 斜看：每格都先过一个轴，不会跳过格子。
        let hit = cells([0.5, 0.5, 0.5], [0.7071067811865476, 0.0, 0.7071067811865476], 3.0, None);
        let pos: Vec<_> = positions(&hit);
        assert_eq!(pos[0], [0, 0, 0]);
        assert_eq!(pos[1], [1, 0, 0]);
        assert_eq!(pos[2], [1, 0, 1]);
        assert_eq!(pos[3], [2, 0, 1]);
        // 每格距离单调递增，且都是 0.7 左右。
        for pair in hit.windows(2) {
            assert!(pair[1].1 >= pair[0].1);
        }
        assert!((hit[1].1 - 0.70710678).abs() < 1e-6);
    }

    #[test]
    fn looks_up_and_down() {
        let up = cells([0.5, 0.5, 0.5], direction(0.0, -90.0), 1.0, None);
        assert_eq!(positions(&up)[1], [0, 1, 0]);
        let down = cells([0.5, 0.5, 0.5], direction(0.0, 90.0), 1.0, None);
        assert_eq!(positions(&down)[1], [0, -1, 0]);
        // yaw 0 朝 +Z（南），yaw 90 朝 -X（西）。
        assert!((direction(0.0, 0.0)[2] - 1.0).abs() < 1e-6);
        assert!((direction(90.0, 0.0)[0] + 1.0).abs() < 1e-6);
    }
}
