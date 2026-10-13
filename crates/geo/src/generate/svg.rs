//! SVG path data as outlines (agents write SVG paths fluently; project spec
//! 17.4). Units are meters and y points up, as in the outline's view. The
//! commands M, L, H, V, C, S, Q, T and Z are understood in absolute and
//! relative forms; curves are flattened to straight segments no longer than
//! `step`. Arcs (A) are not: write them as cubic curves.

use glam::DVec2;

use crate::prim::PrimError;

fn bad(message: impl Into<String>) -> PrimError {
    PrimError::InvalidParam { param: "path", value: "invalid".into(), reason: message.into() }
}

/// Tokens of path data: command letters and numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok {
    Cmd(char),
    Num(f64),
}

/// Splits path data into commands and numbers (commas and spaces separate;
/// `-` and a second `.` start a new number).
///
/// Complexity: O(length).
fn tokens(d: &str) -> Result<Vec<Tok>, PrimError> {
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
            let mut dot = false;
            let mut exp = false;
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
            let s = &d[start..i];
            let v: f64 = s.parse().map_err(|_| bad(format!("`{s}` at {start} is not a number")))?;
            if !v.is_finite() {
                return Err(bad(format!("`{s}` is not finite")));
            }
            out.push(Tok::Num(v));
            if i == start {
                return Err(bad(format!("unexpected `{c}` at {start}")));
            }
        }
    }
    Ok(out)
}

/// The number at `i`, advancing past it.
fn number(toks: &[Tok], i: &mut usize, cmd: char) -> Result<f64, PrimError> {
    match toks.get(*i) {
        Some(Tok::Num(v)) => {
            *i += 1;
            Ok(*v)
        }
        _ => Err(bad(format!("a number is missing after `{cmd}`"))),
    }
}

/// Points of a cubic Bezier after `p0`, at most `step` apart.
fn cubic(out: &mut Vec<DVec2>, p0: DVec2, c1: DVec2, c2: DVec2, p3: DVec2, step: f64) {
    let len = p0.distance(c1) + c1.distance(c2) + c2.distance(p3);
    let n = ((len / step).ceil() as usize).clamp(1, 256);
    for k in 1..=n {
        let t = k as f64 / n as f64;
        let u = 1.0 - t;
        out.push(p0 * (u * u * u) + c1 * (3.0 * u * u * t) + c2 * (3.0 * u * t * t) + p3 * (t * t * t));
    }
}

/// Parses path data into closed outlines (one per subpath; a subpath is
/// closed by Z or by ending where it started, and an open one is closed
/// with a straight line). Repeated points are dropped.
///
/// Complexity: O(length + output points).
pub fn parse_path(d: &str, step: f64) -> Result<Vec<Vec<DVec2>>, PrimError> {
    if !(step.is_finite() && step > 0.0) {
        return Err(bad("the flattening step must be positive"));
    }
    let toks = tokens(d)?;
    let mut out: Vec<Vec<DVec2>> = Vec::new();
    let mut cur: Vec<DVec2> = Vec::new();
    let mut pos = DVec2::ZERO;
    let mut start = DVec2::ZERO;
    // The last control point, for S and T reflections.
    let mut last_c: Option<(char, DVec2)> = None;
    let mut i = 0;
    let mut cmd = ' ';
    let finish = |cur: &mut Vec<DVec2>, out: &mut Vec<Vec<DVec2>>| {
        let mut c = std::mem::take(cur);
        c.dedup_by(|a, b| a.distance(*b) < 1e-12);
        if c.len() > 1 && c[0].distance(c[c.len() - 1]) < 1e-12 {
            c.pop();
        }
        if c.len() >= 3 {
            out.push(c);
        }
    };
    while i < toks.len() {
        if let Tok::Cmd(c) = toks[i] {
            if cmd == ' ' && c != 'M' && c != 'm' {
                return Err(bad("path data must start with M"));
            }
            cmd = c;
            i += 1;
            if c == 'Z' || c == 'z' {
                finish(&mut cur, &mut out);
                pos = start;
                last_c = None;
                continue;
            }
        } else if cmd == ' ' {
            return Err(bad("path data must start with M"));
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { pos } else { DVec2::ZERO };
        let c = cmd;
        let num = |i: &mut usize| number(&toks, i, c);
        let pt = |i: &mut usize| -> Result<DVec2, PrimError> { Ok(base + DVec2::new(number(&toks, i, c)?, number(&toks, i, c)?)) };
        match cmd.to_ascii_uppercase() {
            'M' => {
                finish(&mut cur, &mut out);
                pos = pt(&mut i)?;
                start = pos;
                cur.push(pos);
                // Further pairs after M are line-tos.
                cmd = if rel { 'l' } else { 'L' };
                last_c = None;
            }
            'L' => {
                pos = pt(&mut i)?;
                cur.push(pos);
                last_c = None;
            }
            'H' => {
                let x = num(&mut i)?;
                pos.x = if rel { pos.x + x } else { x };
                cur.push(pos);
                last_c = None;
            }
            'V' => {
                let y = num(&mut i)?;
                pos.y = if rel { pos.y + y } else { y };
                cur.push(pos);
                last_c = None;
            }
            'C' => {
                let (c1, c2, p) = (pt(&mut i)?, pt(&mut i)?, pt(&mut i)?);
                cubic(&mut cur, pos, c1, c2, p, step);
                last_c = Some(('C', c2));
                pos = p;
            }
            'S' => {
                let c1 = match last_c {
                    Some(('C', c)) => pos * 2.0 - c,
                    _ => pos,
                };
                let (c2, p) = (pt(&mut i)?, pt(&mut i)?);
                cubic(&mut cur, pos, c1, c2, p, step);
                last_c = Some(('C', c2));
                pos = p;
            }
            'Q' | 'T' => {
                let q = if cmd.eq_ignore_ascii_case(&'Q') {
                    pt(&mut i)?
                } else {
                    match last_c {
                        Some(('Q', c)) => pos * 2.0 - c,
                        _ => pos,
                    }
                };
                let p = pt(&mut i)?;
                // A quadratic is the cubic with controls 2/3 of the way to q.
                cubic(&mut cur, pos, pos + (q - pos) * (2.0 / 3.0), p + (q - p) * (2.0 / 3.0), p, step);
                last_c = Some(('Q', q));
                pos = p;
            }
            'A' => return Err(bad("arcs (A) are not supported; write the arc as cubic curves (C)")),
            other => return Err(bad(format!("`{other}` is not a path command"))),
        }
    }
    finish(&mut cur, &mut out);
    if out.is_empty() {
        return Err(bad("the path encloses nothing (give at least three distinct points)"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn lines_curves_and_relative_commands() {
        let o = parse_path("M 0 0 L 0.1 0 V 0.1 H 0 Z", 0.01).unwrap();
        assert_eq!(o, vec![vec![DVec2::ZERO, DVec2::new(0.1, 0.0), DVec2::new(0.1, 0.1), DVec2::new(0.0, 0.1)]]);
        let r = parse_path("m0,0 l.1,0 0,.1 -.1,0z", 0.01).unwrap();
        assert_eq!(r[0].len(), 4);
        assert!(r[0][2].distance(DVec2::new(0.1, 0.1)) < 1e-12);
        // A curve flattens into short steps that stay on it.
        let c = parse_path("M 0 0 C 0 0.1 0.1 0.1 0.1 0 Z", 0.005).unwrap();
        assert!(c[0].len() > 20);
        assert!(c[0].iter().all(|p| p.y >= -1e-12 && p.y <= 0.0751));
        // Two subpaths.
        assert_eq!(parse_path("M0 0 L1 0 L1 1 Z M2 0 L3 0 L3 1 Z", 0.1).unwrap().len(), 2);
    }

    #[test]
    fn errors_name_the_problem() {
        assert!(parse_path("L 0 0", 0.1).unwrap_err().to_string().contains("start with M"));
        assert!(parse_path("M 0 0 A 1 1 0 0 1 1 1", 0.1).unwrap_err().to_string().contains("arcs"));
        assert!(parse_path("M 0 0 L 1", 0.1).unwrap_err().to_string().contains("missing"));
        assert!(parse_path("M 0 0 L 1 0", 0.1).unwrap_err().to_string().contains("encloses nothing"));
    }
}
