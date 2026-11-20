//! 流体信息：原版准星忽略流体，这里自己做一条视线射线，找到视线上的第一个非空流体。
//!
//! 从相机（眼睛）沿视线逐格前进（Amanatides & Woo），每格问一次流体状态：
//!
//! - 相机所在的格不算，泡在流体里时连着的那种流体也不算，免得游泳时满脸是“水”；
//! - 准星命中的方块格不算，所以含水楼梯、海草这类方块仍显示方块信息；
//! - 不向服务端要数据。
use mod_api::{ClientHost, CrosshairTarget, LevelRef};

/// 射线最多检查多少格：原版触及距离不到 6 格，留足余量防止死循环。
const MAX_CELLS: usize = 64;

/// 读不到 `minecraft:block_interaction_range` 时的触及距离。
const DEFAULT_REACH: f64 = 4.5;

/// 视线上的第一个流体。
#[derive(Clone, Debug, PartialEq)]
pub struct Fluid {
    /// 流体 id，`flowing_` 前缀已去掉（`minecraft:water`）。
    pub id: String,
    /// 从眼睛到进入该格的距离（格）。
    pub distance: f64,
}

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

/// 视线上的第一个流体；没看到流体时返回 None。
pub fn target(host: &dyn ClientHost) -> Option<Fluid> {
    let camera = host.camera()?;
    let player = host.local_player()?;
    let level: LevelRef = player.level(host)?;
    let eye = eye(host).unwrap_or(camera.position);
    let dir = direction(camera.yaw, camera.pitch);
    let reach = reach(host);
    let until = match host.crosshair_target() {
        Some(CrosshairTarget::Block { pos, .. }) => Some([pos.x, pos.y, pos.z]),
        _ => None,
    };
    // 相机泡在流体里时，一路连着的同种流体都跳过。
    let mut submerged = None;
    for (pos, distance) in cells(eye, dir, reach, until) {
        if until == Some(pos) {
            break;
        }
        let Some(state) = level.fluid_state(host, pos) else {
            // 未加载的格：后面的也读不到，直接放弃。
            break;
        };
        let id = (!state.is_empty()).then(|| normalize(&state.fluid.id(host)));
        if decide(id.as_deref(), distance == 0.0, &mut submerged) {
            return Some(Fluid { id: id.unwrap_or_default(), distance });
        }
    }
    None
}

/// 这一格里的流体要不要当成“看到的流体”。
///
/// `id` 为 None 表示没有流体；`at_eye` 表示这是相机所在的格。
/// 相机所在格的流体只记下来（`submerged`），之后连着的同种流体都不算，
/// 碰到别的流体或空流体就恢复正常判定。
fn decide(id: Option<&str>, at_eye: bool, submerged: &mut Option<String>) -> bool {
    match id {
        None => {
            *submerged = None;
            false
        }
        Some(id) if at_eye => {
            *submerged = Some(id.to_owned());
            false
        }
        Some(id) => submerged.as_deref() != Some(id),
    }
}

/// `minecraft:flowing_water` → `minecraft:water`。
fn normalize(id: &str) -> String {
    match id.split_once(':') {
        Some((namespace, path)) => {
            let path = path.strip_prefix("flowing_").unwrap_or(path);
            format!("{namespace}:{path}")
        }
        None => id.strip_prefix("flowing_").unwrap_or(id).to_owned(),
    }
}

pub fn reach(host: &dyn ClientHost) -> f64 {
    host.local_player()
        .and_then(|player| host.living(player.entity()))
        .and_then(|living| living.attribute_value(host, "minecraft:block_interaction_range"))
        .unwrap_or(DEFAULT_REACH)
}

/// 射线依次经过的格子与进入每格的射线距离（第一格的距离为 0）。
/// 走到 `until` 那格就停（该格仍会出现在结果里）；超过 `max` 或 `MAX_CELLS` 也停。
fn cells(origin: [f64; 3], dir: [f64; 3], max: f64, until: Option<[i32; 3]>) -> Vec<([i32; 3], f64)> {
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

    #[test]
    fn skips_the_fluid_the_camera_is_in() {
        let mut submerged = None;
        // 相机所在格的水：先记下来，不当成目标。
        assert!(!decide(Some("minecraft:water"), true, &mut submerged));
        assert_eq!(submerged.as_deref(), Some("minecraft:water"));
        // 连着的同种水继续跳过。
        assert!(!decide(Some("minecraft:water"), false, &mut submerged));
        // 另一种流体照常显示。
        assert!(decide(Some("minecraft:lava"), false, &mut submerged));
        // 空流体说明离开水面，之后的水又算目标。
        assert!(!decide(None, false, &mut submerged));
        assert!(decide(Some("minecraft:water"), false, &mut submerged));
    }

    #[test]
    fn fluid_in_the_camera_cell_alone_is_not_shown() {
        let mut submerged = None;
        // 相机站在水里但视线离开水面：一格也不命中。
        assert!(!decide(Some("minecraft:water"), true, &mut submerged));
        assert!(!decide(None, false, &mut submerged));
    }

    #[test]
    fn strips_flowing_prefix() {        assert_eq!(normalize("minecraft:flowing_water"), "minecraft:water");
        assert_eq!(normalize("minecraft:water"), "minecraft:water");
        assert_eq!(normalize("mymod:flowing_acid"), "mymod:acid");
        assert_eq!(normalize("flowing_lava"), "lava");
    }
}
