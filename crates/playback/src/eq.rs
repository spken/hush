use std::{
    f64::consts::{SQRT_2, TAU},
    sync::Arc,
};

use arc_swap::{ArcSwap, Guard};
use librespot_playback::{SAMPLE_RATE, player::db_to_ratio};

/// Centre frequencies of the equalizer bands: a low shelf, eight peaking filters one
/// octave apart, and a high shelf.
pub const EQ_BANDS_HZ: [u16; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
const BANDS: usize = EQ_BANDS_HZ.len();
/// Coefficients per biquad: b0, b1, b2, a1, a2, normalised by a0.
const TAPS: usize = 5;
const PEAK_Q: f64 = SQRT_2;
// Moving coefficients over 20 ms instead of jumping avoids a click.
const RAMP_FRAMES: u32 = SAMPLE_RATE / 50;
// Keeps the filter state out of the denormal range during silence; far below audibility.
const ANTI_DENORMAL: f64 = 1e-20;

/// The preamp gain followed by every band's biquad coefficients.
pub(crate) type Coefficients = [f64; 1 + TAPS * BANDS];

/// Computes the coefficients for a preamp and one gain per band, all in dB.
pub(crate) fn coefficients(preamp_db: f64, gains_db: &[f64; BANDS]) -> Coefficients {
    let mut coefficients = [0.0; 1 + TAPS * BANDS];
    coefficients[0] = db_to_ratio(preamp_db);
    let (bands, _) = coefficients[1..].as_chunks_mut::<TAPS>();
    for (index, (taps, &gain_db)) in bands.iter_mut().zip(gains_db).enumerate() {
        *taps = band(index, gain_db);
    }
    coefficients
}

/// RBJ cookbook biquad for band `index`: shelves at the ends, peaking filters between.
fn band(index: usize, gain_db: f64) -> [f64; TAPS] {
    let a = 10f64.powf(gain_db / 40.0);
    let w0 = TAU * f64::from(EQ_BANDS_HZ[index]) / f64::from(SAMPLE_RATE);
    let (sin, cos) = w0.sin_cos();
    let [b0, b1, b2, a0, a1, a2] = if index == 0 || index == BANDS - 1 {
        // Shelf slope 1; `side` turns the low-shelf formulas into the high-shelf ones.
        let k = 2.0 * a.sqrt() * sin / SQRT_2;
        let side = if index == 0 { 1.0 } else { -1.0 };
        let (p, m) = (a + 1.0, a - 1.0);
        [
            a * (p - side * m * cos + k),
            side * 2.0 * a * (m - side * p * cos),
            a * (p - side * m * cos - k),
            p + side * m * cos + k,
            -side * 2.0 * (m + side * p * cos),
            p + side * m * cos - k,
        ]
    } else {
        let alpha = sin / (2.0 * PEAK_Q);
        [
            1.0 + alpha * a,
            -2.0 * cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos,
            1.0 - alpha / a,
        ]
    };
    [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
}

/// The sink's side of the equalizer: follows the shared coefficients, ramping toward
/// each new set, and keeps filter state across tracks so gapless stays seamless.
pub(crate) struct Filters {
    shared: Arc<ArcSwap<Coefficients>>,
    target: Arc<Coefficients>,
    current: Coefficients,
    step: Coefficients,
    ramp_left: u32,
    state: [[[f64; 2]; BANDS]; 2],
}

impl Filters {
    pub(crate) fn new(shared: Arc<ArcSwap<Coefficients>>) -> Self {
        let target = shared.load_full();
        Self {
            current: *target,
            shared,
            target,
            step: [0.0; 1 + TAPS * BANDS],
            ramp_left: 0,
            state: [[[0.0; 2]; BANDS]; 2],
        }
    }

    /// Starts a ramp if the coefficients changed since the last call.
    pub(crate) fn update(&mut self) {
        let latest = self.shared.load();
        if Arc::ptr_eq(&latest, &self.target) {
            return;
        }
        self.target = Guard::into_inner(latest);
        for ((step, target), current) in self.step.iter_mut().zip(&*self.target).zip(&self.current)
        {
            *step = (target - current) / f64::from(RAMP_FRAMES);
        }
        self.ramp_left = RAMP_FRAMES;
    }

    /// Runs the preamp and every band over one stereo frame, in place.
    pub(crate) fn process(&mut self, frame: &mut [f64]) {
        if self.ramp_left > 0 {
            self.ramp_left -= 1;
            if self.ramp_left == 0 {
                self.current = *self.target;
            } else {
                for (current, step) in self.current.iter_mut().zip(&self.step) {
                    *current += step;
                }
            }
        }
        let preamp = self.current[0];
        let (bands, _) = self.current[1..].as_chunks::<TAPS>();
        for (sample, state) in frame.iter_mut().zip(&mut self.state) {
            let mut x = *sample * preamp + ANTI_DENORMAL;
            for (&[b0, b1, b2, a1, a2], z) in bands.iter().zip(state) {
                // Transposed direct form II.
                let y = b0 * x + z[0];
                z[0] = b1 * x - a1 * y + z[1];
                z[1] = b2 * x - a2 * y;
                x = y;
            }
            *sample = x;
        }
    }
}
