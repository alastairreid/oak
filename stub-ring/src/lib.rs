pub mod rand {
    pub mod SystemRandom {
        pub struct SecureRandom {}
        pub fn new() -> SecureRandom {
            SecureRandom{}
        }
    }
}

pub mod signature {
    use super::rand::SystemRandom::*;
    use core::fmt::Debug;

    pub struct UnparsedPublicKey<B: AsRef<[u8]>> {
        bytes: B,
    }
    impl<B: AsRef<[u8]>> UnparsedPublicKey<B> {
        pub fn new(algorithm: &'static dyn VerificationAlgorithm, bytes: B) -> Self {
            Self { bytes }
        }
        pub fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), Unspecified> {
            Ok(())
        }
    }
    mod sealed { // not public so this trait is private
        pub trait Sealed {}
    }
    pub trait VerificationAlgorithm: Debug + Sync + sealed::Sealed {
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct EdDSAParameters;
    pub static ED25519: EdDSAParameters = EdDSAParameters {};
    impl sealed::Sealed for EdDSAParameters {}
    impl VerificationAlgorithm for EdDSAParameters { }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Ed25519KeyPair {}

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Signature;
    impl AsRef<[u8]> for Signature {
        fn as_ref(&self) -> &[u8] { &[] }
    }

    pub trait KeyPair: Debug + Send + Sized + Sync {
        type PublicKey: AsRef<[u8]> + Debug + Clone + Send + Sized + Sync;
        fn public_key(&self) -> &Self::PublicKey;
        fn sign(&self, msg: &[u8]) -> Signature;
    }

    impl KeyPair for Ed25519KeyPair {
        type PublicKey = [u8; 0];
        fn public_key(&self) -> &Self::PublicKey {
            &[]
        }
        fn sign(&self, msg: &[u8]) -> Signature {
            Signature{}
        }
    }

    pub struct Document {}
    impl AsRef<[u8]> for Document {
        fn as_ref(&self) -> &[u8] { &[] }
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Unspecified {}

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct KeyRejected {}
    impl Ed25519KeyPair {
        pub fn generate_pkcs8(rng: &SecureRandom) -> Result<Document, Unspecified> {
            return Ok(Document{})
        }
        pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Self, KeyRejected> {
            return Ok(Ed25519KeyPair{})
        }
    }
}
