/// 64-bit FNV-1a, hex encoded. Deterministic across platforms and Rust versions.
/// Used only for change detection, never for security.
pub fn content_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_and_sensitive() {
        assert_eq!(content_hash("a"), content_hash("a"));
        assert_ne!(content_hash("a"), content_hash("b"));
        assert_eq!(content_hash(""), "cbf29ce484222325");
    }
}
