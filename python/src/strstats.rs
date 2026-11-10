//! `StringStats`: per-string statistics over caller-supplied tables, for
//! scoring how random a string looks (the features of an API-key
//! classifier): the narrowest charset holding the string, normalized Shannon
//! entropy, the mean squared error of its character-distance histogram
//! against the charset's, and the mean bigram log-probability of its
//! normalized characters.
//!
//! Every float comes from the same operations, in the same order, as the
//! plain Python computation: C library `log`/`exp`/`pow` (what `math.log`,
//! `math.exp` and `float ** int` call) and left-to-right additions. So the
//! results are bit-identical to it on the same platform.

use pyo3::prelude::*;
use pyo3::types::{PyList, PyString, PyTuple};
use rustc_hash::FxHashMap;
use std::hint::black_box;

unsafe extern "C" {
    fn log(x: f64) -> f64;
    fn exp(x: f64) -> f64;
    fn pow(x: f64, y: f64) -> f64;
}

// black_box keeps LLVM from folding a constant argument at compile time or
// rewriting pow(x, 2.0) as x * x; either could round differently from the
// C library call Python makes at run time.
fn c_log(x: f64) -> f64 {
    // SAFETY: log is a pure C function of one double, defined for every input.
    unsafe { log(black_box(x)) }
}

fn c_exp(x: f64) -> f64 {
    // SAFETY: as c_log.
    unsafe { exp(black_box(x)) }
}

fn c_pow(x: f64, y: f64) -> f64 {
    // SAFETY: as c_log.
    unsafe { pow(black_box(x), black_box(y)) }
}

struct Charset {
    len: usize,
    /// char -> its position in the charset string (the last one, if repeated)
    index: FxHashMap<u32, usize>,
    /// character distance -> its position in `values` / `absent`
    dist_index: FxHashMap<u32, usize>,
    /// the charset's own distance histogram, in the caller's key order
    values: Vec<f64>,
    /// the MSE term of a distance the string doesn't have, per key
    absent: Vec<f64>,
}

/// A feature value: `int 0` where the Python computation returns the int
/// literal, otherwise a float.
enum Num {
    Zero,
    F(f64),
}

struct Features {
    entropy: f64,
    sequentiality: Num,
    gibberish: Num,
    charset_len: f64,
}

/// (charset string, distance keys, their histogram values, their
/// absent-distance MSE terms)
type CharsetTables = (String, Vec<u32>, Vec<f64>, Vec<f64>);

#[pyclass(frozen, module = "androguard_rs")]
pub struct StringStats {
    charsets: Vec<Charset>,
    /// char -> alphabet position of what it normalizes to, for kept chars
    normalize: FxHashMap<u32, usize>,
    alphabet: usize,
    /// bigram log-probabilities, row-major over alphabet positions
    log_prob: Vec<f64>,
}

impl StringStats {
    fn charset_of(&self, cps: &[u32]) -> Option<&Charset> {
        self.charsets
            .iter()
            .find(|c| cps.iter().all(|cp| c.index.contains_key(cp)))
    }

    fn entropy(cs: &Charset, cps: &[u32]) -> f64 {
        let n = cps.len() as f64;
        let mut counts = vec![0u64; cs.len];
        for cp in cps {
            counts[cs.index[cp]] += 1;
        }
        // present chars in charset order, as the Python walk does
        let log2 = c_log(2.0);
        let mut entropy = 0.0;
        for &c in counts.iter().filter(|&&c| c > 0) {
            let p = c as f64 / n;
            entropy += p * (c_log(p) / log2);
        }
        -entropy / n
    }

    fn sequentiality(cs: &Charset, cps: &[u32]) -> Num {
        let n = cps.len();
        if n <= 2 {
            return Num::Zero;
        }
        let window = c_log(n as f64).floor() as usize;
        let mut counts: FxHashMap<u32, u64> = FxHashMap::default();
        let mut counter: u64 = 0;
        for k in 1..=window.min(n - 1) {
            for (a, b) in cps.iter().zip(&cps[k..]) {
                *counts.entry(a.abs_diff(*b)).or_insert(0) += 1;
            }
            counter += (n - k) as u64;
        }
        let n_keys = cs.values.len() as f64;
        let mut terms = cs.absent.clone();
        for (d, c) in counts {
            if let Some(&i) = cs.dist_index.get(&d) {
                // float ** 2 in Python raises the magnitude of a negative base
                let x = c as f64 / counter as f64 - cs.values[i];
                terms[i] = c_pow(x.abs(), 2.0) / n_keys;
            }
        }
        if terms.is_empty() {
            return Num::Zero;
        }
        Num::F(terms.iter().fold(0.0, |acc, t| acc + t))
    }

    fn gibberish(&self, cps: &[u32]) -> Num {
        let kept: Vec<usize> = cps.iter().filter_map(|cp| self.normalize.get(cp).copied()).collect();
        if kept.len() <= 1 {
            return Num::Zero;
        }
        let k = self.alphabet;
        let lp = kept.windows(2).fold(0.0, |acc, w| acc + self.log_prob[w[0] * k + w[1]]);
        Num::F(c_exp(lp / (kept.len() - 1) as f64))
    }

    fn features(&self, s: &str) -> Option<Features> {
        if s.is_empty() {
            return None;
        }
        let cps: Vec<u32> = s.chars().map(|c| c as u32).collect();
        let cs = self.charset_of(&cps)?;
        Some(Features {
            entropy: Self::entropy(cs, &cps),
            sequentiality: Self::sequentiality(cs, &cps),
            gibberish: self.gibberish(&cps),
            charset_len: cs.len as f64,
        })
    }
}

fn num<'py>(py: Python<'py>, v: &Num) -> PyResult<Bound<'py, PyAny>> {
    Ok(match v {
        Num::Zero => 0i64.into_pyobject(py)?.into_any(),
        Num::F(f) => f.into_pyobject(py)?.into_any(),
    })
}

#[pymethods]
impl StringStats {
    /// `charsets`: (charset string, distance keys, their histogram values,
    /// their absent-distance MSE terms) per charset, narrowest-first.
    /// `normalize`: code point -> alphabet position for every char the
    /// bigram model keeps. `log_prob`: alphabet x alphabet matrix.
    #[new]
    fn new(charsets: Vec<CharsetTables>, normalize: FxHashMap<u32, usize>, log_prob: Vec<Vec<f64>>) -> PyResult<Self> {
        let alphabet = log_prob.len();
        if log_prob.iter().any(|r| r.len() != alphabet) || normalize.values().any(|&p| p >= alphabet) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "log_prob must be square over the alphabet",
            ));
        }
        let mut out = Vec::with_capacity(charsets.len());
        for (chars, keys, values, absent) in charsets {
            if keys.len() != values.len() || keys.len() != absent.len() {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "distance tables differ in length",
                ));
            }
            let index: FxHashMap<u32, usize> = chars.chars().enumerate().map(|(i, c)| (c as u32, i)).collect();
            out.push(Charset {
                len: chars.chars().count(),
                index,
                dist_index: keys.iter().enumerate().map(|(i, &k)| (k, i)).collect(),
                values,
                absent,
            });
        }
        Ok(StringStats {
            charsets: out,
            normalize,
            alphabet,
            log_prob: log_prob.concat(),
        })
    }

    /// Features of each string: `(entropy, sequentiality, gibberish,
    /// float(len(charset)))`, `None` for an empty string or one no charset
    /// holds, and `False` for an item this can't read (not a str, or one
    /// holding lone surrogates), which the caller computes itself. Runs
    /// without the GIL.
    fn features_many<'py>(&self, py: Python<'py>, strings: &Bound<'py, PyList>) -> PyResult<Bound<'py, PyList>> {
        let owned: Vec<Option<String>> = strings
            .iter()
            .map(|s| {
                s.cast::<PyString>()
                    .ok()
                    .and_then(|s| s.to_str().ok())
                    .map(str::to_owned)
            })
            .collect();
        let results: Vec<Option<Option<Features>>> =
            py.detach(|| owned.iter().map(|s| s.as_deref().map(|s| self.features(s))).collect());
        let out = PyList::empty(py);
        for r in results {
            match r {
                None => out.append(false)?,
                Some(None) => out.append(py.None())?,
                Some(Some(f)) => out.append(PyTuple::new(
                    py,
                    [
                        f.entropy.into_pyobject(py)?.into_any(),
                        num(py, &f.sequentiality)?,
                        num(py, &f.gibberish)?,
                        f.charset_len.into_pyobject(py)?.into_any(),
                    ],
                )?)?,
            }
        }
        Ok(out)
    }
}
