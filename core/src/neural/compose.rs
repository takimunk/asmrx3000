//! Base model + species adapter -> ordinary per-species weights.
//!
//! The shared base (`birds_base.asnw`) is conditioned on a species embedding that enters a few
//! layers by concatenation; an adapter (`<species>.asna`, same container) holds that embedding,
//! the input normalisation, optional LoRA deltas and a few constants. Because the conditioning is
//! linear, a fixed species folds into biases:
//!
//! ```text
//! W [x; e] + b  =  W_x x + (b + W_e e)          (W + B A for LoRA layers)
//! ```
//!
//! so composing produces exactly the tensors a per-species model has (`voice.inp.weight` [E, 3],
//! ...), and the audio path costs the same as before. Mirrors `ml/asmr_ml/export_multi.py`.

use super::weights::{Error, Tensor, Weights};

fn tensor(name: &str, dims: Vec<usize>, data: Vec<f32>) -> Tensor {
    Tensor { name: name.to_string(), dims, data }
}

/// `W` [n_out, n_in] whose input columns `cond_at .. cond_at + cond.len()` see the constant
/// `cond`: returns W without those columns and the bias with `W_cond cond` folded in. Columns
/// after the conditioning block stay (inputs such as the contour noise z).
fn fold(w: &[f32], b: &[f32], n_out: usize, n_in: usize, cond_at: usize, cond: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let c = cond.len();
    let keep = n_in - c;
    let mut w2 = Vec::with_capacity(n_out * keep);
    let mut b2 = b.to_vec();
    for o in 0..n_out {
        let row = &w[o * n_in..(o + 1) * n_in];
        w2.extend_from_slice(&row[..cond_at]);
        w2.extend_from_slice(&row[cond_at + c..]);
        b2[o] += row[cond_at..cond_at + c].iter().zip(cond).map(|(a, e)| a * e).sum::<f32>();
    }
    (w2, b2)
}

/// W += B A, with B [n_out, r] and A [r, n_in].
fn add_lora(w: &mut [f32], n_out: usize, n_in: usize, b: &[f32], a: &[f32]) {
    let r = b.len() / n_out;
    for o in 0..n_out {
        for i in 0..n_in {
            let mut d = 0.0;
            for k in 0..r {
                d += b[o * r + k] * a[k * n_in + i];
            }
            w[o * n_in + i] += d;
        }
    }
}

fn dims2(w: &Weights, name: &str) -> Result<(usize, usize), Error> {
    let d = w.dims(name)?;
    if d.len() != 2 {
        return Err(Error(format!("{name}: not 2-D")));
    }
    Ok((d[0], d[1]))
}

pub fn compose(base: &Weights, adapter: &Weights) -> Result<Weights, Error> {
    let id_b = base.scalar("cfg.base_id")?;
    let id_a = adapter.scalar("cfg.base_id")?;
    if id_b != id_a {
        return Err(Error(format!("adapter was trained for base {id_a}, this base is {id_b}")));
    }
    let mut out: Vec<Tensor> = Vec::new();
    let copy = |out: &mut Vec<Tensor>, w: &Weights, name: &str| -> Result<(), Error> {
        let t = w.tensor(name)?;
        out.push(tensor(name, t.dims.clone(), t.data.clone()));
        Ok(())
    };

    // ---- voice ----
    let e = adapter.get("adapter.voice.emb", &[usize::MAX])?;
    let (n_out, n_in) = dims2(base, "voice.inp.weight")?;
    let (w, b) = fold(base.get("voice.inp.weight", &[n_out, n_in])?, base.get("voice.inp.bias", &[n_out])?, n_out, n_in, 3, e);
    out.push(tensor("voice.inp.weight", vec![n_out, n_in - e.len()], w));
    out.push(tensor("voice.inp.bias", vec![n_out], b));
    for name in ["weight_ih_l0", "weight_hh_l0", "bias_ih_l0", "bias_hh_l0"] {
        copy(&mut out, base, &format!("voice.gru.{name}"))?;
    }
    let (m, n1) = dims2(base, "voice.out1.weight")?;
    let mut w1 = base.get("voice.out1.weight", &[m, n1])?.to_vec();
    let (k2, m2) = dims2(base, "voice.out2.weight")?;
    let mut w2 = base.get("voice.out2.weight", &[k2, m2])?.to_vec();
    if adapter.has("adapter.voice.lora1.A") {
        add_lora(&mut w1, m, n1, adapter.get("adapter.voice.lora1.B", &[m, usize::MAX])?, adapter.get("adapter.voice.lora1.A", &[usize::MAX, n1])?);
        add_lora(&mut w2, k2, m2, adapter.get("adapter.voice.lora2.B", &[k2, usize::MAX])?, adapter.get("adapter.voice.lora2.A", &[usize::MAX, m2])?);
    }
    let cond_at = n1 - e.len(); // [g, x, e]
    let (w1, b1) = fold(&w1, base.get("voice.out1.bias", &[m])?, m, n1, cond_at, e);
    out.push(tensor("voice.out1.weight", vec![m, cond_at], w1));
    out.push(tensor("voice.out1.bias", vec![m], b1));
    out.push(tensor("voice.out2.weight", vec![k2, m2], w2));
    copy(&mut out, base, "voice.out2.bias")?;

    // ---- song syntax ----
    copy(&mut out, base, "song.emb.weight")?;
    let s = adapter.get("adapter.song.emb", &[usize::MAX])?;
    let (g3, gin) = dims2(base, "song.gru.weight_ih_l0")?;
    let (wih, bih) = fold(base.get("song.gru.weight_ih_l0", &[g3, gin])?, base.get("song.gru.bias_ih_l0", &[g3])?, g3, gin, gin - s.len(), s);
    out.push(tensor("song.gru.weight_ih_l0", vec![g3, gin - s.len()], wih));
    out.push(tensor("song.gru.bias_ih_l0", vec![g3], bih));
    copy(&mut out, base, "song.gru.weight_hh_l0")?;
    copy(&mut out, base, "song.gru.bias_hh_l0")?;
    copy(&mut out, base, "song.out.weight")?;
    let mut sb = base.tensor("song.out.bias")?.data.clone();
    for (o, d) in sb.iter_mut().zip(adapter.get("adapter.song.bias", &[usize::MAX])?) {
        *o += d;
    }
    let n = sb.len();
    out.push(tensor("song.out.bias", vec![n], sb));

    // ---- contour decoder: dec1 input is [token embedding, species embedding, z] ----
    copy(&mut out, base, "contour.tok.weight")?;
    let ce = adapter.get("adapter.contour.emb", &[usize::MAX])?;
    let tok_dim = base.dims("contour.tok.weight")?[1];
    let (h, din) = dims2(base, "contour.dec1.weight")?;
    let (wd, bd) = fold(base.get("contour.dec1.weight", &[h, din])?, base.get("contour.dec1.bias", &[h])?, h, din, tok_dim, ce);
    out.push(tensor("contour.dec1.weight", vec![h, din - ce.len()], wd));
    out.push(tensor("contour.dec1.bias", vec![h], bd));
    for name in ["contour.dec2.weight", "contour.dec2.bias", "contour.dec3.weight", "contour.dec3.bias"] {
        copy(&mut out, base, name)?;
    }

    // ---- configuration: shared from the base, species constants from the adapter ----
    for t in [base, adapter] {
        for name in t.names().filter(|n| n.starts_with("cfg.") && *n != "cfg.base_id") {
            if !out.iter().any(|o| o.name == name) {
                copy(&mut out, t, name)?;
            }
        }
    }
    Ok(Weights::from_tensors(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_matches_concatenated_input() {
        // y = W [x0, x1, e0, e1, z0] + b
        let w = [1.0, 2.0, 3.0, 4.0, 5.0, -1.0, 0.5, 2.0, -2.0, 1.0];
        let b = [0.1, 0.2];
        let (x, e, z) = ([0.3, -0.7], [1.5, -0.5], [2.0]);
        let full: Vec<f32> = (0..2)
            .map(|o| {
                let inp = [x[0], x[1], e[0], e[1], z[0]];
                b[o] + (0..5).map(|i| w[o * 5 + i] * inp[i]).sum::<f32>()
            })
            .collect();
        let (w2, b2) = fold(&w, &b, 2, 5, 2, &e);
        for o in 0..2 {
            let inp = [x[0], x[1], z[0]];
            let y = b2[o] + (0..3).map(|i| w2[o * 3 + i] * inp[i]).sum::<f32>();
            assert!((y - full[o]).abs() < 1e-5);
        }
    }
}
