use gpui::{Bounds, Pixels, Point};

pub fn moved_far_enough(start: Point<Pixels>, position: Point<Pixels>, threshold: Pixels) -> bool {
    let offset = position - start;
    f32::from(offset.x).abs().max(f32::from(offset.y).abs()) >= f32::from(threshold)
}

/// The drop gap (a master index, `len` for the end) for a pointer position: the gap on the nearer side of the
/// tile closest to the pointer, so gaps between tiles and the space around them still resolve.
pub fn gap_at(
    position: Point<Pixels>,
    tiles: impl Iterator<Item = (usize, Bounds<Pixels>)>,
    vertical: bool,
) -> Option<usize> {
    let (master, bounds) = closest_tile(position, tiles)?;
    let after = if vertical {
        position.y > bounds.center().y
    } else {
        position.x > bounds.center().x
    };
    Some(master + usize::from(after))
}

pub fn closest_tile(
    position: Point<Pixels>,
    tiles: impl Iterator<Item = (usize, Bounds<Pixels>)>,
) -> Option<(usize, Bounds<Pixels>)> {
    let distance = |bounds: &Bounds<Pixels>| {
        let dx = (bounds.left() - position.x).max(position.x - bounds.right());
        let dy = (bounds.top() - position.y).max(position.y - bounds.bottom());
        let (dx, dy) = (f32::from(dx).max(0.), f32::from(dy).max(0.));
        dx * dx + dy * dy
    };
    tiles.min_by(|(ma, a), (mb, b)| distance(a).total_cmp(&distance(b)).then_with(|| ma.cmp(mb)))
}
