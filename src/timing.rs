use rand::RngExt;
use rand_distr::{Distribution, SkewNormal};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DistributionKind {
    #[default]
    Shaped,
    Uniform,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TimingConfig {
    pub min_ms: f64,
    pub max_ms: f64,
    pub center_ms: f64,
    pub deviation_ms: f64,
    pub skew: f64,
    pub kind: DistributionKind,
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            min_ms: 50.,
            max_ms: 450.,
            center_ms: 250.,
            deviation_ms: 70.,
            skew: 0.,
            kind: DistributionKind::Shaped,
        }
    }
}

impl TimingConfig {
    pub fn validate(&self) -> Result<(), String> {
        if ![
            self.min_ms,
            self.max_ms,
            self.center_ms,
            self.deviation_ms,
            self.skew,
        ]
        .iter()
        .all(|v| v.is_finite())
        {
            return Err("Timing values must be finite numbers.".into());
        }
        if self.min_ms < 10. || self.max_ms > 60_000. || self.min_ms > self.max_ms {
            return Err(
                "Use a minimum and maximum between 10 and 60,000 ms, in that order.".into(),
            );
        }
        if self.kind == DistributionKind::Shaped {
            if self.center_ms < self.min_ms || self.center_ms > self.max_ms {
                return Err("Center must sit between your minimum and maximum.".into());
            }
            if self.deviation_ms < 0. || self.deviation_ms > self.max_ms - self.min_ms {
                return Err("Standard deviation must be between zero and the range width.".into());
            }
            if self.skew.abs() > 10. {
                return Err("Skew must be between −10 and +10.".into());
            }
        }
        Ok(())
    }

    pub fn constant(&self) -> Option<f64> {
        if self.min_ms == self.max_ms {
            Some(self.min_ms)
        } else if self.kind == DistributionKind::Shaped && self.deviation_ms == 0. {
            Some(self.center_ms)
        } else {
            None
        }
    }

    /// Rejection truncation avoids artificial probability piles at the bounds.
    /// Validated location, scale and shape keep acceptance bounded away from zero.
    pub fn sample<R: rand::Rng + ?Sized>(&self, rng: &mut R) -> f64 {
        if let Some(x) = self.constant() {
            return x;
        }
        if self.kind == DistributionKind::Uniform {
            return rng.random_range(self.min_ms..=self.max_ms);
        }
        let d = SkewNormal::new(self.center_ms, self.deviation_ms, self.skew)
            .expect("validated timing config");
        loop {
            let x = d.sample(rng);
            if (self.min_ms..=self.max_ms).contains(&x) {
                return x;
            }
        }
    }

    fn raw_density_z(&self, z: f64) -> f64 {
        let phi = (-z * z / 2.).exp() / (2. * std::f64::consts::PI).sqrt();
        phi * libm::erfc(-self.skew * z / std::f64::consts::SQRT_2)
    }

    pub fn model(&self) -> CurveModel {
        if let Some(x) = self.constant() {
            return CurveModel {
                points: vec![(x, 1.)],
                mean_ms: x,
                effective_deviation_ms: 0.,
                normalizer: 1.,
            };
        }
        if self.kind == DistributionKind::Uniform {
            return CurveModel {
                points: vec![
                    (self.min_ms, 1. / (self.max_ms - self.min_ms)),
                    (self.max_ms, 1. / (self.max_ms - self.min_ms)),
                ],
                mean_ms: (self.min_ms + self.max_ms) / 2.,
                effective_deviation_ms: (self.max_ms - self.min_ms) / (12_f64.sqrt()),
                normalizer: 1.,
            };
        }
        // Integrate in standard-normal coordinates, split into small intervals.
        // This remains accurate even when the millisecond range dwarfs sigma.
        let lo = ((self.min_ms - self.center_ms) / self.deviation_ms).max(-12.);
        let hi = ((self.max_ms - self.center_ms) / self.deviation_ms).min(12.);
        let n = (((hi - lo) / 0.015).ceil() as usize)
            .max(128)
            .next_multiple_of(2);
        let h = (hi - lo) / n as f64;
        let (mut mass, mut m1, mut m2) = (0., 0., 0.);
        let mut points = Vec::with_capacity(n + 3);
        points.push((
            self.min_ms,
            self.raw_density_z((self.min_ms - self.center_ms) / self.deviation_ms)
                / self.deviation_ms,
        ));
        for i in 0..=n {
            let z = lo + i as f64 * h;
            let f = self.raw_density_z(z);
            let weight = if i == 0 || i == n {
                1.
            } else if i % 2 == 0 {
                2.
            } else {
                4.
            };
            mass += weight * f;
            m1 += weight * f * z;
            m2 += weight * f * z * z;
            points.push((
                self.center_ms + z * self.deviation_ms,
                f / self.deviation_ms,
            ));
        }
        mass *= h / 3.;
        m1 *= h / 3.;
        m2 *= h / 3.;
        points.push((
            self.max_ms,
            self.raw_density_z((self.max_ms - self.center_ms) / self.deviation_ms)
                / self.deviation_ms,
        ));
        for (_, y) in &mut points {
            *y /= mass;
        }
        let mean_z = m1 / mass;
        CurveModel {
            points,
            mean_ms: self.center_ms + self.deviation_ms * mean_z,
            effective_deviation_ms: self.deviation_ms
                * (m2 / mass - mean_z * mean_z).max(0.).sqrt(),
            normalizer: mass,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CurveModel {
    pub points: Vec<(f64, f64)>,
    pub mean_ms: f64,
    pub effective_deviation_ms: f64,
    pub normalizer: f64,
}

impl CurveModel {
    pub fn wpm(&self) -> f64 {
        12_000. / self.mean_ms
    }
    pub fn seconds(&self, characters: usize) -> f64 {
        characters as f64 * self.mean_ms / 1000.
    }
}

pub fn required_wpm(characters: usize, minutes: f64) -> Option<f64> {
    (minutes.is_finite() && minutes > 0.).then(|| characters as f64 / 5. / minutes)
}

pub fn target_feasible(config: &TimingConfig, characters: usize, minutes: f64) -> bool {
    let ms = minutes * 60_000.;
    minutes.is_finite()
        && minutes > 0.
        && characters > 0
        && ms >= characters as f64 * config.min_ms
        && ms <= characters as f64 * config.max_ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    #[test]
    fn defaults_have_expected_speed_and_duration() {
        let m = TimingConfig::default().model();
        assert!((m.mean_ms - 250.).abs() < 1e-8);
        assert!((m.wpm() - 48.).abs() < 1e-8);
        assert!((m.seconds(4800) - 1200.).abs() < 1e-8);
        assert_eq!(required_wpm(4800, 20.), Some(48.));
        assert!(target_feasible(&TimingConfig::default(), 4800, 20.));
        assert!(!target_feasible(&TimingConfig::default(), 10, 20.));
    }
    #[test]
    fn seeded_samples_match_bounded_curve() {
        for skew in [-10., -3., 0., 3., 10.] {
            let c = TimingConfig {
                skew,
                center_ms: 110.,
                ..Default::default()
            };
            let mut rng = rand::rngs::StdRng::seed_from_u64(7);
            let mut total = 0.;
            for _ in 0..50_000 {
                let x = c.sample(&mut rng);
                assert!((c.min_ms..=c.max_ms).contains(&x));
                total += x;
            }
            assert!(
                (total / 50_000. - c.model().mean_ms).abs() < 1.5,
                "skew {skew}"
            );
        }
    }
    #[test]
    fn uniform_and_constant_modes() {
        let mut c = TimingConfig {
            kind: DistributionKind::Uniform,
            ..Default::default()
        };
        let mut rng = rand::rngs::StdRng::seed_from_u64(10);
        let mean: f64 = (0..20_000).map(|_| c.sample(&mut rng)).sum::<f64>() / 20_000.;
        assert!((mean - 250.).abs() < 3.);
        assert!((c.model().effective_deviation_ms - 400. / 12_f64.sqrt()).abs() < 1e-8);
        c.max_ms = c.min_ms;
        assert_eq!(c.sample(&mut rng), 50.);
        c = TimingConfig {
            deviation_ms: 0.,
            ..Default::default()
        };
        assert_eq!(c.sample(&mut rng), 250.);
    }
    #[test]
    fn narrow_sigma_extreme_skew_and_large_bounds_remain_finite() {
        for center in [10., 30_000., 60_000.] {
            for skew in [-10., 0., 10.] {
                let c = TimingConfig {
                    min_ms: 10.,
                    max_ms: 60_000.,
                    center_ms: center,
                    deviation_ms: 0.0001,
                    skew,
                    kind: DistributionKind::Shaped,
                };
                let m = c.model();
                assert!(m.mean_ms.is_finite() && m.normalizer > 0.);
                assert!((c.min_ms..=c.max_ms).contains(&m.mean_ms));
            }
        }
    }
    #[test]
    fn validation_rejects_bad_configs() {
        for c in [
            TimingConfig {
                min_ms: 0.,
                ..Default::default()
            },
            TimingConfig {
                deviation_ms: 500.,
                ..Default::default()
            },
            TimingConfig {
                skew: f64::NAN,
                ..Default::default()
            },
            TimingConfig {
                center_ms: 500.,
                ..Default::default()
            },
        ] {
            assert!(c.validate().is_err());
        }
    }
    #[test]
    fn deviation_flattens_density_and_skew_matches_closed_form_moments() {
        let narrow = TimingConfig {
            min_ms: 10.,
            max_ms: 2000.,
            center_ms: 1000.,
            deviation_ms: 50.,
            ..Default::default()
        };
        let wide = TimingConfig {
            deviation_ms: 100.,
            ..narrow.clone()
        };
        let peak = |c: &TimingConfig| c.model().points.iter().map(|p| p.1).fold(0., f64::max);
        assert!((peak(&narrow) / peak(&wide) - 2.).abs() < 0.001);
        assert!((wide.model().effective_deviation_ms - 100.).abs() < 0.001);
        for skew in [-10., -2., 0., 2., 10.] {
            let c = TimingConfig {
                skew,
                ..wide.clone()
            };
            let delta = skew / (1. + skew * skew).sqrt();
            let expected_mean =
                c.center_ms + c.deviation_ms * delta * (2. / std::f64::consts::PI).sqrt();
            let expected_sd =
                c.deviation_ms * (1. - 2. * delta * delta / std::f64::consts::PI).sqrt();
            let model = c.model();
            assert!((model.mean_ms - expected_mean).abs() < 0.001);
            assert!((model.effective_deviation_ms - expected_sd).abs() < 0.001);
        }
    }
}
