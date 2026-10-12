//! Number formatting for perception text: millimeters, 0.1 degree,
//! thousands separators, and a token estimate for size budgets.

use glam::Vec3;

/// Millimeters from meters: one decimal below 10 mm, whole millimeters
/// above. Never prints `-0`.
pub fn mm(m: f32) -> String {
    let v = f64::from(m) * 1000.0;
    let s = if v.abs() < 9.95 { format!("{:.1}", v) } else { format!("{:.0}", v) };
    let s = s.strip_suffix(".0").map(str::to_owned).unwrap_or(s);
    if s == "-0" { "0".into() } else { s }
}

/// Whole millimeters (for coordinates in outlines, where 0.1 mm is noise).
pub fn mm0(m: f32) -> String {
    let s = format!("{:.0}", f64::from(m) * 1000.0);
    if s == "-0" { "0".into() } else { s }
}

/// `600 x 600 x 600 mm` (W x H x D).
pub fn dims(d: Vec3) -> String {
    format!("{} x {} x {} mm", mm(d.x), mm(d.y), mm(d.z))
}

/// `(0, 300, 0)` in millimeters.
pub fn point(p: Vec3) -> String {
    format!("({}, {}, {})", mm(p.x), mm(p.y), mm(p.z))
}

/// A direction with two decimals per component: `(0.87, 0, -0.5)`.
pub fn unit(v: Vec3) -> String {
    let c = |x: f32| {
        let s = format!("{:.2}", x);
        let s = s.trim_end_matches('0').trim_end_matches('.').to_owned();
        if s == "-0" { "0".to_owned() } else { s }
    };
    format!("({}, {}, {})", c(v.x), c(v.y), c(v.z))
}

/// Degrees with at most one decimal: `35`, `12.5`.
pub fn deg(d: f32) -> String {
    let s = format!("{:.1}", d);
    let s = s.strip_suffix(".0").map(str::to_owned).unwrap_or(s);
    if s == "-0" { "0".into() } else { s }
}

/// Three significant digits: `0.216`, `12.5`, `1,250`.
pub fn sig3(x: f64) -> String {
    if x == 0.0 || !x.is_finite() {
        return "0".into();
    }
    let digits = x.abs().log10().floor() as i32;
    let decimals = (2 - digits).max(0) as usize;
    let s = format!("{:.*}", decimals, x);
    if decimals == 0 { thousands_str(&s) } else { s.trim_end_matches('0').trim_end_matches('.').to_owned() }
}

/// `1234567` as `1,234,567`.
pub fn thousands(n: usize) -> String {
    thousands_str(&n.to_string())
}

fn thousands_str(s: &str) -> String {
    let (sign, digits) = s.strip_prefix('-').map_or(("", s), |d| ("-", d));
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    out.push_str(sign);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Area in square meters or square centimeters, whichever reads better.
pub fn area(m2: f64) -> String {
    if m2 < 0.01 { format!("{} cm2", sig3(m2 * 1e4)) } else { format!("{} m2", sig3(m2)) }
}

/// Volume in liters (cubic decimeters) or cubic meters above 1,000 L.
pub fn volume(m3: f64) -> String {
    if m3.abs() >= 1.0 { format!("{} m3", sig3(m3)) } else { format!("{} L", sig3(m3 * 1e3)) }
}

/// Estimated model tokens of a text: runs of letters (about 5 letters per
/// token), digits in groups of three, and every other visible character on
/// its own. Close to subword tokenizers on numeric text, slightly high on
/// prose: good for keeping outputs under a budget.
///
/// Complexity: O(len).
pub fn approx_tokens(text: &str) -> usize {
    #[derive(PartialEq, Clone, Copy)]
    enum Run {
        None,
        Alpha(usize),
        Digit(usize),
    }
    let mut n = 0;
    let mut run = Run::None;
    let close = |r: Run| match r {
        Run::None => 0,
        Run::Alpha(k) => k.div_ceil(5),
        Run::Digit(k) => k.div_ceil(3),
    };
    for c in text.chars() {
        run = match (run, c) {
            (Run::Alpha(k), c) if c.is_alphabetic() || c == '_' => Run::Alpha(k + 1),
            (Run::Digit(k), c) if c.is_ascii_digit() => Run::Digit(k + 1),
            (r, c) => {
                n += close(r);
                if c.is_alphabetic() || c == '_' {
                    Run::Alpha(1)
                } else if c.is_ascii_digit() {
                    Run::Digit(1)
                } else {
                    if !c.is_whitespace() {
                        n += 1;
                    }
                    Run::None
                }
            }
        };
    }
    n + close(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_and_angles() {
        assert_eq!(mm(0.6), "600");
        assert_eq!(mm(0.0004), "0.4");
        assert_eq!(mm(0.008), "8");
        assert_eq!(mm(-0.00001), "0");
        assert_eq!(mm(0.0125), "13");
        assert_eq!(mm0(-0.0004), "0");
        assert_eq!(dims(Vec3::new(0.6, 0.04, 0.62)), "600 x 40 x 620 mm");
        assert_eq!(point(Vec3::new(0.0, 0.3, -0.0)), "(0, 300, 0)");
        assert_eq!(deg(35.0), "35");
        assert_eq!(deg(12.46), "12.5");
    }

    #[test]
    fn numbers() {
        assert_eq!(sig3(0.216), "0.216");
        assert_eq!(sig3(12.54), "12.5");
        assert_eq!(sig3(1250.4), "1,250");
        assert_eq!(thousands(1188), "1,188");
        assert_eq!(area(0.36), "0.36 m2");
        assert_eq!(area(0.0012), "12 cm2");
        assert_eq!(volume(0.216), "216 L");
        assert_eq!(volume(2.5), "2.5 m3");
    }

    #[test]
    fn token_estimate() {
        assert_eq!(approx_tokens(""), 0);
        assert_eq!(approx_tokens("(-119,182 ear_l.tip)"), 9);
        assert_eq!(approx_tokens("silhouette front"), 3);
        assert_eq!(approx_tokens("1234567"), 3);
    }
}
