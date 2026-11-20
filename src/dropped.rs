//! 掉落物：原版准星不会瞄到掉落物实体（它不可拾取），所以自己沿视线做射线检测，
//! 取触及距离内最近的一个，并且要比准星命中的方块或实体更近。
use mod_api::{ClientHost, CrosshairTarget, EntityRef};

use crate::fluid::{direction, distance, eye, reach};

/// 射线与轴对齐盒的进入距离（slab 法）；射线起点在盒内时为 0，不相交返回 None。
pub fn ray_box(origin: [f64; 3], dir: [f64; 3], min: [f64; 3], max: [f64; 3]) -> Option<f64> {
    let (mut near, mut far) = (0.0_f64, f64::INFINITY);
    for axis in 0..3 {
        if dir[axis].abs() < 1e-12 {
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
            continue;
        }
        let (a, b) = ((min[axis] - origin[axis]) / dir[axis], (max[axis] - origin[axis]) / dir[axis]);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
        if near > far {
            return None;
        }
    }
    Some(near)
}

/// 视线上的掉落物，以“准星命中的实体”的形式返回，后续走实体提示框与服务端数据。
pub fn target(host: &dyn ClientHost) -> Option<CrosshairTarget> {
    let camera = host.camera()?;
    let level = host.local_player()?.level(host)?;
    let origin = eye(host).unwrap_or(camera.position);
    let dir = direction(camera.yaw, camera.pitch);
    let mut limit = reach(host);
    // 准星命中的方块或实体比掉落物更近时，掉落物被挡住。
    match host.crosshair_target() {
        Some(CrosshairTarget::Block { pos, .. }) => {
            let min = [f64::from(pos.x), f64::from(pos.y), f64::from(pos.z)];
            if let Some(d) = ray_box(origin, dir, min, [min[0] + 1.0, min[1] + 1.0, min[2] + 1.0]) {
                limit = limit.min(d);
            }
        }
        Some(CrosshairTarget::Entity { point, .. }) => limit = limit.min(distance(origin, point)),
        _ => {}
    }
    let end = [origin[0] + dir[0] * limit, origin[1] + dir[1] * limit, origin[2] + dir[2] * limit];
    let pad = 0.5;
    let low = [origin[0].min(end[0]) - pad, origin[1].min(end[1]) - pad, origin[2].min(end[2]) - pad];
    let high = [origin[0].max(end[0]) + pad, origin[1].max(end[1]) + pad, origin[2].max(end[2]) + pad];
    let mut best: Option<(f64, EntityRef)> = None;
    let dropped_kind = |e: EntityRef| e.kind(host).is_some_and(|k| k.id(host) == "minecraft:item");
    for entity in level.entities_in(host, low, high)? {
        let Some([min, max]) = entity.bounding_box(host) else { continue };
        // 实体盒放宽一点：掉落物的碰撞箱只有 0.25 格，原版也是按放宽后的盒子拾取瞄准。
        let grow = 0.1;
        let d = ray_box(
            origin,
            dir,
            [min[0] - grow, min[1] - grow, min[2] - grow],
            [max[0] + grow, max[1] + grow, max[2] + grow],
        );
        if let Some(d) = d.filter(|d| *d <= limit) {
            if best.as_ref().is_none_or(|(b, _)| d < *b) {
                if dropped_kind(entity) {
                    best = Some((d, entity));
                }
            }
        }
    }
    let (d, entity) = best?;
    let point = [origin[0] + dir[0] * d, origin[1] + dir[1] * d, origin[2] + dir[2] * d];
    Some(CrosshairTarget::Entity { entity, point })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_box_in_front_only() {
        let o = [0.0, 0.0, 0.0];
        let d = ray_box(o, [0.0, 0.0, 1.0], [-0.5, -0.5, 2.0], [0.5, 0.5, 3.0]).unwrap();
        assert!((d - 2.0).abs() < 1e-9);
        assert!(ray_box(o, [0.0, 0.0, -1.0], [-0.5, -0.5, 2.0], [0.5, 0.5, 3.0]).is_none());
        assert!(ray_box(o, [0.0, 0.0, 1.0], [1.0, -0.5, 2.0], [2.0, 0.5, 3.0]).is_none());
    }

    #[test]
    fn origin_inside_box_is_zero() {
        assert_eq!(ray_box([0.5; 3], [1.0, 0.0, 0.0], [0.0; 3], [1.0; 3]), Some(0.0));
    }
}
