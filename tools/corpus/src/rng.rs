// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use fileroom::conventions::Identifier;

/// SplitMix64: small, seedable, and the same on every platform.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn draw(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.draw() % n.max(1)
    }

    pub fn range(&mut self, low: i64, high: i64) -> i64 {
        low + self.below((high - low + 1) as u64) as i64
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }

    /// A UUID version 7 whose timestamp is the instant given, in milliseconds.
    pub fn uuid7(&mut self, unix_ms: u64) -> Identifier {
        let r = self.draw();
        let s = format!(
            "{:08x}-{:04x}-7{:03x}-{:04x}-{:012x}",
            (unix_ms >> 16) & 0xFFFF_FFFF,
            unix_ms & 0xFFFF,
            (r >> 52) & 0xFFF,
            0x8000 | ((r >> 36) & 0x3FFF),
            r & 0xFFFF_FFFF_FFFF
        );
        Identifier::parse(&s).expect("well formed by construction")
    }
}
