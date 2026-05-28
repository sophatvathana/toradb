pub fn popcnt_u64(word: u64) -> u32 {
    (crate::dispatch::table().popcnt_u64)(word)
}

pub fn popcnt_slice_u64(words: &[u64]) -> u64 {
    (crate::dispatch::table().popcnt_slice_u64)(words)
}

#[cfg(test)]
mod tests {
    use super::{popcnt_slice_u64, popcnt_u64};

    #[test]
    fn popcnt_word_works() {
        assert_eq!(popcnt_u64(0), 0);
        assert_eq!(popcnt_u64(u64::MAX), 64);
        assert_eq!(popcnt_u64(0b101010), 3);
    }

    #[test]
    fn popcnt_slice_matches_reference() {
        let words = [0u64, u64::MAX, 0xF0F0_F0F0_F0F0_F0F0, 0x0123_4567_89AB_CDEF];
        let expected: u64 = words.iter().map(|w| w.count_ones() as u64).sum();
        assert_eq!(popcnt_slice_u64(&words), expected);
    }
}
