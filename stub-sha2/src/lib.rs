pub struct Sha256 {
}

impl Sha256 {
    pub fn new() -> Self {
        Sha256{}
    }
    pub fn update(&mut self, _input: impl AsRef<[u8]>) {
    }
    pub fn finalize(self) -> [u8; 0] {
        []
    }
}

pub mod digest {
    pub trait Digest {}
}
