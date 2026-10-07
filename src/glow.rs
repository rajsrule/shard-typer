use eframe::egui::{Color32, Painter, Pos2, Shape, Stroke, Vec2, epaint::Mesh};

/// Paint a rounded stroke surrounded by a smoothly feathered halo.
/// `radius` is the halo's reach beyond either edge of the core stroke.
/// Leave that much room in the painter's clip rect, including past the endpoints.
pub fn stroke(painter: &Painter, points: &[Pos2], color: Color32, width: f32, radius: f32) {
    if !width.is_finite() || width <= 0.0 || color.a() == 0 {
        return;
    }
    let points = simplify(points, (0.8 / painter.ctx().pixels_per_point()).max(0.2));
    if points.is_empty() {
        return;
    }
    let half_width = width * 0.5;
    if radius.is_finite() && radius > 0.0 {
        let steps = ((radius * painter.ctx().pixels_per_point()).ceil() as usize).clamp(20, 48);
        let mut mesh = Mesh::default();
        if points.len() == 1 {
            cap(
                &mut mesh,
                points[0],
                0.0,
                std::f32::consts::TAU,
                half_width,
                radius,
                steps,
                color,
            );
        } else {
            ribbon(&mut mesh, &points, half_width, radius, steps, color);
            let first_direction = (points[1] - points[0]).normalized();
            let last_direction = (points[points.len() - 1] - points[points.len() - 2]).normalized();
            cap(
                &mut mesh,
                points[0],
                first_direction.angle() + std::f32::consts::FRAC_PI_2,
                std::f32::consts::PI,
                half_width,
                radius,
                steps,
                color,
            );
            cap(
                &mut mesh,
                points[points.len() - 1],
                last_direction.angle() - std::f32::consts::FRAC_PI_2,
                std::f32::consts::PI,
                half_width,
                radius,
                steps,
                color,
            );
        }
        painter.add(Shape::mesh(mesh));
    }
    if points.len() > 1 {
        painter.add(Shape::line(points.clone(), Stroke::new(width, color)));
    }
    painter.circle_filled(points[0], half_width, color);
    if points.len() > 1 {
        painter.circle_filled(points[points.len() - 1], half_width, color);
    }
}

fn simplify(points: &[Pos2], spacing: f32) -> Vec<Pos2> {
    let mut result = Vec::with_capacity(points.len().min(1024));
    let mut last = None;
    for &point in points.iter().filter(|point| point.is_finite()) {
        last = Some(point);
        if result
            .last()
            .is_none_or(|previous: &Pos2| previous.distance_sq(point) >= spacing * spacing)
        {
            result.push(point);
        }
    }
    if let Some(last) = last
        && result
            .last()
            .is_some_and(|previous| previous.distance_sq(last) > 0.0001)
    {
        result.push(last);
    }
    result
}

fn feather(color: Color32, distance: f32, half_width: f32, radius: f32) -> Color32 {
    let normalized = ((distance - half_width) / radius).clamp(0.0, 1.0);
    // The tail reaches exactly zero. Interpolated mesh colors supply a continuous
    // gradient instead of the visible bands left by stacked translucent strokes.
    let cutoff = (-4.5_f32).exp();
    let weight = ((-4.5 * normalized * normalized).exp() - cutoff) / (1.0 - cutoff);
    let alpha = (f32::from(color.a()) * 0.36 * weight).round() as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn ribbon(
    mesh: &mut Mesh,
    points: &[Pos2],
    half_width: f32,
    radius: f32,
    steps: usize,
    color: Color32,
) {
    let extent = half_width + radius;
    let columns = steps * 2 + 1;
    let base = mesh.vertices.len() as u32;
    for (index, &point) in points.iter().enumerate() {
        let before = index
            .checked_sub(1)
            .map(|i| (point - points[i]).normalized());
        let after = points
            .get(index + 1)
            .map(|&next| (next - point).normalized());
        let direction = match (before, after) {
            (Some(before), Some(after)) if (before + after).length_sq() > 0.0001 => {
                (before + after).normalized()
            }
            (Some(before), _) => before,
            (_, Some(after)) => after,
            _ => Vec2::X,
        };
        let normal = Vec2::new(-direction.y, direction.x);
        // A limited miter keeps the halo connected through turns without spikes.
        let miter = after.or(before).map_or(1.0, |segment| {
            direction.dot(segment).abs().max(0.75).recip()
        });
        for column in 0..columns {
            let distance = extent * (column as f32 / steps as f32 - 1.0);
            mesh.colored_vertex(
                point + normal * (distance * miter),
                feather(color, distance.abs(), half_width, radius),
            );
        }
        if index > 0 {
            let previous = base + ((index - 1) * columns) as u32;
            let current = base + (index * columns) as u32;
            for column in 0..columns - 1 {
                let offset = column as u32;
                mesh.add_triangle(previous + offset, current + offset, previous + offset + 1);
                mesh.add_triangle(
                    previous + offset + 1,
                    current + offset,
                    current + offset + 1,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn cap(
    mesh: &mut Mesh,
    center: Pos2,
    start_angle: f32,
    sweep: f32,
    half_width: f32,
    radius: f32,
    steps: usize,
    color: Color32,
) {
    let extent = half_width + radius;
    let angles = ((extent * sweep).ceil() as usize).clamp(24, 96);
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(center, feather(color, 0.0, half_width, radius));
    for ring in 1..=steps {
        let distance = extent * ring as f32 / steps as f32;
        let tint = feather(color, distance, half_width, radius);
        for angle in 0..=angles {
            let theta = start_angle + sweep * angle as f32 / angles as f32;
            mesh.colored_vertex(center + Vec2::angled(theta) * distance, tint);
        }
        let current = base + 1 + ((ring - 1) * (angles + 1)) as u32;
        for angle in 0..angles as u32 {
            if ring == 1 {
                mesh.add_triangle(base, current + angle, current + angle + 1);
            } else {
                let previous = current - (angles + 1) as u32;
                mesh.add_triangle(previous + angle, current + angle, previous + angle + 1);
                mesh.add_triangle(previous + angle + 1, current + angle, current + angle + 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halo_fades_to_transparent_and_caps_extend_past_the_path() {
        let color = Color32::from_rgb(220, 245, 255);
        assert_eq!(feather(color, 15.0, 2.0, 13.0).a(), 0);
        let mut previous = u8::MAX;
        for step in 0..=100 {
            let alpha = feather(color, step as f32 * 0.15, 2.0, 13.0).a();
            assert!(alpha <= previous);
            previous = alpha;
        }
        let mut mesh = Mesh::default();
        cap(
            &mut mesh,
            Pos2::ZERO,
            std::f32::consts::FRAC_PI_2,
            std::f32::consts::PI,
            2.0,
            13.0,
            20,
            color,
        );
        assert!(mesh.is_valid());
        assert!(mesh.vertices.iter().any(|vertex| vertex.pos.x < -14.5));
        assert!(mesh.vertices.iter().all(|vertex| vertex.pos.is_finite()));
    }
}
