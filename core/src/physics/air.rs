//! Atmospheric absorption of sound, ISO 9613-1:1993 (pure-tone attenuation coefficient), and
//! the speed of sound.

/// Reference pressure (kPa) and temperatures (K) of ISO 9613-1.
const P_REF: f64 = 101.325;
const T_REF: f64 = 293.15;
const T_TRIPLE: f64 = 273.16;

/// Speed of sound in air, m/s (ISO 9613-1 c = 343.2 sqrt(T / T0)).
pub fn speed_of_sound(temperature_c: f64) -> f64 {
    343.2 * ((temperature_c + 273.15) / T_REF).sqrt()
}

/// Attenuation coefficient alpha in dB per metre for frequency `f` (Hz), air temperature (deg C),
/// relative humidity (%), ambient pressure (kPa). ISO 9613-1 equations (3)-(5) and Annex B.
pub fn absorption_db_per_m(f: f64, temperature_c: f64, humidity_pct: f64, pressure_kpa: f64) -> f64 {
    let t = temperature_c + 273.15;
    let pa = pressure_kpa / P_REF;
    let tr = t / T_REF;
    // Molar concentration of water vapour h (%), from relative humidity via the saturation
    // vapour pressure (Annex B).
    let c = -6.8346 * (T_TRIPLE / t).powf(1.261) + 4.6151;
    let psat = 10f64.powf(c);
    let h = humidity_pct * psat / pa;
    // Relaxation frequencies of oxygen and nitrogen.
    let fr_o = pa * (24.0 + 4.04e4 * h * (0.02 + h) / (0.391 + h));
    let fr_n = pa * tr.powf(-0.5) * (9.0 + 280.0 * h * (-4.170 * (tr.powf(-1.0 / 3.0) - 1.0)).exp());
    let f2 = f * f;
    8.686
        * f2
        * (1.84e-11 / pa * tr.sqrt()
            + tr.powf(-2.5)
                * (0.01275 * (-2239.1 / t).exp() / (fr_o + f2 / fr_o)
                    + 0.1068 * (-3352.0 / t).exp() / (fr_n + f2 / fr_n)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ISO 9613-2:1996 Table 2: octave-band attenuation coefficients (dB/km), computed by the
    /// standard from ISO 9613-1 at the exact mid-band frequencies 63..8000 Hz.
    const TABLE: [(f64, f64, [f64; 8]); 6] = [
        (10.0, 70.0, [0.1, 0.4, 1.0, 1.9, 3.7, 9.7, 32.8, 117.0]),
        (20.0, 70.0, [0.1, 0.3, 1.1, 2.8, 5.0, 9.0, 22.9, 76.6]),
        (30.0, 70.0, [0.1, 0.3, 1.0, 3.1, 7.4, 12.7, 23.1, 59.3]),
        (15.0, 20.0, [0.3, 0.6, 1.2, 2.7, 8.2, 28.2, 88.8, 202.0]),
        (15.0, 50.0, [0.1, 0.5, 1.2, 2.2, 4.2, 10.8, 36.2, 129.0]),
        (15.0, 80.0, [0.1, 0.3, 1.1, 2.4, 4.1, 8.3, 23.7, 82.8]),
    ];

    #[test]
    fn matches_iso_9613_2_table_2() {
        for (t, rh, row) in TABLE {
            for (i, want) in row.iter().enumerate() {
                // Exact mid-band frequencies: 1000 * 10^(3k/10) (base-ten octaves).
                let f = 1000.0 * 10f64.powf(0.3 * (i as f64 - 4.0));
                let got = absorption_db_per_m(f, t, rh, P_REF) * 1000.0;
                // The table is rounded to 0.1 dB/km (3 significant digits above 100).
                let tol = (0.06f64).max(want * 0.006);
                assert!((got - want).abs() <= tol, "T={t} RH={rh} f={f:.0}: {got:.3} vs {want}");
            }
        }
    }

    #[test]
    fn speed_of_sound_at_20c() {
        assert!((speed_of_sound(20.0) - 343.2).abs() < 1e-9);
        assert!((speed_of_sound(0.0) - 331.3).abs() < 0.1);
    }
}
