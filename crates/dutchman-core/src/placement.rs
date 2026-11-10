//! Rigid placements, row-major 3 × 4 (a rotation and a translation in millimetres), and the
//! little arithmetic the assembly code needs on them.

/// A rigid placement: rows of a rotation followed by a translation, in millimetres.
pub type Placement3x4 = [[f64; 4]; 3];

/// The identity placement.
pub const IDENTITY: Placement3x4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

/// Reflection in the YZ plane. Every improper orthogonal placement is `R * MIRROR_X`.
pub const MIRROR_X: Placement3x4 = [
    [-1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

/// Rotation from Fusion 360's default Y-up coordinate system to SolidCraft's default Z-up
/// coordinate system (+90° about +X: X -> X, Y -> Z, Z -> -Y).
pub const STAND_UP_Y_TO_Z: Placement3x4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 0.0, -1.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
];

/// Admit orthogonal placements within 1e-6, remove numeric drift, and factor out an X
/// reflection when needed. The returned placement is always proper rigid; translation is
/// unchanged. Scaling and shear outside that tolerance are refused, never repaired.
pub fn rigid(placement: &Placement3x4) -> Result<(Placement3x4, bool), &'static str> {
    const TOLERANCE: f64 = 1e-6;
    if placement.iter().flatten().any(|v| !v.is_finite()) {
        return Err("non-finite placement");
    }
    let [mut x, mut y, z] = [0, 1, 2].map(|c| column(placement, c));
    let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    if [x, y, z]
        .into_iter()
        .any(|a| (dot(a, a) - 1.0).abs() > TOLERANCE)
    {
        return Err("scaled placement");
    }
    if [dot(x, y), dot(x, z), dot(y, z)]
        .into_iter()
        .any(|v| v.abs() > TOLERANCE)
    {
        return Err("sheared placement");
    }
    let cross =
        |[a, b, c]: [f64; 3], [d, e, f]: [f64; 3]| [b * f - c * e, c * d - a * f, a * e - b * d];
    let determinant = dot(cross(x, y), z);
    if (determinant.abs() - 1.0).abs() > TOLERANCE {
        return Err("non-orthogonal placement");
    }
    let reflected = determinant < 0.0;
    let length = dot(x, x).sqrt();
    x = x.map(|v| v / length * if reflected { -1.0 } else { 1.0 });
    let projection = dot(x, y);
    for (v, axis) in y.iter_mut().zip(x) {
        *v -= projection * axis;
    }
    let length = dot(y, y).sqrt();
    y = y.map(|v| v / length);
    let z = cross(x, y);
    let mut out = *placement;
    for (row, [x, y, z]) in out.iter_mut().zip((0..3).map(|i| {
        [
            x.get(i).copied().unwrap_or(0.0),
            y.get(i).copied().unwrap_or(0.0),
            z.get(i).copied().unwrap_or(0.0),
        ]
    })) {
        row[0] = x;
        row[1] = y;
        row[2] = z;
    }
    Ok((out, reflected))
}

/// `outer` after `inner`: a point placed by `inner`, then by `outer`.
pub fn compose(outer: &Placement3x4, inner: &Placement3x4) -> Placement3x4 {
    let mut out = [[0.0; 4]; 3];
    for (row, out_row) in outer.iter().zip(out.iter_mut()) {
        for (col, value) in out_row.iter_mut().enumerate() {
            let mut sum: f64 = row
                .iter()
                .take(3)
                .zip(inner.iter())
                .map(|(r, inner_row)| r * inner_row.get(col).copied().unwrap_or(0.0))
                .sum();
            if col == 3 {
                sum += row.get(3).copied().unwrap_or(0.0);
            }
            *value = sum;
        }
    }
    out
}

/// The inverse of a rigid placement.
pub fn inverse(placement: &Placement3x4) -> Placement3x4 {
    let mut out = [[0.0; 4]; 3];
    for (i, out_row) in out.iter_mut().enumerate() {
        // The rotation transposed, and the translation taken back through it.
        let mut moved = 0.0;
        for (j, row) in placement.iter().enumerate() {
            let r = row.get(i).copied().unwrap_or(0.0);
            if let Some(cell) = out_row.get_mut(j) {
                *cell = r;
            }
            moved -= r * row.get(3).copied().unwrap_or(0.0);
        }
        out_row[3] = moved;
    }
    out
}

/// Column `c` of a placement: an axis for 0 to 2, the origin for 3.
pub fn column(placement: &Placement3x4, c: usize) -> [f64; 3] {
    [0, 1, 2].map(|r| {
        placement
            .get(r)
            .and_then(|row| row.get(c))
            .copied()
            .unwrap_or(0.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rigid_placements_factor_reflections_and_repair_only_small_drift() {
        let turn = [
            [0.0, -1.0, 0.0, 7.0],
            [1.0, 0.0, 0.0, -2.0],
            [0.0, 0.0, 1.0, 3.0],
        ];
        for source in [IDENTITY, turn, MIRROR_X, compose(&turn, &MIRROR_X)] {
            let (rotation, reflected) = rigid(&source).unwrap();
            assert!(!rigid(&rotation).unwrap().1);
            let restored = compose(&rotation, if reflected { &MIRROR_X } else { &IDENTITY });
            assert_eq!(restored, source);
        }
        let mut drift = turn;
        drift[0][0] += 1e-8;
        drift[1][0] += 1e-8;
        let (rotation, reflected) = rigid(&drift).unwrap();
        assert!(!reflected);
        for (row, expected) in compose(&inverse(&rotation), &rotation).iter().zip(IDENTITY) {
            for (value, expected) in row.iter().zip(expected) {
                assert!((value - expected).abs() < 1e-12);
            }
        }
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut bad = IDENTITY;
            bad[2][3] = value;
            assert_eq!(rigid(&bad), Err("non-finite placement"));
        }
        let mut scale = IDENTITY;
        scale[0][0] = 1.01;
        assert_eq!(rigid(&scale), Err("scaled placement"));
        let mut shear = IDENTITY;
        shear[0][1] = 1e-4;
        assert_eq!(rigid(&shear), Err("sheared placement"));
    }

    #[test]
    fn a_placement_and_its_inverse_compose_to_the_identity() {
        let turned = [
            [0.0, -1.0, 0.0, 30.0],
            [1.0, 0.0, 0.0, 5.0],
            [0.0, 0.0, 1.0, -2.0],
        ];
        let both = compose(&inverse(&turned), &turned);
        for (row, want) in both.iter().zip(IDENTITY) {
            for (got, want) in row.iter().zip(want) {
                assert!((got - want).abs() < 1e-12, "{both:?}");
            }
        }
        assert_eq!(column(&turned, 3), [30.0, 5.0, -2.0]);
    }
}
