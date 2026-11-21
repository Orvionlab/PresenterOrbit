//! ProPresenterScreen/Screen/EdgeBlend wire fields from the pinned screen schemas.
use super::*;
use crate::output_layout::*;

fn number(fields: &[Field], field: u32) -> f64 {
    f64_field(fields, field).unwrap_or(0.)
}
fn curve_mode(value: u64) -> crate::projector_correction::BlendMode {
    match value {
        1 => crate::projector_correction::BlendMode::Cubic,
        2 => crate::projector_correction::BlendMode::Quadratic,
        _ => crate::projector_correction::BlendMode::Linear,
    }
}
fn curve_number(mode: crate::projector_correction::BlendMode) -> u64 {
    match mode {
        crate::projector_correction::BlendMode::Linear => 0,
        crate::projector_correction::BlendMode::Cubic => 1,
        crate::projector_correction::BlendMode::Quadratic => 2,
    }
}

fn target(fields: &[Field], width: usize, height: usize) -> DeviceTarget {
    let mut target = DeviceTarget::placeholder(width, height);
    if let Some(mode) = nested_field(fields, 9) {
        target.width = varint_field(&mode, 2)
            .filter(|v| *v > 0)
            .map_or(width, |v| v as usize);
        target.height = varint_field(&mode, 3)
            .filter(|v| *v > 0)
            .map_or(height, |v| v as usize);
        target.frame_rate = f64_field(&mode, 4).filter(|v| *v > 0.).unwrap_or(60.);
    }
    target.name = string_field(fields, 1).unwrap_or_default();
    target.id = string_field(fields, 10).unwrap_or_default();
    // Import preserves device identity, but an unknown display remains explicitly unassigned.
    target.kind = match varint_field(fields, 8) {
        Some(3) if !target.name.is_empty() => "ndi",
        Some(1) if !target.id.is_empty() => "display",
        _ => "placeholder",
    }
    .into();
    target.video_delay = varint_field(fields, 12).unwrap_or(0) as usize;
    target.source_type = varint_field(fields, 8).filter(|kind| matches!(kind, 2 | 4 | 5));
    target
}

pub(super) fn decode(
    screen: &[Field],
    arrangement: &[Field],
    mode: &str,
) -> Result<Option<OutputLayout>, String> {
    let devices: Vec<_> =
        nested_fields(arrangement, if mode == "edge-blend" { 2 } else { 1 }).collect();
    if devices.is_empty() {
        return Ok(None);
    }
    let mut outputs = Vec::new();
    let mut positions = Vec::new();
    let mut bounds_by_device = Vec::new();
    for device in &devices {
        if number(device, 9) != 0.
            || number(device, 10) != 0.
            || number(device, 11) != 0.
            || bool_field(device, 15) == Some(true)
            || nested_field(device, 17).is_some_and(|value| number(&value, 1) != 0.)
        {
            return Err("Imported screen uses rotation, legacy gamma/black levels, device color or blend compensation not represented by this output layout.".into());
        }
        if let Some(rectangle) = nested_field(device, 8) {
            let rectangle = decode_rect(&rectangle);
            let full = rectangle["origin"]["x"].as_f64() == Some(0.)
                && rectangle["origin"]["y"].as_f64() == Some(0.)
                && rectangle["size"]["width"].as_f64() == Some(1.)
                && rectangle["size"]["height"].as_f64() == Some(1.);
            if !full {
                return Err("Imported device uses a partial hardware output target; only Full is currently supported.".into());
            }
        }
        let bounds = nested_field(device, 4)
            .map(|value| decode_rect(&value))
            .ok_or("Imported output device has no canvas bounds.")?;
        let dimension = |key: &str| {
            bounds["size"][key]
                .as_f64()
                .filter(|value| value.is_finite() && *value > 0. && value.fract() == 0.)
                .map(|value| value as usize)
                .ok_or_else(|| format!("Imported device {key} must be a positive whole number."))
        };
        let (width, height) = (dimension("width")?, dimension("height")?);
        let target = nested_field(device, 14).map_or_else(
            || DeviceTarget::placeholder(width, height),
            |fields| target(&fields, width, height),
        );
        positions.push(bounds["origin"]["x"].as_f64().unwrap_or(0.));
        bounds_by_device.push(bounds);
        let adjustment = nested_field(device, 16).unwrap_or_default();
        let color = OutputColor {
            gamma: number(&adjustment, 1),
            black: number(&adjustment, 2),
            red: number(&adjustment, 3),
            green: number(&adjustment, 4),
            blue: number(&adjustment, 5),
            brightness: number(&adjustment, 6),
            contrast: number(&adjustment, 7),
        };
        let corner_pin = if bool_field(device, 7) == Some(true) {
            let fields = nested_field(device, 13)
                .ok_or("Imported corner pin is enabled without corner values.")?;
            let mut corners = neutral_corners();
            for (field, point, sx, sy) in [
                (1, &mut corners.top_left, -1., -1.),
                (2, &mut corners.top_right, 1., -1.),
                (3, &mut corners.bottom_left, -1., 1.),
                (4, &mut corners.bottom_right, 1., 1.),
            ] {
                let value = nested_field(&fields, field)
                    .ok_or("Imported corner pin is missing a corner.")?;
                point.x += sx * number(&value, 1) / width as f64;
                point.y += sy * number(&value, 2) / height as f64;
            }
            Some(corners)
        } else {
            None
        };
        let alpha = nested_field(device, 18).and_then(|alpha| {
            let mode = match varint_field(&alpha, 1) {
                Some(2) => crate::key_fill::FillMode::Premultiplied,
                Some(3) => crate::key_fill::FillMode::Straight,
                _ => return None,
            };
            let target = nested_field(&alpha, 2)
                .and_then(|value| nested_field(&value, 1))
                .map_or_else(
                    || DeviceTarget::placeholder(width, height),
                    |fields| self::target(&fields, width, height),
                );
            Some(AlphaOutput { mode, target })
        });
        outputs.push(OutputDevice {
            id: uuid(device, 1).ok_or("Imported output device has no UUID.")?,
            label: string_field(device, 2).unwrap_or_default(),
            target,
            color,
            corner_pin,
            alpha,
        });
    }
    let first_bounds = nested_field(&devices[0], 4)
        .map(|value| decode_rect(&value))
        .ok_or("Imported device bounds are unavailable.")?;
    let mut layout = OutputLayout {
        mode: match mode {
            "combined" => LayoutMode::Grouped,
            "edge-blend" => LayoutMode::EdgeBlend,
            _ if outputs.len() > 1 => LayoutMode::Mirror,
            _ => LayoutMode::Single,
        },
        rows: if mode == "combined" {
            varint_field(arrangement, 2).unwrap_or(1) as usize
        } else {
            1
        },
        columns: if mode == "combined" {
            varint_field(arrangement, 3).unwrap_or(1) as usize
        } else {
            outputs.len()
        },
        tile_width: first_bounds["size"]["width"]
            .as_f64()
            .ok_or("Imported width is unavailable.")? as usize,
        tile_height: first_bounds["size"]["height"]
            .as_f64()
            .ok_or("Imported height is unavailable.")? as usize,
        outputs,
        blends: Vec::new(),
        screen_color: if bool_field(screen, 8) == Some(true) {
            nested_field(screen, 6).and_then(|value| color(&value))
        } else {
            None
        },
    };
    if layout.mode == LayoutMode::EdgeBlend {
        let seams: Vec<_> = nested_fields(arrangement, 3).collect();
        for index in 0..layout.outputs.len() - 1 {
            let overlap = positions[index] + layout.tile_width as f64 - positions[index + 1];
            if !overlap.is_finite() || overlap <= 0. || overlap.fract() != 0. {
                return Err(
                    "Imported edge-blend devices must have a positive whole-pixel overlap.".into(),
                );
            }
            let seam = seams
                .iter()
                .find(|seam| {
                    nested_field(seam, 7)
                        .or_else(|| nested_field(seam, 5))
                        .and_then(|value| uuid(&value, 1))
                        .as_deref()
                        == Some(layout.outputs[index].id.as_str())
                })
                .ok_or("Imported overlap has no matching edge-blend settings.")?;
            let edge = |field: u32, alternate: u32| -> Result<BlendCurve, String> {
                let value = nested_field(seam, field)
                    .or_else(|| nested_field(seam, alternate))
                    .ok_or("Imported blend is missing an edge.")?;
                Ok(BlendCurve {
                    mode: curve_mode(varint_field(&value, 5).unwrap_or(0)),
                    radius: number(&value, 6),
                    intensity: number(&value, 7),
                    gamma: number(&value, 3),
                    black_level: number(&value, 4),
                })
            };
            layout.blends.push(BlendSeam {
                width: overlap as usize,
                left: edge(7, 5)?,
                right: edge(8, 6)?,
            });
        }
    }
    layout.validate()?;
    for (index, bounds) in bounds_by_device.iter().enumerate() {
        let region = layout.region(index);
        if bounds["size"]["width"].as_f64() != Some(region.width as f64)
            || bounds["size"]["height"].as_f64() != Some(region.height as f64)
            || bounds["origin"]["x"].as_f64() != Some(region.x as f64)
            || bounds["origin"]["y"].as_f64() != Some(region.y as f64)
        {
            return Err("Imported output layout requires equal device tiles arranged from the canvas origin in row order.".into());
        }
    }
    Ok(Some(layout))
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Vec<u8> {
    let mut origin = Vec::new();
    put_f64(1, x, &mut origin);
    put_f64(2, y, &mut origin);
    let mut size = Vec::new();
    put_f64(1, width, &mut size);
    put_f64(2, height, &mut size);
    let mut value = Vec::new();
    put_message(1, origin, &mut value);
    put_message(2, size, &mut value);
    value
}

fn display(target: &DeviceTarget) -> Vec<u8> {
    let mut value = Vec::new();
    put_string(1, &target.name, &mut value);
    put_string(10, &target.id, &mut value);
    put_varint(
        8,
        match target.kind.as_str() {
            "display" => 1,
            "ndi" => 3,
            _ => target.source_type.unwrap_or(0),
        },
        &mut value,
    );
    let mut mode = Vec::new();
    put_varint(2, target.width as u64, &mut mode);
    put_varint(3, target.height as u64, &mut mode);
    put_f64(4, target.frame_rate, &mut mode);
    put_message(9, mode, &mut value);
    put_varint(12, target.video_delay as u64, &mut value);
    value
}

fn device(layout: &OutputLayout, index: usize) -> Vec<u8> {
    let device = &layout.outputs[index];
    let region = layout.region(index);
    let mut value = Vec::new();
    put_uuid(1, &device.id, &mut value);
    put_string(2, &device.label, &mut value);
    put_message(
        4,
        rect(
            region.x as f64,
            region.y as f64,
            region.width as f64,
            region.height as f64,
        ),
        &mut value,
    );
    put_message(14, display(&device.target), &mut value);
    put_message(8, rect(0., 0., 1., 1.), &mut value);
    let mut color = Vec::new();
    for (field, amount) in [
        (1, device.color.gamma),
        (2, device.color.black),
        (3, device.color.red),
        (4, device.color.green),
        (5, device.color.blue),
        (6, device.color.brightness),
        (7, device.color.contrast),
    ] {
        put_f64(field, amount, &mut color);
    }
    put_message(16, color, &mut value);
    if let Some(corners) = device.corner_pin {
        put_varint(7, 1, &mut value);
        let mut points = Vec::new();
        for (field, point, ox, oy, sx, sy) in [
            (1, corners.top_left, 0., 0., -1., -1.),
            (2, corners.top_right, 1., 0., 1., -1.),
            (3, corners.bottom_left, 0., 1., -1., 1.),
            (4, corners.bottom_right, 1., 1., 1., 1.),
        ] {
            let mut pair = Vec::new();
            put_f64(1, (point.x - ox) * sx * layout.tile_width as f64, &mut pair);
            put_f64(
                2,
                (point.y - oy) * sy * layout.tile_height as f64,
                &mut pair,
            );
            put_message(field, pair, &mut points);
        }
        put_message(13, points, &mut value);
    }
    if let Some(alpha) = &device.alpha {
        let mut settings = Vec::new();
        put_varint(
            1,
            match alpha.mode {
                crate::key_fill::FillMode::Premultiplied => 2,
                crate::key_fill::FillMode::Straight => 3,
            },
            &mut settings,
        );
        let mut key = Vec::new();
        put_message(1, display(&alpha.target), &mut key);
        put_message(2, rect(0., 0., 1., 1.), &mut key);
        put_message(2, key, &mut settings);
        put_message(18, settings, &mut value);
    }
    value
}

pub(crate) fn encode(workspace: &Value, mut bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    for (id, config) in workspace["screenConfig"].as_object().into_iter().flatten() {
        if config
            .get("canvasView")
            .is_some_and(|value| !value.is_null())
            || config.get("keyFill").is_some_and(|value| !value.is_null())
            || config
                .get("projectorCorrection")
                .is_some_and(|value| !value.is_null())
        {
            return Err(format!("Screen '{id}' uses independent canvas/key/projector routing that has no ProPresenter layout mapping."));
        }
        let layout = OutputLayout::from_screen(id, config)?;
        layout.validate()?;
        if layout
            .outputs
            .iter()
            .any(|output| output.target.format != "sdr")
        {
            return Err(
                "The pinned ProPresenter screen schema cannot represent Orbit HDR output formats."
                    .into(),
            );
        }
        let mut screen = Vec::new();
        put_string(1, config["label"].as_str().unwrap_or(id), &mut screen);
        put_varint(
            2,
            if config["type"] == "stage" { 2 } else { 1 },
            &mut screen,
        );
        put_uuid(7, id, &mut screen);
        // Newer ProPresenter builds add rendering_enabled as field 9.
        put_varint(9, u64::from(config["enabled"] == true), &mut screen);
        if let Some(color) = &layout.screen_color {
            put_color(6, color, &mut screen);
            put_varint(8, 1, &mut screen);
        }
        let mut arrangement = Vec::new();
        let field = if layout.mode == LayoutMode::EdgeBlend {
            2
        } else {
            1
        };
        for index in 0..layout.outputs.len() {
            put_message(field, device(&layout, index), &mut arrangement);
        }
        match layout.mode {
            LayoutMode::Grouped => {
                put_varint(2, layout.rows as u64, &mut arrangement);
                put_varint(3, layout.columns as u64, &mut arrangement);
            }
            LayoutMode::EdgeBlend => {
                put_varint(1, layout.outputs.len() as u64, &mut arrangement);
                for (index, seam) in layout.blends.iter().enumerate() {
                    let mut blend = Vec::new();
                    put_uuid(1, &format!("{id}:blend:{index}"), &mut blend);
                    for (field, output, curve, direction) in [
                        (7, &layout.outputs[index], seam.left, 2),
                        (8, &layout.outputs[index + 1], seam.right, 1),
                    ] {
                        let mut edge = Vec::new();
                        put_uuid(1, &output.id, &mut edge);
                        put_varint(2, direction, &mut edge);
                        put_f64(3, curve.gamma, &mut edge);
                        put_f64(4, curve.black_level, &mut edge);
                        put_varint(5, curve_number(curve.mode), &mut edge);
                        put_f64(6, curve.radius, &mut edge);
                        put_f64(7, curve.intensity, &mut edge);
                        put_message(field, edge, &mut blend);
                    }
                    put_message(3, blend, &mut arrangement);
                }
            }
            _ => {}
        }
        put_message(
            match layout.mode {
                LayoutMode::Grouped => 4,
                LayoutMode::EdgeBlend => 5,
                _ => 3,
            },
            arrangement,
            &mut screen,
        );
        put_message(1, screen, &mut bytes);
    }
    for look in workspace["audienceLooks"].as_array().into_iter().flatten() {
        let mut value = Vec::new();
        put_uuid(
            1,
            look["id"]
                .as_str()
                .ok_or("Look ID is required for export.")?,
            &mut value,
        );
        put_string(
            2,
            look["title"]
                .as_str()
                .ok_or("Look title is required for export.")?,
            &mut value,
        );
        for entry in look["screenLooks"].as_array().into_iter().flatten() {
            let mut screen = Vec::new();
            put_uuid(
                1,
                entry["screenId"]
                    .as_str()
                    .ok_or("Look screen ID is required for export.")?,
                &mut screen,
            );
            for (field, key) in [
                (10, "props"),
                (4, "media"),
                (3, "videoInput"),
                (7, "slide"),
                (11, "messages"),
                (9, "announcements"),
            ] {
                put_varint(
                    field,
                    u64::from(entry["lookLayers"][key] != false),
                    &mut screen,
                );
            }
            if let Some(mask) = entry["maskId"].as_str().filter(|value| !value.is_empty()) {
                put_uuid(8, mask, &mut screen);
            }
            put_message(3, screen, &mut value);
        }
        put_message(2, value, &mut bytes);
    }
    Ok(bytes)
}
