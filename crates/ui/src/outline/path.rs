//! Editable outlines: closed contours of points with Bézier handles, read
//! from and written back to what `gen.shell` and `sdf.silhouettes` take
//! (SVG path data in meters, or `[[x, y], ...]` point lists). The SVG
//! commands M, L, H, V, C, S, Q, T and Z are read in absolute and relative
//! forms (quadratic curves become the equal cubic ones); arcs are refused,
//! as the ops refuse them. Paths are written with M, L, C and Z.

use glam::DVec2;
use serde_json::{Value, json};

/// One point of a contour and its handles, as offsets from the point (zero
/// means no handle: the segment on that side is straight there).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    pub p: DVec2,
    pub h_in: DVec2,
    pub h_out: DVec2,
    /// Moving one handle turns the other to stay opposite.
    pub smooth: bool,
}

impl Node {
    pub fn corner(p: DVec2) -> Node {
        Node { p, h_in: DVec2::ZERO, h_out: DVec2::ZERO, smooth: false }
    }
}

/// A closed contour.
pub type Contour = Vec<Node>;

/// One outline: closed contours (a path may hold several; the ops fill
/// them even-odd, so inner ones are holes).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outline {
    pub contours: Vec<Contour>,
    /// It was written as a point list: written back as one while it has a
    /// single contour without handles.
    pub points_form: bool,
}

/// A segment of a contour: straight, or a cubic Bézier curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    Line(DVec2, DVec2),
    Cubic(DVec2, DVec2, DVec2, DVec2),
}

impl Seg {
    /// The point at `t` in [0, 1].
    pub fn at(self, t: f64) -> DVec2 {
        match self {
            Seg::Line(a, b) => a.lerp(b, t),
            Seg::Cubic(a, c1, c2, b) => {
                let u = 1.0 - t;
                a * (u * u * u) + c1 * (3.0 * u * u * t) + c2 * (3.0 * u * t * t) + b * (t * t * t)
            }
        }
    }

    /// Points along the segment, `a` left out (`n` for a curve, one for a
    /// line).
    ///
    /// Complexity: O(n).
    pub fn flatten_into(self, n: usize, out: &mut Vec<DVec2>) {
        match self {
            Seg::Line(_, b) => out.push(b),
            Seg::Cubic(..) => out.extend((1..=n).map(|i| self.at(i as f64 / n as f64))),
        }
    }

    /// The parameter of the point nearest `q`, and its distance.
    ///
    /// Complexity: O(1) (a fixed number of samples and refinements).
    pub fn nearest(self, q: DVec2) -> (f64, f64) {
        match self {
            Seg::Line(a, b) => {
                let d = b - a;
                let t = if d.length_squared() > 0.0 { ((q - a).dot(d) / d.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                (t, self.at(t).distance(q))
            }
            Seg::Cubic(..) => {
                const N: usize = 32;
                let mut best = (0.0, f64::MAX);
                for i in 0..=N {
                    let t = i as f64 / N as f64;
                    let d = self.at(t).distance(q);
                    if d < best.1 {
                        best = (t, d);
                    }
                }
                // Refine around the best sample.
                let mut step = 0.5 / N as f64;
                for _ in 0..12 {
                    for t in [best.0 - step, best.0 + step] {
                        let t = t.clamp(0.0, 1.0);
                        let d = self.at(t).distance(q);
                        if d < best.1 {
                            best = (t, d);
                        }
                    }
                    step *= 0.5;
                }
                best
            }
        }
    }
}

/// The segment from node `i` to the next one (wrapping).
pub fn segment(c: &Contour, i: usize) -> Seg {
    let a = c[i];
    let b = c[(i + 1) % c.len()];
    if a.h_out == DVec2::ZERO && b.h_in == DVec2::ZERO { Seg::Line(a.p, b.p) } else { Seg::Cubic(a.p, a.p + a.h_out, b.p + b.h_in, b.p) }
}

/// A contour as a closed polyline (curves in `n` pieces).
///
/// Complexity: O(nodes x n).
pub fn flatten(c: &Contour, n: usize) -> Vec<DVec2> {
    let mut out = Vec::with_capacity(c.len() * n);
    if let Some(last) = c.last() {
        out.push(last.p);
    }
    for i in 0..c.len() {
        segment(c, (i + c.len() - 1) % c.len()).flatten_into(n, &mut out);
    }
    out
}

/// The bounds of contours (curves included), or `None` when empty.
///
/// Complexity: O(nodes).
pub fn bounds<'a>(contours: impl IntoIterator<Item = &'a Contour>) -> Option<(DVec2, DVec2)> {
    let mut b: Option<(DVec2, DVec2)> = None;
    for c in contours {
        for p in flatten(c, 12) {
            b = Some(b.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
        }
    }
    b
}

/// Inserts a point on segment `i` at `t`, keeping the shape (a curve is
/// split in two). Returns the new point's index.
pub fn insert(c: &mut Contour, i: usize, t: f64) -> usize {
    let n = c.len();
    let j = (i + 1) % n;
    match segment(c, i) {
        Seg::Line(a, b) => {
            c.insert(i + 1, Node::corner(a.lerp(b, t)));
        }
        Seg::Cubic(a, c1, c2, b) => {
            // De Casteljau.
            let (p01, p12, p23) = (a.lerp(c1, t), c1.lerp(c2, t), c2.lerp(b, t));
            let (p012, p123) = (p01.lerp(p12, t), p12.lerp(p23, t));
            let m = p012.lerp(p123, t);
            c[i].h_out = p01 - a;
            c[j].h_in = p23 - b;
            c.insert(i + 1, Node { p: m, h_in: p012 - m, h_out: p123 - m, smooth: true });
        }
    }
    i + 1
}

/// Turns a smooth point into a corner (no handles), or a corner smooth:
/// its handles turned opposite along their mean direction (or, without
/// handles, along the neighbours' direction, a third of the way to each).
pub fn toggle_smooth(c: &mut Contour, i: usize) {
    let n = c.len();
    if n < 3 {
        return;
    }
    let (prev, next) = (c[(i + n - 1) % n].p, c[(i + 1) % n].p);
    let node = &mut c[i];
    if node.smooth {
        *node = Node::corner(node.p);
        return;
    }
    let along = (node.h_out - node.h_in).normalize_or_zero();
    let d = if along == DVec2::ZERO { (next - prev).normalize_or_zero() } else { along };
    let len = |h: DVec2, to: DVec2| if h == DVec2::ZERO { node.p.distance(to) / 3.0 } else { h.length() };
    let (li, lo) = (len(node.h_in, prev), len(node.h_out, next));
    node.h_in = -d * li;
    node.h_out = d * lo;
    node.smooth = true;
}

/// Sets a handle (`out` or in) of node `i`; a smooth point turns the other
/// one opposite, keeping its length.
pub fn set_handle(c: &mut Contour, i: usize, out: bool, h: DVec2) {
    let node = &mut c[i];
    let (this, other) = if out { (&mut node.h_out, &mut node.h_in) } else { (&mut node.h_in, &mut node.h_out) };
    *this = h;
    if node.smooth && h != DVec2::ZERO {
        let len = if *other == DVec2::ZERO { h.length() } else { other.length() };
        *other = -h.normalize_or_zero() * len;
    }
}

fn opposite(a: DVec2, b: DVec2) -> bool {
    let (la, lb) = (a.length(), b.length());
    la > 0.0 && lb > 0.0 && a.perp_dot(b).abs() <= 1e-3 * la * lb && a.dot(b) < 0.0
}

// ---- reading --------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok {
    Cmd(char),
    Num(f64),
}

/// Commands and numbers of path data.
///
/// Complexity: O(length).
fn tokens(d: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let b = d.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_whitespace() || c == ',' {
            i += 1;
        } else if c.is_ascii_alphabetic() && c != 'e' && c != 'E' {
            out.push(Tok::Cmd(c));
            i += 1;
        } else {
            let start = i;
            let (mut dot, mut exp) = (false, false);
            if b[i] == b'-' || b[i] == b'+' {
                i += 1;
            }
            while i < b.len() {
                let ch = b[i];
                if ch.is_ascii_digit() {
                    i += 1;
                } else if ch == b'.' && !dot && !exp {
                    dot = true;
                    i += 1;
                } else if (ch == b'e' || ch == b'E') && !exp {
                    exp = true;
                    i += 1;
                    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            if i == start {
                return Err(format!("unexpected `{c}` at {start}"));
            }
            let s = &d[start..i];
            let v: f64 = s.parse().map_err(|_| format!("`{s}` is not a number"))?;
            if !v.is_finite() {
                return Err(format!("`{s}` is not finite"));
            }
            out.push(Tok::Num(v));
        }
    }
    Ok(out)
}

/// Contours being read: the open one, closed when a new one starts.
#[derive(Default)]
struct Reader {
    done: Vec<Contour>,
    open: Contour,
}

impl Reader {
    fn close(&mut self) {
        let mut c = std::mem::take(&mut self.open);
        // A closing point on the start is the start.
        if c.len() > 1
            && let (Some(first), Some(last)) = (c.first().copied(), c.last().copied())
            && first.p.distance(last.p) <= 1e-9
        {
            c[0].h_in = last.h_in;
            c.pop();
        }
        for node in &mut c {
            node.smooth = opposite(node.h_in, node.h_out);
        }
        if !c.is_empty() {
            self.done.push(c);
        }
    }
}

/// Reads SVG path data (meters, y up) into contours.
///
/// Complexity: O(length).
pub fn parse_svg(d: &str) -> Result<Vec<Contour>, String> {
    let toks = tokens(d)?;
    let mut r = Reader::default();
    let (mut cur, mut start) = (DVec2::ZERO, DVec2::ZERO);
    // The last curve's second control point, for S and T reflections.
    let (mut last_c, mut last_q): (Option<DVec2>, Option<DVec2>) = (None, None);
    let mut i = 0;
    let mut cmd: Option<char> = None;
    let num = |i: &mut usize, c: char| -> Result<f64, String> {
        match toks.get(*i) {
            Some(Tok::Num(v)) => {
                *i += 1;
                Ok(*v)
            }
            _ => Err(format!("`{c}` needs more numbers")),
        }
    };
    while i < toks.len() {
        let c = match toks[i] {
            Tok::Cmd(c) => {
                i += 1;
                c
            }
            // Numbers after a command repeat it (after M, as L).
            Tok::Num(_) => match cmd {
                Some('M') => 'L',
                Some('m') => 'l',
                Some(c) if !matches!(c, 'Z' | 'z') => c,
                _ => return Err("path data must start with M".to_owned()),
            },
        };
        if cmd.is_none() && !matches!(c, 'M' | 'm') {
            return Err("path data must start with M".to_owned());
        }
        cmd = Some(c);
        let rel = c.is_ascii_lowercase();
        let base = if rel { cur } else { DVec2::ZERO };
        let pt = |i: &mut usize| -> Result<DVec2, String> { Ok(base + DVec2::new(num(i, c)?, num(i, c)?)) };
        let (mut next_c, mut next_q) = (None, None);
        match c.to_ascii_uppercase() {
            'M' => {
                r.close();
                cur = pt(&mut i)?;
                start = cur;
                r.open.push(Node::corner(cur));
            }
            'L' => {
                cur = pt(&mut i)?;
                r.open.push(Node::corner(cur));
            }
            'H' => {
                let x = num(&mut i, c)? + if rel { cur.x } else { 0.0 };
                cur = DVec2::new(x, cur.y);
                r.open.push(Node::corner(cur));
            }
            'V' => {
                let y = num(&mut i, c)? + if rel { cur.y } else { 0.0 };
                cur = DVec2::new(cur.x, y);
                r.open.push(Node::corner(cur));
            }
            'C' | 'S' | 'Q' | 'T' => {
                let up = c.to_ascii_uppercase();
                let (c1, c2, p) = match up {
                    'C' => (pt(&mut i)?, pt(&mut i)?, pt(&mut i)?),
                    'S' => {
                        let c1 = last_c.map_or(cur, |k| cur * 2.0 - k);
                        (c1, pt(&mut i)?, pt(&mut i)?)
                    }
                    _ => {
                        let q = if up == 'Q' { pt(&mut i)? } else { last_q.map_or(cur, |k| cur * 2.0 - k) };
                        let p = pt(&mut i)?;
                        next_q = Some(q);
                        (cur + (q - cur) * (2.0 / 3.0), p + (q - p) * (2.0 / 3.0), p)
                    }
                };
                if r.open.is_empty() {
                    r.open.push(Node::corner(cur));
                }
                if let Some(prev) = r.open.last_mut() {
                    prev.h_out = c1 - prev.p;
                }
                r.open.push(Node { p, h_in: c2 - p, h_out: DVec2::ZERO, smooth: false });
                if matches!(up, 'C' | 'S') {
                    next_c = Some(c2);
                }
                cur = p;
            }
            'Z' => {
                r.close();
                cur = start;
            }
            'A' => return Err("arcs (A) are not supported: write them as cubic curves (C)".to_owned()),
            other => return Err(format!("unknown command `{other}`")),
        }
        (last_c, last_q) = (next_c, next_q);
    }
    r.close();
    if r.done.is_empty() {
        return Err("the path has no points".to_owned());
    }
    Ok(r.done)
}

/// A length written as a number (meters) or with a unit (`"2cm"`).
fn length(v: &Value) -> Option<f64> {
    serde_json::from_value::<p3d_ops::params::Length>(v.clone()).ok().map(|l| l.0)
}

impl Outline {
    /// Reads an outline parameter: SVG path data or a point list.
    ///
    /// Complexity: O(size of the value).
    pub fn from_value(v: &Value) -> Result<Outline, String> {
        match v {
            Value::String(d) => Ok(Outline { contours: parse_svg(d)?, points_form: false }),
            Value::Array(points) => {
                let nodes = points
                    .iter()
                    .map(|p| {
                        let a = p.as_array().filter(|a| a.len() == 2).ok_or("each point is [x, y]")?;
                        Ok(Node::corner(DVec2::new(length(&a[0]).ok_or("x is not a length")?, length(&a[1]).ok_or("y is not a length")?)))
                    })
                    .collect::<Result<Vec<_>, &str>>()?;
                Ok(Outline { contours: vec![nodes], points_form: true })
            }
            _ => Err("an outline is SVG path data or a list of points".to_owned()),
        }
    }

    /// The outline as the ops take it: a point list when it came as one
    /// and still fits, else SVG path data (M, L, C, Z).
    ///
    /// Complexity: O(nodes).
    pub fn to_value(&self) -> Value {
        let plain = self.contours.len() == 1 && self.contours.iter().flatten().all(|n| n.h_in == DVec2::ZERO && n.h_out == DVec2::ZERO);
        if self.points_form && plain {
            return json!(self.contours[0].iter().map(|n| [round(n.p.x), round(n.p.y)]).collect::<Vec<_>>());
        }
        Value::String(self.to_svg())
    }

    /// SVG path data with M, L, C and Z, numbers in meters to a micrometre.
    ///
    /// Complexity: O(nodes).
    pub fn to_svg(&self) -> String {
        let mut s = String::new();
        let put = |p: DVec2, s: &mut String| {
            s.push(' ');
            s.push_str(&num(p.x));
            s.push(' ');
            s.push_str(&num(p.y));
        };
        for c in &self.contours {
            let Some(first) = c.first() else { continue };
            if !s.is_empty() {
                s.push(' ');
            }
            s.push('M');
            put(first.p, &mut s);
            for i in 0..c.len() {
                let last = i + 1 == c.len();
                match segment(c, i) {
                    // The closing line is the Z.
                    Seg::Line(_, b) if !last => {
                        s.push_str(" L");
                        put(b, &mut s);
                    }
                    Seg::Line(..) => {}
                    Seg::Cubic(_, c1, c2, b) => {
                        s.push_str(" C");
                        put(c1, &mut s);
                        put(c2, &mut s);
                        put(b, &mut s);
                    }
                }
            }
            s.push_str(" Z");
        }
        s
    }

    /// Bounds of every contour.
    ///
    /// Complexity: O(nodes).
    pub fn bounds(&self) -> Option<(DVec2, DVec2)> {
        bounds(&self.contours)
    }

    /// A rectangle from `lo` to `hi`, as corners.
    pub fn rect(lo: DVec2, hi: DVec2) -> Outline {
        let c = vec![Node::corner(lo), Node::corner(DVec2::new(hi.x, lo.y)), Node::corner(hi), Node::corner(DVec2::new(lo.x, hi.y))];
        Outline { contours: vec![c], points_form: false }
    }
}

/// A coordinate rounded to a micrometre.
fn round(v: f64) -> f64 {
    let r = (v * 1e6).round() / 1e6;
    if r == 0.0 { 0.0 } else { r }
}

/// A coordinate as short text: meters to a micrometre, no trailing zeros.
fn num(v: f64) -> String {
    let s = format!("{:.6}", round(v));
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_owned() } else { s.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: DVec2, b: DVec2) -> bool {
        a.distance(b) < 1e-9
    }

    #[test]
    fn lines_and_closing_points_read_as_corners() {
        let c = parse_svg("M 0 0 L 0.1 0 L 0.1 0.2 L 0 0.2 L 0 0 Z").unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].len(), 4, "the closing point is the start");
        assert!(c[0].iter().all(|n| !n.smooth && n.h_in == DVec2::ZERO));
        // Relative, H and V, implicit L after M, no Z.
        let r = parse_svg("m 0 0 0.1 0 v 0.2 h -0.1").unwrap();
        let ps: Vec<DVec2> = r[0].iter().map(|n| n.p).collect();
        assert!(close(ps[1], DVec2::new(0.1, 0.0)) && close(ps[2], DVec2::new(0.1, 0.2)) && close(ps[3], DVec2::new(0.0, 0.2)));
    }

    #[test]
    fn curves_keep_their_handles_and_quadratics_become_cubics() {
        let c = parse_svg("M 0 0 C 0.03 0.02 0.025 0.07 0 0.1 C -0.025 0.07 -0.03 0.02 0 0 Z").unwrap();
        assert_eq!(c[0].len(), 2);
        assert!(close(c[0][0].h_out, DVec2::new(0.03, 0.02)));
        assert!(close(c[0][0].h_in, DVec2::new(-0.03, 0.02)), "the closing curve's handle lands on the start");
        assert!(close(c[0][1].h_in, DVec2::new(0.025, -0.03)));
        // S reflects the last control point; T the last quadratic one.
        let s = parse_svg("M 0 0 C 0 1 1 1 1 0 S 2 -1 2 0").unwrap();
        assert!(close(s[0][1].h_out, DVec2::new(0.0, -1.0)), "{:?}", s[0][1]);
        assert!(s[0][1].smooth);
        let q = parse_svg("M 0 0 Q 1 1 2 0 Z").unwrap();
        assert!(close(q[0][0].h_out, DVec2::new(2.0 / 3.0, 2.0 / 3.0)));
        assert!(parse_svg("M 0 0 A 1 1 0 0 1 1 1").unwrap_err().contains("arcs"));
        assert!(parse_svg("L 1 1").is_err());
    }

    #[test]
    fn paths_round_trip_through_m_l_c_z() {
        let d = "M 0 0 C 0.03 0.02 0.025 0.07 0 0.1 L -0.02 0.05 Z M 0.005 0.03 L 0.01 0.03 L 0.0075 0.04 Z";
        let o = Outline::from_value(&json!(d)).unwrap();
        let back = o.to_svg();
        assert_eq!(back, "M 0 0 C 0.03 0.02 0.025 0.07 0 0.1 L -0.02 0.05 Z M 0.005 0.03 L 0.01 0.03 L 0.0075 0.04 Z");
        assert_eq!(Outline::from_value(&json!(back)).unwrap(), o);
        // A curve back to the start ends with C to it.
        let c = Outline::from_value(&json!("M 0 0 C 1 0 1 1 0 1 C -1 1 -1 0 0 0 Z")).unwrap();
        assert_eq!(c.to_svg(), "M 0 0 C 1 0 1 1 0 1 C -1 1 -1 0 0 0 Z");
    }

    #[test]
    fn point_lists_stay_point_lists_until_curved() {
        let mut o = Outline::from_value(&json!([[0, 0], ["10cm", 0], [0.1, 0.2]])).unwrap();
        assert!(o.points_form);
        assert_eq!(o.to_value(), json!([[0.0, 0.0], [0.1, 0.0], [0.1, 0.2]]));
        toggle_smooth(&mut o.contours[0], 1);
        assert!(o.to_value().is_string(), "a curve needs path data");
    }

    #[test]
    fn inserting_on_a_curve_keeps_its_shape() {
        let mut c = parse_svg("M 0 0 C 0 1 1 1 1 0 Z").unwrap().remove(0);
        let before: Vec<DVec2> = (0..=10).map(|i| segment(&c, 0).at(i as f64 / 10.0)).collect();
        let k = insert(&mut c, 0, 0.5);
        assert_eq!((k, c.len()), (1, 3));
        assert!(close(c[1].p, DVec2::new(0.5, 0.75)));
        // The first half matches the old curve's first half.
        for (i, p) in before.iter().take(6).enumerate() {
            assert!(segment(&c, 0).at(i as f64 / 5.0).distance(*p) < 1e-9);
        }
        let (t, d) = segment(&c, 1).nearest(DVec2::new(1.0, 0.0));
        assert!((t - 1.0).abs() < 1e-6 && d < 1e-9);
        // A line splits at the point.
        let mut l = parse_svg("M 0 0 L 1 0 L 0 1 Z").unwrap().remove(0);
        insert(&mut l, 0, 0.25);
        assert!(close(l[1].p, DVec2::new(0.25, 0.0)) && l[1] == Node::corner(l[1].p));
    }

    #[test]
    fn smooth_points_keep_handles_opposite() {
        let mut c = parse_svg("M 0 0 L 1 0 L 1 1 L 0 1 Z").unwrap().remove(0);
        toggle_smooth(&mut c, 1);
        assert!(c[1].smooth && opposite(c[1].h_in, c[1].h_out));
        set_handle(&mut c, 1, true, DVec2::new(0.0, 0.5));
        assert!(close(c[1].h_in.normalize(), DVec2::new(0.0, -1.0)), "{:?}", c[1].h_in);
        toggle_smooth(&mut c, 1);
        assert_eq!(c[1], Node::corner(DVec2::new(1.0, 0.0)));
        let (lo, hi) = bounds(&[c]).unwrap();
        assert!(close(lo, DVec2::ZERO) && close(hi, DVec2::ONE));
    }
}
