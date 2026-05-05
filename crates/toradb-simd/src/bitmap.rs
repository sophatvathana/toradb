pub fn popcnt_u64(word: u64) -> u32 {
    word.count_ones()
}
