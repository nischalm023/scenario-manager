//! Within `design`, `grid_geometry` is the AXIS-POSITION model: where each grid
//! row or column physically sits along its axis, in mm. It converts the
//! authoring format's optional per-axis center list (or a single
//! `cell_size_mm`) into the two questions the compiler actually asks — "where is
//! index `i`?" and "how far apart are `i` and `j`?" — so no compile step has to
//! know which of the two forms the design file used.
//!
//! It exists because one `cell_size_mm` cannot describe a real rack. On cencora
//! a rack unit is 2140mm holding 3 relays, 645mm center-to-center and 425mm from
//! relay center to unit edge, so relay pitch along a rack column alternates
//! 645, 645, 850 (the 850 being the two 425mm margins either side of an upright).
//! No uniform pitch lands on that sequence at any value.

/// Physical center positions along one grid axis, in mm, indexed by grid index.
///
/// Positions rather than extents are authoritative deliberately. Extents *can*
/// encode the same geometry (850, 440, 850 repeating yields cencora's centers),
/// but they correspond to no dimension on a drawing and have to be solved for,
/// and "extent" is ambiguous once spacing is non-uniform: the tiling extent
/// whose cumulative sum plus half gives the center (850, 440, 850) and the
/// midpoint-to-midpoint extent (747.5, 645, 747.5) disagree, and a cell is not
/// symmetric about its center under either. Centers avoid the choice.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct AxisGeometry {
    centers: Vec<f64>,
}

impl AxisGeometry {
    /// Uniform axis: center of index `i` is `i * pitch + pitch / 2`. Reproduces
    /// the pre-existing `col * cell_size + cell_size / 2` exactly, so a design
    /// with no center list compiles bit-identically to before.
    pub(super) fn uniform(count: u32, pitch: f64) -> Self {
        Self { centers: (0..count).map(|i| i as f64 * pitch + pitch / 2.0).collect() }
    }

    /// Explicit centers. `axis` names the axis for error messages only.
    ///
    /// Rejects anything that would make `pitch` meaningless downstream: wrong
    /// length, non-finite, negative, or not strictly increasing. Strictly
    /// increasing (not merely non-decreasing) because two rows sharing a center
    /// would produce a zero-length edge, which the pathfinder would treat as a
    /// free move between distinct cells.
    pub(super) fn from_centers(centers: &[f64], expected: u32, axis: &str) -> Result<Self, String> {
        if centers.len() != expected as usize {
            return Err(format!(
                "{axis}_centers_mm has {} entries but the grid {axis} count is {expected}",
                centers.len(),
            ));
        }
        for (i, &c) in centers.iter().enumerate() {
            if !c.is_finite() {
                return Err(format!("{axis}_centers_mm[{i}] is not finite ({c})"));
            }
            if c < 0.0 {
                return Err(format!("{axis}_centers_mm[{i}] is negative ({c})"));
            }
        }
        for (i, pair) in centers.windows(2).enumerate() {
            if pair[1] <= pair[0] {
                return Err(format!(
                    "{axis}_centers_mm must be strictly increasing, but [{i}] = {} and [{}] = {}",
                    pair[0],
                    i + 1,
                    pair[1],
                ));
            }
        }
        Ok(Self { centers: centers.to_vec() })
    }

    /// Center of `i` in mm. Out-of-range indices clamp to the ends rather than
    /// panicking: callers are compile-time loops already bounded by the grid, so
    /// a clamp here would only ever mask a bug in *this* module, and every
    /// caller is in a hot construction loop where a bounds check is noise.
    pub(super) fn center(&self, i: u32) -> f64 {
        match self.centers.get(i as usize) {
            Some(&c) => c,
            None => self.centers.last().copied().unwrap_or(0.0),
        }
    }

    /// Center-to-center distance between two indices, in mm. This is exactly
    /// what `Edge.distance_mm` is documented to hold.
    pub(super) fn pitch(&self, a: u32, b: u32) -> f64 {
        (self.center(b) - self.center(a)).abs()
    }

    /// Physical extent covered by indices `first..=last`, in mm.
    ///
    /// Center-to-center span plus one leading pitch, which reproduces
    /// `count * cell_size_mm` exactly on a uniform axis. It is an approximation
    /// on a non-uniform one: it cannot know the real margin between the outermost
    /// center and the structure's edge (425mm on a cencora rack unit, against a
    /// 322.5mm half-pitch). Callers that need the true extent should carry it
    /// explicitly rather than derive it here.
    pub(super) fn span(&self, first: u32, last: u32) -> f64 {
        if self.centers.is_empty() {
            return 0.0;
        }
        // Single-index range: there is no center-to-center span, so the extent is
        // the one pitch adjacent to `first`. Must be read at `first`, not at the
        // start of the axis — a one-row zone sitting on a cross-upright row is
        // 850mm wide even where rows 0..1 are 645mm apart. Prefer the pitch after
        // `first`, falling back to the one before it at the end of the axis.
        if last <= first {
            let n = self.centers.len() as u32;
            return if first + 1 < n {
                self.pitch(first, first + 1)
            } else if first > 0 {
                self.pitch(first - 1, first)
            } else {
                0.0
            };
        }
        self.pitch(first, last) + self.pitch(first, first + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_reproduces_the_legacy_formula() {
        let g = AxisGeometry::uniform(4, 850.0);
        // `col * cell_size + cell_size / 2`, the formula this replaces.
        for i in 0..4u32 {
            assert_eq!(g.center(i), i as f64 * 850.0 + 425.0);
        }
        assert_eq!(g.pitch(0, 1), 850.0);
        assert_eq!(g.pitch(0, 3), 2550.0);
        // span == count * pitch, so zone extents are unchanged for uniform maps.
        assert_eq!(g.span(0, 3), 4.0 * 850.0);
    }

    /// The cencora rack: 6 units of 2140mm, 3 relays each at 645mm centers,
    /// 425mm from relay center to unit edge. 425 + 645 + 645 + 425 = 2140.
    fn cencora_rack_centers(units: u32) -> Vec<f64> {
        let mut v = Vec::new();
        for u in 0..units {
            let base = u as f64 * 2140.0;
            v.extend([base + 425.0, base + 1070.0, base + 1715.0]);
        }
        v
    }

    #[test]
    fn cencora_pitch_alternates_645_645_850() {
        let centers = cencora_rack_centers(6);
        assert_eq!(centers.len(), 18, "6 units x 3 relays is the 18 the model already generates");
        let g = AxisGeometry::from_centers(&centers, 18, "row").expect("valid");

        // Relay 2 sits at the exact unit center.
        assert_eq!(g.center(1), 1070.0);
        assert_eq!(1070.0 * 2.0, 2140.0);

        let pitches: Vec<f64> = (0..17).map(|i| g.pitch(i, i + 1)).collect();
        let expected: Vec<f64> = (0..17).map(|i| if i % 3 == 2 { 850.0 } else { 645.0 }).collect();
        assert_eq!(pitches, expected, "within-unit gaps are 645, the cross-upright gap is 850");

        // Center-to-center span, and the average pitch that a uniform grid
        // cannot represent: 2140 / 3.
        assert_eq!(g.pitch(0, 17), 11990.0);
        let avg = g.pitch(0, 17) / 17.0;
        assert!((avg - 705.29).abs() < 0.01, "average pitch {avg}");
    }

    #[test]
    fn rejects_wrong_length() {
        let err = AxisGeometry::from_centers(&[0.0, 1.0], 3, "row").unwrap_err();
        assert!(err.contains("2 entries") && err.contains("is 3"), "{err}");
    }

    #[test]
    fn rejects_non_monotonic() {
        // Equal is rejected too: a zero-length edge would read as a free move.
        let err = AxisGeometry::from_centers(&[0.0, 500.0, 500.0], 3, "row").unwrap_err();
        assert!(err.contains("strictly increasing"), "{err}");
        let err = AxisGeometry::from_centers(&[0.0, 900.0, 500.0], 3, "row").unwrap_err();
        assert!(err.contains("strictly increasing"), "{err}");
    }

    #[test]
    fn rejects_non_finite_and_negative() {
        let err = AxisGeometry::from_centers(&[0.0, f64::NAN, 900.0], 3, "row").unwrap_err();
        assert!(err.contains("not finite"), "{err}");
        let err = AxisGeometry::from_centers(&[0.0, f64::INFINITY, 900.0], 3, "row").unwrap_err();
        assert!(err.contains("not finite"), "{err}");
        let err = AxisGeometry::from_centers(&[-1.0, 500.0, 900.0], 3, "row").unwrap_err();
        assert!(err.contains("negative"), "{err}");
    }

    #[test]
    fn single_index_span_reads_the_pitch_at_that_index() {
        // 645, 645, 850 repeating: a one-row zone must get the pitch beside ITS
        // OWN row, not whatever rows 0..1 happen to be.
        let g = AxisGeometry::from_centers(&cencora_rack_centers(2), 6, "row").expect("valid");
        assert_eq!(g.span(0, 0), 645.0, "row 0 -> row 1 is a within-unit gap");
        assert_eq!(g.span(2, 2), 850.0, "row 2 -> row 3 crosses the upright");
        assert_eq!(g.span(3, 3), 645.0);
        // Last index has no successor, so it falls back to the preceding pitch.
        assert_eq!(g.span(5, 5), 645.0);
    }

    #[test]
    fn single_index_axis_is_not_a_panic() {
        let g = AxisGeometry::uniform(1, 850.0);
        assert_eq!(g.center(0), 425.0);
        assert_eq!(g.pitch(0, 0), 0.0);
        // Degenerate span falls back to the leading pitch rather than dividing by zero.
        assert!(g.span(0, 0).is_finite());
    }
}
