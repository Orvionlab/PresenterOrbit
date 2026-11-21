// The OKLab matrices are published with more digits than f32 holds; keep them verbatim.
#![allow(clippy::excessive_precision)]

use gpui::Rgba;

/// Converts packed RGB from authored content to an opaque display color.
pub fn opaque_rgb24(value: u32) -> Rgba {
    gpui::rgb(value)
}

/// CSS `color-mix(in srgb, a weight_a, b)`: premultiplied linear interpolation of the encoded sRGB channels.
pub fn mix_srgb(a: Rgba, weight_a: f32, b: Rgba) -> Rgba {
    let weight_b = 1.0 - weight_a;
    let alpha = a.a * weight_a + b.a * weight_b;
    let channel = |a_c: f32, b_c: f32| {
        if alpha == 0.0 {
            0.0
        } else {
            (a_c * a.a * weight_a + b_c * b.a * weight_b) / alpha
        }
    };
    Rgba {
        r: channel(a.r, b.r),
        g: channel(a.g, b.g),
        b: channel(a.b, b.b),
        a: alpha,
    }
}

/// CSS `color-mix(in oklch, a weight_a, b)` with the shorter hue arc. Colors must be opaque.
pub fn mix_oklch(a: Rgba, weight_a: f32, b: Rgba) -> Rgba {
    assert!(a.a == 1.0 && b.a == 1.0, "mix_oklch mixes opaque colors");
    let weight_b = 1.0 - weight_a;
    let ([l1, c1, mut h1], [l2, c2, mut h2]) = (to_oklch(a), to_oklch(b));
    if h2 - h1 > 180.0 {
        h1 += 360.0;
    } else if h2 - h1 < -180.0 {
        h2 += 360.0;
    }
    from_oklch([
        l1 * weight_a + l2 * weight_b,
        c1 * weight_a + c2 * weight_b,
        h1 * weight_a + h2 * weight_b,
    ])
}

fn to_linear(encoded: f32) -> f32 {
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

fn to_encoded(linear: f32) -> f32 {
    if linear <= 0.0031308 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

fn to_oklch(color: Rgba) -> [f32; 3] {
    let (r, g, b) = (to_linear(color.r), to_linear(color.g), to_linear(color.b));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let lightness = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
    let b = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
    [
        lightness,
        a.hypot(b),
        b.atan2(a).to_degrees().rem_euclid(360.0),
    ]
}

fn from_oklch([lightness, chroma, hue]: [f32; 3]) -> Rgba {
    let (a, b) = (
        chroma * hue.to_radians().cos(),
        chroma * hue.to_radians().sin(),
    );
    let l = (lightness + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m = (lightness - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s = (lightness - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    let linear = [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ];
    // Out-of-gamut results clip, as the browser does when it serializes to sRGB.
    let [r, g, b] = linear.map(|value| to_encoded(value).clamp(0.0, 1.0));
    Rgba { r, g, b, a: 1.0 }
}
