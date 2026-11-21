use super::{
    layer_pixel_bounds, native_object_builds::BuildEffect, native_transitions, NativeLayerBounds,
    NativeLayerFrame, NativeLayeredFrame,
};
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

/// Animate the complete object plane, including its stroke, shadow and media.
pub(super) fn apply(
    layer: &mut NativeLayerFrame,
    effects: &[BuildEffect],
    beneath: &[NativeLayerFrame],
    width: usize,
    height: usize,
) {
    if effects.is_empty() {
        return;
    }
    let background = if layer.replace {
        NativeLayeredFrame {
            width,
            height,
            layers: beneath.to_vec(),
            dynamic_media: false,
        }
        .flatten()
        .pixels
    } else {
        vec![0; width * height * 4]
    };
    let bounds = if layer.replace {
        difference_bounds(&background, layer.pixels.as_slice(), width, height)
    } else {
        layer.bounds
    };
    let Some(bounds) = bounds else {
        return;
    };
    let baseline = crop(&background, width, bounds);
    let mut object = crop(layer.pixels.as_slice(), width, bounds);
    let mut hasher = std::hash::DefaultHasher::new();
    layer.visual_key.hash(&mut hasher);
    for effect in effects {
        format!("{:?}", effect.transition).hash(&mut hasher);
        effect.progress.to_bits().hash(&mut hasher);
        effect.outgoing.hash(&mut hasher);
        let (from, to) = if effect.outgoing {
            (&object, &baseline)
        } else {
            (&baseline, &object)
        };
        let mut output = vec![0; object.len()];
        native_transitions::blend_object(
            &effect.transition,
            effect.progress as f64,
            from,
            to,
            &mut output,
            bounds.width,
            bounds.height,
        );
        object = output;
    }
    let mut pixels = background;
    for row in 0..bounds.height {
        let target = ((bounds.y + row) * width + bounds.x) * 4;
        let source = row * bounds.width * 4;
        pixels[target..target + bounds.width * 4]
            .copy_from_slice(&object[source..source + bounds.width * 4]);
    }
    layer.pixels = Arc::new(pixels);
    layer.visual_key = hasher.finish();
    layer.dynamic_media = true;
    layer.bounds = layer_pixel_bounds(layer.pixels.as_slice(), width, height, layer.replace);
}

fn crop(pixels: &[u8], width: usize, bounds: NativeLayerBounds) -> Vec<u8> {
    let mut cropped = Vec::with_capacity(bounds.width * bounds.height * 4);
    for row in bounds.y..bounds.y + bounds.height {
        let offset = (row * width + bounds.x) * 4;
        cropped.extend_from_slice(&pixels[offset..offset + bounds.width * 4]);
    }
    cropped
}

fn difference_bounds(
    from: &[u8],
    to: &[u8],
    width: usize,
    height: usize,
) -> Option<NativeLayerBounds> {
    let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
    for y in 0..height {
        for x in 0..width {
            let offset = (y * width + x) * 4;
            if from[offset..offset + 4] != to[offset..offset + 4] {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    (right > left && bottom > top).then_some(NativeLayerBounds {
        x: left,
        y: top,
        width: right.saturating_sub(left),
        height: bottom.saturating_sub(top),
    })
}
