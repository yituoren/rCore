//! Banker's algorithm helpers used by mutex/semaphore deadlock detection.

use alloc::vec;
use alloc::vec::Vec;

/// Grow `v` so that `v.len() >= n`, padding with zeros.
pub fn ensure_vec(v: &mut Vec<i32>, n: usize) {
    if v.len() < n {
        v.resize(n, 0);
    }
}

/// Grow `m` so that row `tid` exists and has at least `n_res` columns.
pub fn ensure_matrix(m: &mut Vec<Vec<i32>>, tid: usize, n_res: usize) {
    while m.len() <= tid {
        m.push(Vec::new());
    }
    ensure_vec(&mut m[tid], n_res);
    // also extend earlier rows so that they all have the same width
    for row in m.iter_mut() {
        ensure_vec(row, n_res);
    }
}

/// Banker's safety check: returns true if there exists an ordering of threads
/// such that each thread's pending `need` can be satisfied by the running
/// `work` vector (initially `available` plus all already-finished threads'
/// `allocation`).
pub fn is_safe(available: &[i32], allocation: &[Vec<i32>], need: &[Vec<i32>]) -> bool {
    let n_thread = allocation.len();
    let n_res = available.len();
    let mut work: Vec<i32> = available.to_vec();
    let mut finish = vec![false; n_thread];
    loop {
        let mut progressed = false;
        for i in 0..n_thread {
            if finish[i] {
                continue;
            }
            let row_need = &need[i];
            let can = (0..n_res).all(|j| row_need.get(j).copied().unwrap_or(0) <= work[j]);
            if can {
                finish[i] = true;
                let row_alloc = &allocation[i];
                for j in 0..n_res {
                    work[j] += row_alloc.get(j).copied().unwrap_or(0);
                }
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    finish.iter().all(|&f| f)
}
