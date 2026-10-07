//! One continuous-corner boundary for the renderer, native window and shadow.
//!
//! Each corner is a fourth-order superellipse joined to straight edges. Its
//! curvature approaches zero at those joins, unlike a circular rounded rect.

const CORNER_SEGMENTS: usize = 40;

#[derive(Clone, Debug)]
pub struct ContinuousRect {
    size: [f32; 2],
    radius: f32,
    corner: Vec<[f32; 2]>,
    points: Vec<[f32; 2]>,
}

impl ContinuousRect {
    pub fn new(size: [f32; 2], radius: f32) -> Self {
        let finite = |value: f32| if value.is_finite() { value.max(0.) } else { 0. };
        let size = [finite(size[0]), finite(size[1])];
        let radius = finite(radius).min(size[0].min(size[1]) * 0.5);
        let mut corner = Vec::new();
        let mut points = Vec::new();
        if size[0] > 0. && size[1] > 0. {
            if radius == 0. {
                points.extend([[0., 0.], [size[0], 0.], size, [0., size[1]]]);
            } else {
                for index in 0..=CORNER_SEGMENTS {
                    let angle = std::f32::consts::FRAC_PI_2 * index as f32 / CORNER_SEGMENTS as f32;
                    corner.push([
                        radius * angle.cos().max(0.).sqrt(),
                        radius * angle.sin().max(0.).sqrt(),
                    ]);
                }
                // Exact endpoints avoid trigonometric rounding at the joins.
                corner[0] = [radius, 0.];
                corner[CORNER_SEGMENTS] = [0., radius];
                for (center, signs) in [
                    ([size[0] - radius, radius], [1., -1.]),
                    ([size[0] - radius, size[1] - radius], [1., 1.]),
                    ([radius, size[1] - radius], [-1., 1.]),
                    ([radius, radius], [-1., -1.]),
                ] {
                    let reverse = signs[0] * signs[1] < 0.;
                    for index in 0..=CORNER_SEGMENTS {
                        let point = corner[if reverse {
                            CORNER_SEGMENTS - index
                        } else {
                            index
                        }];
                        let point = [
                            center[0] + signs[0] * point[0],
                            center[1] + signs[1] * point[1],
                        ];
                        if points.last() != Some(&point) {
                            points.push(point);
                        }
                    }
                }
                if points.first() == points.last() {
                    points.pop();
                }
            }
        }
        Self {
            size,
            radius,
            corner,
            points,
        }
    }

    /// Clockwise convex boundary in local coordinates, without a duplicate
    /// closing point. Translate by the GUI rectangle's origin before painting.
    pub fn points(&self) -> &[[f32; 2]] {
        &self.points
    }

    pub fn contains(&self, point: [f32; 2]) -> bool {
        if self.points.is_empty()
            || point[0] < 0.
            || point[1] < 0.
            || point[0] > self.size[0]
            || point[1] > self.size[1]
        {
            return false;
        }
        let q = self.corner_coordinates(point);
        q[0] <= 0.
            || q[1] <= 0.
            || (q[0] / self.radius).powi(4) + (q[1] / self.radius).powi(4) <= 1.
    }

    /// Euclidean distance to the sampled boundary, negative inside. Straight
    /// edges take a constant-time path; only corner pixels examine the samples.
    pub fn signed_distance(&self, point: [f32; 2]) -> f32 {
        if self.points.is_empty() {
            return f32::INFINITY;
        }
        let q = self.corner_coordinates(point);
        let edge = [q[0] - self.radius, q[1] - self.radius];
        if self.radius == 0. {
            return edge[0].max(0.).hypot(edge[1].max(0.)) + edge[0].max(edge[1]).min(0.);
        }
        if q[0] <= 0. || q[1] <= 0. {
            return edge[0].max(edge[1]);
        }
        let distance = self
            .corner
            .windows(2)
            .map(|segment| {
                let a = segment[0];
                let b = segment[1];
                let delta = [b[0] - a[0], b[1] - a[1]];
                let length_squared = delta[0] * delta[0] + delta[1] * delta[1];
                let t = (((q[0] - a[0]) * delta[0] + (q[1] - a[1]) * delta[1]) / length_squared)
                    .clamp(0., 1.);
                (q[0] - a[0] - delta[0] * t).hypot(q[1] - a[1] - delta[1] * t)
            })
            .fold(f32::INFINITY, f32::min);
        if self.contains(point) {
            -distance
        } else {
            distance
        }
    }

    fn corner_coordinates(&self, point: [f32; 2]) -> [f32; 2] {
        [
            (point[0] - self.size[0] * 0.5).abs() - (self.size[0] * 0.5 - self.radius),
            (point[1] - self.size[1] * 0.5).abs() - (self.size[1] * 0.5 - self.radius),
        ]
    }
}

pub fn continuous_rect(size: [f32; 2], radius: f32) -> Vec<[f32; 2]> {
    ContinuousRect::new(size, radius).points
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_is_convex_symmetric_and_follows_all_four_edges() {
        let shape = ContinuousRect::new([440., 600.], 22.);
        let path = shape.points();
        for index in 0..path.len() {
            let a = path[index];
            let b = path[(index + 1) % path.len()];
            let c = path[(index + 2) % path.len()];
            let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
            assert!(cross >= -0.001, "boundary must stay convex");
            assert!((0. ..=440.).contains(&a[0]) && (0. ..=600.).contains(&a[1]));
            let reflected = [440. - a[0], 600. - a[1]];
            assert!(path.iter().any(
                |p| (p[0] - reflected[0]).abs() < 0.001 && (p[1] - reflected[1]).abs() < 0.001
            ));
            assert!(shape.signed_distance(a).abs() < 0.001);
        }
        assert!(shape.contains([220., 0.]));
        assert!(shape.contains([0., 300.]));
        assert!(!shape.contains([0., 0.]));
        assert!(!shape.contains([440., 600.]));
    }

    #[test]
    fn straight_edges_join_continuously_and_shadow_distance_has_correct_sign() {
        let shape = ContinuousRect::new([440., 600.], 22.);
        assert!(shape.signed_distance([220., 300.]) < 0.);
        assert!((shape.signed_distance([450., 300.]) - 10.).abs() < 0.001);
        let a = shape.corner[0];
        let b = shape.corner[1];
        // At a straight-edge join the curve bends gradually instead of starting
        // a circle's nonzero curvature immediately.
        assert!((a[0] - b[0]).abs() / (b[1] - a[1]).abs() < 0.01);
        let scaled = ContinuousRect::new([880., 1200.], 44.);
        assert!((scaled.signed_distance([900., 600.]) - 20.).abs() < 0.001);
    }

    #[test]
    fn pill_and_extreme_radius_remain_valid() {
        let pill = ContinuousRect::new([440., 72.], 200.);
        assert!(pill.contains([220., 36.]));
        assert!(!pill.contains([1., 1.]));
        assert_eq!(continuous_rect([100., 60.], 0.).len(), 4);
        assert!(continuous_rect([0., 60.], 20.).is_empty());
        assert!(continuous_rect([f32::NAN, 60.], 20.).is_empty());
    }
}
