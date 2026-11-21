//! 流体信息：原版准星忽略流体，这里自己做一条视线射线，找到视线上的第一个非空流体。
//!
//! 从相机（眼睛）沿视线逐格前进（见 `ray::cells`），每格问一次流体状态：
//!
//! - 相机所在的格不算，泡在流体里时连着的那种流体也不算，免得游泳时满脸是“水”；
//! - 准星命中的方块格不算，所以含水楼梯、海草这类方块仍显示方块信息；
//! - 不向服务端要数据。
use mod_api::{ClientHost, CrosshairTarget, LevelRef};

use crate::ray::{cells, direction, eye, reach};

/// 视线上的第一个流体。
#[derive(Clone, Debug, PartialEq)]
pub struct Fluid {
    /// 流体 id，`flowing_` 前缀已去掉（`minecraft:water`）。
    pub id: String,
    /// 从眼睛到进入该格的距离（格）。
    pub distance: f64,
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn strips_flowing_prefix() {
        assert_eq!(normalize("minecraft:flowing_water"), "minecraft:water");
        assert_eq!(normalize("minecraft:water"), "minecraft:water");
        assert_eq!(normalize("mymod:flowing_acid"), "mymod:acid");
        assert_eq!(normalize("flowing_lava"), "lava");
    }
}
