//! Strict, software-only per-output projector correction settings and math.
//!
//! Blend behavior is an Orbit policy: linear uses `t`, quadratic uses a symmetric
//! piecewise quadratic ease-in/out,
//! and cubic uses smoothstep `t²(3 - 2t)`. The edge blend is raised to
//! `1 / blendGamma`; this does not claim to model a projector's physical or
//! optical response.

use serde::{Deserialize, Serialize};
use serde_json::Value;

const CONVEXITY_THRESHOLD: f64 = 1.0e-6;
const NUMERICAL_EPSILON: f64 = 1.0e-12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlendMode {
    Linear,
    Quadratic,
    Cubic,
}

impl BlendMode {
    fn curve(self, amount: f64) -> f64 {
        match self {
            Self::Linear => amount,
            Self::Quadratic if amount < 0.5 => 2.0 * amount * amount,
            Self::Quadratic => 1.0 - 2.0 * (1.0 - amount) * (1.0 - amount),
            Self::Cubic => amount * amount * (3.0 - 2.0 * amount),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectorCorrection {
    pub edges: EdgeFractions,
    pub blend_mode: BlendMode,
    pub blend_gamma: f64,
    pub gain: RgbGain,
    pub black_level: f64,
    pub corners: ProjectorCorners,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<crate::output_layout::OutputColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_curves: Option<[Option<crate::output_layout::BlendCurve>; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EdgeFractions {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RgbGain {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectorCorners {
    pub top_left: Point,
    pub top_right: Point,
    pub bottom_right: Point,
    pub bottom_left: Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl ProjectorCorrection {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("left", self.edges.left),
            ("right", self.edges.right),
            ("top", self.edges.top),
            ("bottom", self.edges.bottom),
        ] {
            validate_range(value, 0.0, 0.5, &format!("Edge {name}"))?;
        }
        if self.edges.left + self.edges.right >= 1.0 {
            return Err(
                "Projector correction left and right edge widths must sum to less than 1.".into(),
            );
        }
        if self.edges.top + self.edges.bottom >= 1.0 {
            return Err(
                "Projector correction top and bottom edge widths must sum to less than 1.".into(),
            );
        }
        validate_range(
            self.blend_gamma,
            0.1,
            5.0,
            "Projector correction blendGamma",
        )?;
        for (name, value) in [
            ("red", self.gain.red),
            ("green", self.gain.green),
            ("blue", self.gain.blue),
        ] {
            validate_range(
                value,
                0.0,
                2.0,
                &format!("Projector correction gain.{name}"),
            )?;
        }
        validate_range(
            self.black_level,
            0.0,
            0.5,
            "Projector correction blackLevel",
        )?;
        if let Some(color) = self.color {
            color.validate()?;
        }
        if let Some(curves) = self.edge_curves {
            for curve in curves.into_iter().flatten() {
                curve.validate()?;
            }
        }
        self.corners.validate()
    }

    pub fn edge_weight(&self, x: f64, y: f64) -> f64 {
        let width = |distance: f64, edge: f64, index: usize| {
            if edge == 0.0 {
                1.0
            } else if let Some(curve) = self.edge_curves.and_then(|curves| curves[index]) {
                curve.weight(distance / edge)
            } else {
                self.blend_mode
                    .curve((distance / edge).clamp(0.0, 1.0))
                    .powf(1.0 / self.blend_gamma)
            }
        };
        width(x, self.edges.left, 0)
            * width(1.0 - x, self.edges.right, 1)
            * width(y, self.edges.top, 2)
            * width(1.0 - y, self.edges.bottom, 3)
    }

    pub fn inverse_homography(&self) -> Result<InverseHomography, String> {
        self.validate()?;
        InverseHomography::from_corners(self.corners)
    }
}

impl ProjectorCorners {
    pub fn points(self) -> [Point; 4] {
        [
            self.top_left,
            self.top_right,
            self.bottom_right,
            self.bottom_left,
        ]
    }

    pub fn validate(&self) -> Result<(), String> {
        let points = self.points();
        for (index, point) in points.iter().enumerate() {
            validate_range(point.x, -1.0, 2.0, &format!("Projector corner {index} x"))?;
            validate_range(point.y, -1.0, 2.0, &format!("Projector corner {index} y"))?;
        }
        for index in 0..4 {
            let a = points[index];
            let b = points[(index + 1) % 4];
            let c = points[(index + 2) % 4];
            if cross(a, b, c) <= CONVEXITY_THRESHOLD {
                return Err(
                    "Projector correction corners must form a strictly convex clockwise quadrilateral.".into(),
                );
            }
        }
        let area_twice = points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let next = points[(index + 1) % 4];
                point.x * next.y - next.x * point.y
            })
            .sum::<f64>();
        if !area_twice.is_finite() || area_twice <= CONVEXITY_THRESHOLD {
            return Err("Projector correction corners must have a nonzero clockwise area.".into());
        }
        Ok(())
    }
}

/// Parse the optional screen/scene field and validate numeric ranges and geometry.
pub fn parse_optional(value: Option<&Value>) -> Result<Option<ProjectorCorrection>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let correction: ProjectorCorrection = serde_json::from_value(value.clone())
        .map_err(|error| format!("Projector correction is invalid: {error}"))?;
    correction.validate()?;
    Ok(Some(correction))
}

fn validate_range(value: f64, min: f64, max: f64, field: &str) -> Result<(), String> {
    if !value.is_finite() || !(min..=max).contains(&value) {
        return Err(format!(
            "{field} must be a finite number between {min} and {max}."
        ));
    }
    Ok(())
}

fn cross(a: Point, b: Point, c: Point) -> f64 {
    (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x)
}

/// Inverse perspective map from output coordinates to the source rectangle.
#[derive(Debug, Clone, Copy)]
pub struct InverseHomography {
    coefficients: [f64; 8],
}

impl InverseHomography {
    fn from_corners(corners: ProjectorCorners) -> Result<Self, String> {
        let destinations = corners.points();
        let sources = [
            Point { x: 0.0, y: 0.0 },
            Point { x: 1.0, y: 0.0 },
            Point { x: 1.0, y: 1.0 },
            Point { x: 0.0, y: 1.0 },
        ];
        let mut matrix = [[0.0; 9]; 8];
        for index in 0..4 {
            let Point { x, y } = destinations[index];
            let Point { x: u, y: v } = sources[index];
            let row = index * 2;
            matrix[row] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
            matrix[row + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
        }
        let coefficients = solve_8x8(matrix).ok_or_else(|| {
            "Projector correction corners produce a degenerate perspective transform.".to_string()
        })?;
        Ok(Self { coefficients })
    }

    pub fn map(&self, x: f64, y: f64) -> Result<Point, String> {
        let h = self.coefficients;
        let denominator = h[6] * x + h[7] * y + 1.0;
        if !denominator.is_finite() || denominator.abs() <= NUMERICAL_EPSILON {
            return Err(
                "Projector correction perspective transform is degenerate at an output pixel."
                    .into(),
            );
        }
        let point = Point {
            x: (h[0] * x + h[1] * y + h[2]) / denominator,
            y: (h[3] * x + h[4] * y + h[5]) / denominator,
        };
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(
                "Projector correction perspective transform produced a non-finite coordinate."
                    .into(),
            );
        }
        Ok(point)
    }
}

pub fn contains_output_point(corners: ProjectorCorners, point: Point) -> bool {
    let points = corners.points();
    (0..4).all(|index| cross(points[index], points[(index + 1) % 4], point) >= -NUMERICAL_EPSILON)
}

fn solve_8x8(mut matrix: [[f64; 9]; 8]) -> Option<[f64; 8]> {
    for column in 0..8 {
        let pivot = (column..8).max_by(|left, right| {
            matrix[*left][column]
                .abs()
                .total_cmp(&matrix[*right][column].abs())
        })?;
        if matrix[pivot][column].abs() <= NUMERICAL_EPSILON {
            return None;
        }
        matrix.swap(column, pivot);
        let divisor = matrix[column][column];
        for cell in column..=8 {
            matrix[column][cell] /= divisor;
        }
        for row in 0..8 {
            if row == column {
                continue;
            }
            let factor = matrix[row][column];
            for cell in column..=8 {
                matrix[row][cell] -= factor * matrix[column][cell];
            }
        }
    }
    let mut result = [0.0; 8];
    for row in 0..8 {
        result[row] = matrix[row][8];
        if !result[row].is_finite() {
            return None;
        }
    }
    Some(result)
}
