use std::hash::{BuildHasherDefault, Hasher};

#[derive(Default)]
pub struct IdentityHasher(u64);

pub type BuildIdentityHasher = BuildHasherDefault<IdentityHasher>;

impl Hasher for IdentityHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, _: &[u8]) {
        panic!("only write_u64 supported")
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}
