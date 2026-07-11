//! Pure stats helpers for dashboard trends (OLS regression, direction labels).

use serde::Serialize;

#[derive(Debug, Clone, PartialEq)]
pub struct DataPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Regression {
    pub slope: f64,
    pub intercept: f64,
    pub r_squared: f64,
    pub slope_per_day: f64,
    pub slope_per_week: f64,
    pub n: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrendDirection {
    Up,
    Down,
    Flat,
    InsufficientData,
}

/// Always present on HTML report overview for agents.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MetricTrend {
    pub direction: TrendDirection,
    pub label: String,
    pub n: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slope_per_day: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slope_per_week: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r_squared: Option<f64>,
}

/// Flat band for weight (~0.035 kg/week).
pub const WEIGHT_FLAT_KG_PER_DAY: f64 = 0.005;
/// Flat band for body fat % (~0.07 %/week).
pub const BF_FLAT_PCT_PER_DAY: f64 = 0.01;

/// Two-pass ordinary least squares. Returns `None` when `n < 2` or all x identical.
pub fn linear_regression(points: &[DataPoint]) -> Option<Regression> {
    let n = points.len();
    if n < 2 {
        return None;
    }

    let n_f = n as f64;
    let x_mean = points.iter().map(|p| p.x).sum::<f64>() / n_f;
    let y_mean = points.iter().map(|p| p.y).sum::<f64>() / n_f;

    let mut ss_xy = 0.0;
    let mut ss_xx = 0.0;
    let mut ss_yy = 0.0;
    for p in points {
        let dx = p.x - x_mean;
        let dy = p.y - y_mean;
        ss_xy += dx * dy;
        ss_xx += dx * dx;
        ss_yy += dy * dy;
    }

    if ss_xx.abs() < f64::EPSILON {
        return None;
    }

    let slope = ss_xy / ss_xx;
    let intercept = y_mean - slope * x_mean;

    let ss_res: f64 = points
        .iter()
        .map(|p| {
            let residual = p.y - (slope * p.x + intercept);
            residual * residual
        })
        .sum();

    let r_squared = if ss_yy.abs() < f64::EPSILON {
        1.0
    } else {
        1.0 - ss_res / ss_yy
    };

    Some(Regression {
        slope,
        intercept,
        r_squared,
        slope_per_day: slope,
        slope_per_week: slope * 7.0,
        n,
    })
}

/// Classifies slope vs metric-specific flat band. Never returns `InsufficientData`
/// (that is constructed only when `n < 2` or regression fails).
pub fn trend_direction(slope_per_day: f64, flat_eps: f64) -> TrendDirection {
    if slope_per_day.abs() < flat_eps {
        TrendDirection::Flat
    } else if slope_per_day > 0.0 {
        TrendDirection::Up
    } else {
        TrendDirection::Down
    }
}

/// Human card subtitle, e.g. `"↓ -0.12 kg/wk"` or `"→ flat"`.
pub fn trend_label_weekly(slope_per_week: f64, unit: &str, dir: TrendDirection) -> String {
    match dir {
        TrendDirection::InsufficientData => "—".into(),
        TrendDirection::Flat => "→ flat".into(),
        TrendDirection::Up => format!("↑ {:+.2} {}/wk", slope_per_week, unit),
        TrendDirection::Down => format!("↓ {:+.2} {}/wk", slope_per_week, unit),
    }
}

/// Build a [`MetricTrend`] from (day-offset, y) points after filtering nulls.
pub fn metric_trend_from_points(points: &[DataPoint], unit: &str, flat_eps: f64) -> MetricTrend {
    let n = points.len();
    if n < 2 {
        return MetricTrend {
            direction: TrendDirection::InsufficientData,
            label: "—".into(),
            n,
            slope_per_day: None,
            slope_per_week: None,
            r_squared: None,
        };
    }
    match linear_regression(points) {
        Some(reg) => {
            let dir = trend_direction(reg.slope_per_day, flat_eps);
            MetricTrend {
                direction: dir,
                label: trend_label_weekly(reg.slope_per_week, unit, dir),
                n: reg.n,
                slope_per_day: Some(reg.slope_per_day),
                slope_per_week: Some(reg.slope_per_week),
                r_squared: Some(reg.r_squared),
            }
        }
        None => MetricTrend {
            direction: TrendDirection::InsufficientData,
            label: "—".into(),
            n,
            slope_per_day: None,
            slope_per_week: None,
            r_squared: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_line() {
        let pts: Vec<DataPoint> = (0..5)
            .map(|i| DataPoint {
                x: i as f64,
                y: 2.0 * i as f64 + 1.0,
            })
            .collect();
        let r = linear_regression(&pts).expect("regression");
        assert!((r.slope - 2.0).abs() < 1e-9);
        assert!((r.intercept - 1.0).abs() < 1e-9);
        assert!((r.r_squared - 1.0).abs() < 1e-9);
        assert!((r.slope_per_week - 14.0).abs() < 1e-9);
        assert_eq!(r.n, 5);
    }

    #[test]
    fn single_point_none() {
        assert!(linear_regression(&[DataPoint { x: 0.0, y: 1.0 }]).is_none());
    }

    #[test]
    fn identical_x_none() {
        let pts = vec![DataPoint { x: 1.0, y: 2.0 }, DataPoint { x: 1.0, y: 3.0 }];
        assert!(linear_regression(&pts).is_none());
    }

    #[test]
    fn flat_within_eps() {
        assert_eq!(
            trend_direction(0.001, WEIGHT_FLAT_KG_PER_DAY),
            TrendDirection::Flat
        );
        assert_eq!(
            trend_direction(0.01, WEIGHT_FLAT_KG_PER_DAY),
            TrendDirection::Up
        );
        assert_eq!(
            trend_direction(-0.01, WEIGHT_FLAT_KG_PER_DAY),
            TrendDirection::Down
        );
    }

    #[test]
    fn trend_label_flat_has_no_unit() {
        assert_eq!(
            trend_label_weekly(0.0, "kg", TrendDirection::Flat),
            "→ flat"
        );
        assert_eq!(
            trend_label_weekly(-0.12, "kg", TrendDirection::Down),
            "↓ -0.12 kg/wk"
        );
    }

    #[test]
    fn metric_trend_insufficient() {
        let t = metric_trend_from_points(
            &[DataPoint { x: 0.0, y: 80.0 }],
            "kg",
            WEIGHT_FLAT_KG_PER_DAY,
        );
        assert_eq!(t.direction, TrendDirection::InsufficientData);
        assert_eq!(t.label, "—");
        assert_eq!(t.n, 1);
        assert!(t.slope_per_day.is_none());
    }

    #[test]
    fn metric_trend_sufficient_serializes_snake_case() {
        let pts = vec![DataPoint { x: 0.0, y: 82.0 }, DataPoint { x: 7.0, y: 81.0 }];
        let t = metric_trend_from_points(&pts, "kg", WEIGHT_FLAT_KG_PER_DAY);
        assert_eq!(t.direction, TrendDirection::Down);
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["direction"], "down");
        assert!(v.get("slope_per_week").is_some());
    }
}
