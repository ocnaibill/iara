//! Curva dos sliders de ganho e das barras dos medidores (spec 5): posição normalizada 0..=1 ↔ dB em −60..=0.
//!
//! dB = −60·(1 − p)². Linear em dB (a curva antiga, −60 + 60p) deixava metade do curso abaixo de −30 dB e soava baixa
//! demais perto do meio; a quadrática dá −15 dB no meio, −3,75 dB a três quartos e resolução fina perto de 0 dB (onde se
//! mixa), sem zona morta: o piso −60 dB continua no começo do curso. O mesmo eixo vale para o medidor desenhado atrás do
//! slider (um sinal de −6 dBFS chega à altura do slider posto em −6 dB). Silêncio exato (posição 0) é tratado por quem chama.

/// Piso da curva em dB (o começo do curso).
pub const FLOOR_DB: f64 = -60.0;

/// Posição 0..=1 → dB em −60..=0 (fora da faixa satura).
pub fn position_to_db(position: f64) -> f64 {
    let p = if position.is_nan() {
        0.0
    } else {
        position.clamp(0.0, 1.0)
    };
    FLOOR_DB * (1.0 - p) * (1.0 - p)
}

/// dB → posição 0..=1 (abaixo do piso vai a 0; acima de 0 dB satura em 1; NaN vai a 0).
pub fn db_to_position(db: f64) -> f64 {
    if db.is_nan() || db <= FLOOR_DB {
        return 0.0;
    }
    if db >= 0.0 {
        return 1.0;
    }
    1.0 - (db / FLOOR_DB).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curve_hits_the_landmarks_and_round_trips() {
        assert_eq!(position_to_db(1.0), 0.0);
        assert_eq!(position_to_db(0.0), -60.0);
        assert!((position_to_db(0.5) + 15.0).abs() < 1e-12);
        assert!((position_to_db(0.75) + 3.75).abs() < 1e-12);
        assert_eq!(position_to_db(7.0), 0.0, "acima do fim satura");
        assert_eq!(position_to_db(f64::NAN), -60.0);
        for i in 0..=100 {
            let p = f64::from(i) / 100.0;
            assert!((db_to_position(position_to_db(p)) - p).abs() < 1e-9, "{p}");
        }
        assert_eq!(db_to_position(f64::NEG_INFINITY), 0.0);
        assert_eq!(db_to_position(6.0), 1.0);
        assert_eq!(db_to_position(f64::NAN), 0.0);
    }

    #[test]
    fn the_curve_is_monotonic() {
        let mut last = f64::NEG_INFINITY;
        for i in 0..=1000 {
            let db = position_to_db(f64::from(i) / 1000.0);
            assert!(db >= last);
            last = db;
        }
    }
}
