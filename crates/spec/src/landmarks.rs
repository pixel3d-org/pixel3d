//! Landmark tables (project spec 17.8, ADR 0010): generic extremes every
//! part has, and category landmarks as directions on the part's bounding
//! ellipsoid (x is the character's left, y up, z front), projected onto the
//! surface by a ray from outside. Category proportions are those of a
//! typical blockout; parts can add or override landmarks by name.

/// Generic landmarks: the surface point outside the part's center in each
/// direction.
pub const GENERIC: [(&str, [f32; 3]); 6] = [
    ("top", [0.0, 1.0, 0.0]),
    ("bottom", [0.0, -1.0, 0.0]),
    ("front", [0.0, 0.0, 1.0]),
    ("back", [0.0, 0.0, -1.0]),
    ("left", [1.0, 0.0, 0.0]),
    ("right", [-1.0, 0.0, 0.0]),
];

/// A humanoid head, chin to crown: eyes near the middle, ears level with
/// the eyes and slightly behind the center.
const HUMANOID_HEAD: [(&str, [f32; 3]); 21] = [
    ("crown", [0.0, 1.0, 0.0]),
    ("chin", [0.0, -0.95, 0.6]),
    ("nose_root", [0.0, 0.08, 1.0]),
    ("nose_base", [0.0, -0.3, 1.0]),
    ("mouth", [0.0, -0.5, 1.0]),
    ("eye_l", [0.32, 0.02, 1.0]),
    ("eye_r", [-0.32, 0.02, 1.0]),
    ("brow_l", [0.32, 0.18, 1.0]),
    ("brow_r", [-0.32, 0.18, 1.0]),
    ("cheek_l", [0.65, -0.2, 0.85]),
    ("cheek_r", [-0.65, -0.2, 0.85]),
    ("jaw_l", [0.75, -0.6, 0.3]),
    ("jaw_r", [-0.75, -0.6, 0.3]),
    ("ear_root_l", [1.0, -0.05, -0.05]),
    ("ear_root_r", [-1.0, -0.05, -0.05]),
    ("temple_l", [0.85, 0.3, 0.5]),
    ("temple_r", [-0.85, 0.3, 0.5]),
    ("hairline_front", [0.0, 0.6, 0.85]),
    ("nape", [0.0, -0.45, -1.0]),
    ("back_of_head", [0.0, 0.15, -1.0]),
    ("forehead", [0.0, 0.4, 1.0]),
];

/// Known categories with landmark tables.
pub const CATEGORIES: [&str; 1] = ["humanoid_head"];

/// The landmark table of a category.
pub fn category(name: &str) -> Option<&'static [(&'static str, [f32; 3])]> {
    match name {
        "humanoid_head" => Some(&HUMANOID_HEAD),
        _ => None,
    }
}

/// The direction of a generic or category landmark.
///
/// Complexity: O(table size).
pub fn direction(name: &str, category_name: Option<&str>) -> Option<[f32; 3]> {
    category_name.and_then(category).into_iter().flatten().chain(GENERIC.iter()).find(|(n, _)| *n == name).map(|(_, d)| *d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_and_right_landmarks_mirror() {
        let table = category("humanoid_head").unwrap_or(&[]);
        for (name, d) in table {
            if let Some(stem) = name.strip_suffix("_l") {
                let r = direction(&format!("{stem}_r"), Some("humanoid_head"));
                assert_eq!(r, Some([-d[0], d[1], d[2]]), "{name}");
            }
        }
        assert_eq!(direction("top", None), Some([0.0, 1.0, 0.0]));
        assert_eq!(direction("crown", None), None, "category landmarks need the category");
    }
}
