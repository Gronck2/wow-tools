//! Version numbers compared by their digits.

/// "v1.2.3-beta" -> [1, 2, 3].
fn numbers(v: &str) -> Vec<u64> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let v = v.split(['-', '+']).next().unwrap_or_default();
    v.split('.').map(|n| n.parse().unwrap_or(0)).collect()
}

/// `candidate` is a later version than `than`.
pub fn newer(candidate: &str, than: &str) -> bool {
    let (mut a, mut b) = (numbers(candidate), numbers(than));
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a > b
}
