//! Exact image ROI ratios. Camera constraints are read at runtime, because
//! increments and usable sensor dimensions differ between Basler models.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ratio {
    pub width: i64,
    pub height: i64,
}

pub const PRESETS: [Ratio; 10] = [
    Ratio {
        width: 1,
        height: 1,
    },
    Ratio {
        width: 4,
        height: 3,
    },
    Ratio {
        width: 3,
        height: 4,
    },
    Ratio {
        width: 16,
        height: 9,
    },
    Ratio {
        width: 9,
        height: 16,
    },
    Ratio {
        width: 3,
        height: 2,
    },
    Ratio {
        width: 2,
        height: 3,
    },
    Ratio {
        width: 5,
        height: 4,
    },
    Ratio {
        width: 4,
        height: 5,
    },
    Ratio {
        width: 7,
        height: 3,
    },
];

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub min: i64,
    pub max: i64,
    pub step: i64,
}

impl Limits {
    pub fn accepts(self, value: i64) -> bool {
        value >= self.min
            && value <= self.max
            && (self.step <= 1 || (value - self.min) % self.step == 0)
    }

    pub fn nearest(self, desired: i64) -> Option<i64> {
        if self.max < self.min {
            return None;
        }
        let step = self.step.max(1);
        let high = self.min + (self.max - self.min) / step * step;
        let below = self.min + (desired.clamp(self.min, high) - self.min) / step * step;
        let above = (below + step).min(high);
        Some(if (desired - below).abs() <= (above - desired).abs() {
            below
        } else {
            above
        })
    }
}

pub fn largest(ratio: Ratio, width: Limits, height: Limits) -> Option<(i64, i64)> {
    let maximum = (width.max / ratio.width).min(height.max / ratio.height);
    for multiplier in (1..=maximum).rev() {
        let pair = (ratio.width * multiplier, ratio.height * multiplier);
        if width.accepts(pair.0) && height.accepts(pair.1) {
            return Some(pair);
        }
    }
    None
}

pub fn paired(
    ratio: Ratio,
    changed_width: bool,
    value: i64,
    width: Limits,
    height: Limits,
) -> Option<(i64, i64)> {
    let (base, other) = if changed_width {
        (ratio.width, ratio.height)
    } else {
        (ratio.height, ratio.width)
    };
    if value % base != 0 {
        return None;
    }
    let counterpart = value.checked_div(base)?.checked_mul(other)?;
    let pair = if changed_width {
        (value, counterpart)
    } else {
        (counterpart, value)
    };
    (width.accepts(pair.0) && height.accepts(pair.1)).then_some(pair)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn largest_exact_pair_respects_both_increments() {
        let width = Limits {
            min: 64,
            max: 4096,
            step: 16,
        };
        let height = Limits {
            min: 64,
            max: 3000,
            step: 8,
        };
        assert_eq!(largest(PRESETS[1], width, height), Some((4000, 3000)));
    }

    #[test]
    fn paired_rejects_unrepresentable_dimension() {
        let width = Limits {
            min: 64,
            max: 4096,
            step: 16,
        };
        let height = Limits {
            min: 64,
            max: 3000,
            step: 8,
        };
        assert_eq!(
            paired(PRESETS[1], true, 1600, width, height),
            Some((1600, 1200))
        );
        assert_eq!(paired(PRESETS[1], true, 1601, width, height), None);
        assert_eq!(paired(PRESETS[1], false, 3016, width, height), None);
    }

    #[test]
    fn wide_preset_uses_reduced_exact_ratio() {
        let limits = Limits {
            min: 1,
            max: 1400,
            step: 1,
        };
        assert_eq!(largest(PRESETS[9], limits, limits), Some((1400, 600)));
    }
}
