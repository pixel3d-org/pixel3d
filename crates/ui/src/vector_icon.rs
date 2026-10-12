//! Line icons drawn from SVG path data on a 24 x 24 grid, the way Tabler
//! Icons ship them. The parser covers every path command (M L H V C S Q T A
//! Z, absolute and relative); each icon is flattened once into polylines,
//! cached, and stroked with round ends at any size.

use egui::{Color32, Painter, Pos2, Rect, Stroke, Vec2, pos2, vec2};

/// One flattened subpath on the 24 unit grid.
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    pub points: Vec<Pos2>,
    pub closed: bool,
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl Lexer<'_> {
    fn skip(&mut self) {
        while self.i < self.s.len() && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',') {
            self.i += 1;
        }
    }

    fn done(&mut self) -> bool {
        self.skip();
        self.i >= self.s.len()
    }

    fn command(&mut self) -> Option<u8> {
        self.skip();
        let c = *self.s.get(self.i)?;
        if c.is_ascii_alphabetic() {
            self.i += 1;
            Some(c)
        } else {
            None
        }
    }

    fn at_number(&mut self) -> bool {
        self.skip();
        self.s.get(self.i).is_some_and(|c| matches!(c, b'0'..=b'9' | b'-' | b'+' | b'.'))
    }

    fn number(&mut self) -> Option<f32> {
        self.skip();
        let start = self.i;
        let digits = |l: &mut Self| {
            let from = l.i;
            while l.i < l.s.len() && l.s[l.i].is_ascii_digit() {
                l.i += 1;
            }
            l.i > from
        };
        if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
            self.i += 1;
        }
        let mut any = digits(self);
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            any |= digits(self);
        }
        if !any {
            return None;
        }
        if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            let back = self.i;
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
                self.i += 1;
            }
            if !digits(self) {
                self.i = back;
            }
        }
        std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()
    }

    fn point(&mut self) -> Option<Vec2> {
        Some(vec2(self.number()?, self.number()?))
    }

    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let f = match self.s.get(self.i)? {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.i += 1;
        Some(f)
    }
}

/// Ends the current subpath. Open paths that end where they started (two
/// arcs drawing a circle) count as closed.
fn flush(out: &mut Vec<Outline>, pts: &mut Vec<Pos2>, mut closed: bool) {
    if pts.len() >= 2 {
        if let (Some(a), Some(b)) = (pts.first(), pts.last())
            && a.distance(*b) < 1e-3
        {
            closed = true;
            pts.pop();
        }
        out.push(Outline { points: std::mem::take(pts), closed });
    } else {
        pts.clear();
    }
}

/// Points along a curve: about one per 1.5 grid units of control polygon.
fn steps(len: f32) -> usize {
    ((len / 1.5).ceil() as usize).clamp(2, 24)
}

fn cubic(pts: &mut Vec<Pos2>, p0: Pos2, c1: Pos2, c2: Pos2, p: Pos2) {
    let n = steps(p0.distance(c1) + c1.distance(c2) + c2.distance(p));
    for k in 1..=n {
        let t = k as f32 / n as f32;
        let u = 1.0 - t;
        let v = p0.to_vec2() * (u * u * u) + c1.to_vec2() * (3.0 * u * u * t) + c2.to_vec2() * (3.0 * u * t * t) + p.to_vec2() * (t * t * t);
        pts.push(v.to_pos2());
    }
}

fn quad(pts: &mut Vec<Pos2>, p0: Pos2, c: Pos2, p: Pos2) {
    let n = steps(p0.distance(c) + c.distance(p));
    for k in 1..=n {
        let t = k as f32 / n as f32;
        let u = 1.0 - t;
        pts.push((p0.to_vec2() * (u * u) + c.to_vec2() * (2.0 * u * t) + p.to_vec2() * (t * t)).to_pos2());
    }
}

/// An elliptical arc from `p0` to `p` (SVG endpoint parameterization,
/// converted to center form as in SVG 1.1 appendix F.6.5).
#[allow(clippy::too_many_arguments)] // the SVG arc command's own parameters
fn arc(pts: &mut Vec<Pos2>, p0: Pos2, rx: f32, ry: f32, rot_deg: f32, large: bool, sweep: bool, p: Pos2) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if p0.distance(p) < 1e-6 {
        return;
    }
    if rx < 1e-6 || ry < 1e-6 {
        pts.push(p);
        return;
    }
    let (sin, cos) = rot_deg.to_radians().sin_cos();
    let d = (p0 - p) / 2.0;
    let x1 = cos * d.x + sin * d.y;
    let y1 = -sin * d.x + cos * d.y;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let coef = (num / den).max(0.0).sqrt() * if large == sweep { -1.0 } else { 1.0 };
    let (cx1, cy1) = (coef * rx * y1 / ry, -coef * ry * x1 / rx);
    let mid = (p0.to_vec2() + p.to_vec2()) / 2.0;
    let center = vec2(cos * cx1 - sin * cy1, sin * cx1 + cos * cy1) + mid;
    let angle = |u: Vec2, v: Vec2| (u.x * v.y - u.y * v.x).atan2(u.x * v.x + u.y * v.y);
    let u = vec2((x1 - cx1) / rx, (y1 - cy1) / ry);
    let v = vec2((-x1 - cx1) / rx, (-y1 - cy1) / ry);
    let start = angle(Vec2::X, u);
    let mut delta = angle(u, v);
    if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    }
    let n = ((delta.abs() * rx.max(ry).sqrt() * 2.0).ceil() as usize).clamp(2, 48);
    for k in 1..=n {
        let (s, c) = (start + delta * k as f32 / n as f32).sin_cos();
        let (x, y) = (rx * c, ry * s);
        pts.push((vec2(cos * x - sin * y, sin * x + cos * y) + center).to_pos2());
    }
    // Land exactly on the endpoint.
    if let Some(last) = pts.last_mut() {
        *last = p;
    }
}

/// Parses SVG path data into polylines; `None` when the data is malformed.
///
/// Complexity: O(n) in the length of `d` plus the points emitted (curves
/// emit at most 48 points each).
pub fn parse(d: &str) -> Option<Vec<Outline>> {
    let mut lx = Lexer { s: d.as_bytes(), i: 0 };
    let mut out = Vec::new();
    let mut pts: Vec<Pos2> = Vec::new();
    let (mut cur, mut start) = (Pos2::ZERO, Pos2::ZERO);
    // The previous curve's last control point, for S and T.
    let (mut prev_cubic, mut prev_quad): (Option<Pos2>, Option<Pos2>) = (None, None);
    if lx.done() {
        return Some(out);
    }
    let mut cmd = lx.command()?;
    if !matches!(cmd, b'M' | b'm') {
        return None;
    }
    loop {
        let base = if cmd.is_ascii_lowercase() { cur.to_vec2() } else { Vec2::ZERO };
        let upper = cmd.to_ascii_uppercase();
        if upper != b'M' && upper != b'Z' && pts.is_empty() {
            pts.push(cur);
        }
        let (mut cubic_ctrl, mut quad_ctrl) = (None, None);
        match upper {
            b'M' => {
                let p = (lx.point()? + base).to_pos2();
                flush(&mut out, &mut pts, false);
                pts.push(p);
                (cur, start) = (p, p);
                // Further pairs after a moveto are linetos.
                cmd = if cmd == b'm' { b'l' } else { b'L' };
            }
            b'L' => {
                cur = (lx.point()? + base).to_pos2();
                pts.push(cur);
            }
            b'H' => {
                cur = pos2(lx.number()? + base.x, cur.y);
                pts.push(cur);
            }
            b'V' => {
                cur = pos2(cur.x, lx.number()? + base.y);
                pts.push(cur);
            }
            b'C' | b'S' => {
                let c1 = if upper == b'C' { (lx.point()? + base).to_pos2() } else { prev_cubic.map_or(cur, |c| cur + (cur - c)) };
                let c2 = (lx.point()? + base).to_pos2();
                let p = (lx.point()? + base).to_pos2();
                cubic(&mut pts, cur, c1, c2, p);
                (cur, cubic_ctrl) = (p, Some(c2));
            }
            b'Q' | b'T' => {
                let c = if upper == b'Q' { (lx.point()? + base).to_pos2() } else { prev_quad.map_or(cur, |c| cur + (cur - c)) };
                let p = (lx.point()? + base).to_pos2();
                quad(&mut pts, cur, c, p);
                (cur, quad_ctrl) = (p, Some(c));
            }
            b'A' => {
                let (rx, ry, rot) = (lx.number()?, lx.number()?, lx.number()?);
                let (large, sweep) = (lx.flag()?, lx.flag()?);
                let p = (lx.point()? + base).to_pos2();
                arc(&mut pts, cur, rx, ry, rot, large, sweep, p);
                cur = p;
            }
            b'Z' => {
                flush(&mut out, &mut pts, true);
                cur = start;
            }
            _ => return None,
        }
        (prev_cubic, prev_quad) = (cubic_ctrl, quad_ctrl);
        if lx.at_number() {
            if upper == b'Z' {
                return None;
            }
            continue;
        }
        if lx.done() {
            break;
        }
        cmd = lx.command()?;
    }
    flush(&mut out, &mut pts, false);
    Some(out)
}

/// Parses every path of an icon; malformed paths are skipped.
///
/// Complexity: O(total path length).
pub fn parse_all(paths: &[&str]) -> Vec<Outline> {
    paths.iter().filter_map(|d| parse(d)).flatten().collect()
}

/// The stroke width for an icon drawn `size` pixels wide: thinner than the
/// source's 2 units, for a light look at toolbar sizes.
pub fn stroke_width(size: f32) -> f32 {
    (size / 24.0 * 1.75).clamp(1.1, 2.0)
}

/// Draws outlines scaled from the 24 unit grid into `rect`, stroked with
/// round ends, or filled when `fill` is set.
///
/// Complexity: O(points).
pub fn paint(painter: &Painter, rect: Rect, outlines: &[Outline], color: Color32, fill: bool) {
    let size = rect.width().min(rect.height());
    let s = size / 24.0;
    let origin = rect.center() - vec2(12.0, 12.0) * s;
    let w = stroke_width(size);
    let map = |p: &Pos2| origin + p.to_vec2() * s;
    for o in outlines {
        let points: Vec<Pos2> = o.points.iter().map(map).collect();
        if fill {
            painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
            continue;
        }
        if !o.closed
            && let (Some(a), Some(b)) = (points.first(), points.last())
        {
            painter.circle_filled(*a, w / 2.0, color);
            painter.circle_filled(*b, w / 2.0, color);
        }
        let shape = if o.closed { egui::Shape::closed_line(points, Stroke::new(w, color)) } else { egui::Shape::line(points, Stroke::new(w, color)) };
        painter.add(shape);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Pos2, b: Pos2) -> bool {
        a.distance(b) < 1e-3
    }

    #[test]
    fn lines_and_relative_commands() {
        let o = parse("M3 4h10v2l-2 2H4V5z").unwrap();
        assert_eq!(o.len(), 1);
        assert!(o[0].closed);
        let p = &o[0].points;
        assert!(near(p[0], pos2(3.0, 4.0)) && near(p[1], pos2(13.0, 4.0)) && near(p[2], pos2(13.0, 6.0)) && near(p[3], pos2(11.0, 8.0)));
        assert!(near(p[4], pos2(4.0, 8.0)) && near(p[5], pos2(4.0, 5.0)));
        // A moveto followed by pairs draws lines; two subpaths.
        let o = parse("M1 1 2 2m3 0l1 1").unwrap();
        assert_eq!(o.len(), 2);
        assert!(near(o[0].points[1], pos2(2.0, 2.0)) && near(o[1].points[0], pos2(5.0, 2.0)));
        // Compact numbers: "-.5.5" is two numbers.
        let o = parse("M0 0l-.5.5").unwrap();
        assert!(near(o[0].points[1], pos2(-0.5, 0.5)));
    }

    #[test]
    fn arcs_draw_circles() {
        // Tabler's circle idiom: two half arcs around (12, 12), radius 9.
        let o = parse("M12 12m-9 0a9 9 0 1 0 18 0a9 9 0 1 0 -18 0").unwrap();
        assert_eq!(o.len(), 1, "the lone moveto point is dropped");
        assert!(o[0].closed, "ends where it starts");
        for p in &o[0].points {
            assert!((p.distance(pos2(12.0, 12.0)) - 9.0).abs() < 1e-3);
        }
        assert!(o[0].points.len() >= 24, "smooth enough");
        // An arc too small for its endpoints is scaled up to reach them.
        let o = parse("M0 0a1 1 0 0 1 10 0").unwrap();
        assert!(near(*o[0].points.last().unwrap(), pos2(10.0, 0.0)));
        assert!(o[0].points.iter().all(|p| (p.distance(pos2(5.0, 0.0)) - 5.0).abs() < 1e-3));
    }

    #[test]
    fn curves_pass_through_their_ends() {
        let o = parse("M0 0c0 10 10 10 10 0s10 -10 10 0q5 5 10 0t10 0").unwrap();
        let p = &o[0].points;
        assert!(p.iter().any(|q| near(*q, pos2(10.0, 0.0))));
        assert!(p.iter().any(|q| near(*q, pos2(20.0, 0.0))));
        assert!(near(*p.last().unwrap(), pos2(40.0, 0.0)));
        // The smooth curve mirrors the previous control point: it dips below.
        assert!(p.iter().any(|q| q.x > 10.0 && q.x < 20.0 && q.y < -5.0));
    }

    #[test]
    fn malformed_data_is_refused() {
        for bad in ["L1 1", "M1", "M1 1 Z 2", "M1 1 K 2 2", "M0 0a1 1 0 2 0 1 1"] {
            assert!(parse(bad).is_none(), "{bad}");
        }
        assert_eq!(parse("").map(|o| o.len()), Some(0));
    }

    #[test]
    fn every_icon_parses_inside_the_grid() {
        for &icon in crate::icons::Icon::ALL {
            let outlines = crate::icons::outlines(icon);
            assert!(!outlines.is_empty(), "{icon:?}");
            for o in outlines {
                assert!(o.points.iter().all(|p| (-0.5..=24.5).contains(&p.x) && (-0.5..=24.5).contains(&p.y)), "{icon:?}");
            }
        }
    }
}
