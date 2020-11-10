//
// Copyright 2020 The Project Oak Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//

//! Shared data structures and functionality for inter-node communication.

mod decodable;
mod encodable;
mod error;
pub mod handle;
mod receiver;
mod sender;

pub use decodable::Decodable;
use either::Either;
pub use encodable::Encodable;
pub use error::OakError;
use handle::{ReadHandle, WriteHandle};
pub use oak_abi::{Handle, OakStatus};
pub use receiver::Receiver;
pub use sender::Sender;

use proptest_derive::Arbitrary;

/// A simple holder for bytes + handles, using internally owned buffers.
#[derive(Clone, PartialEq, Eq)]
pub struct Message {
    pub bytes: Vec<u8>,
    pub handles: Vec<Handle>,
}

/// Manual `Debug` implementation to allow hex display.
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Message")
            .field("handles", &self.handles)
            .field("bytes", &format_args!("{}", hex::encode(&self.bytes)))
            .finish()
    }
}

/// Implementation of [`Encodable`] for [`Either`] that encodes the variant (0 or 1) in the first
/// byte of the resulting [`Message`].
impl<L: Encodable, R: Encodable> Encodable for Either<L, R> {
    fn encode(&self) -> Result<Message, OakError> {
        let (variant, mut inner) = match self {
            // Left variant == 0.
            Either::Left(m) => (0, m.encode()?),
            // Right variant == 1.
            Either::Right(m) => (1, m.encode()?),
        };
        // Insert the variant byte as the beginning of the message bytes, and leave handles as they
        // are.
        inner.bytes.insert(0, variant);
        Ok(inner)
    }
}

/// Implementation of [`Decodable`] for [`Either`] that decodes the variant (0 or 1) from the first
/// byte of the provided [`Message`].
impl<L: Decodable, R: Decodable> Decodable for Either<L, R> {
    fn decode(message: &Message) -> Result<Self, OakError> {
        match message.bytes.get(0) {
            // Left variant == 0.
            Some(0) => {
                let inner_message = Message {
                    bytes: message.bytes[1..].to_vec(),
                    handles: message.handles.clone(),
                };
                Ok(Either::Left(L::decode(&inner_message)?))
            }
            // Right variant == 1.
            Some(1) => {
                let inner_message = Message {
                    bytes: message.bytes[1..].to_vec(),
                    handles: message.handles.clone(),
                };
                Ok(Either::Right(R::decode(&inner_message)?))
            }
            // Invalid variant, or not enough bytes.
            _ => Err(OakStatus::ErrInvalidArgs.into()),
        }
    }
}

/// A wrapper struct that holds an init message, plus the handle of a channel from which to read
/// command messages. This is useful for patterns in which an Oak node needs some data at
/// initialization time, but then handles commands of a different type and possibly coming from a
/// different channel, to be processed in an event-loop pattern.
#[derive(Arbitrary, Clone, Debug, PartialEq)]
pub struct InitWrapper<Init, Command: Decodable> {
    pub init: Init,
    pub command_receiver: Receiver<Command>,
}

/// Implementation of [`Encodable`] for [`InitWrapper`] that encodes the handle of the command
/// receiver channel in the first handle of the resulting [`Message`].
impl<Init: Encodable, Command: Encodable + Decodable> Encodable for InitWrapper<Init, Command> {
    fn encode(&self) -> Result<Message, OakError> {
        let mut init_message = self.init.encode()?;
        // Insert the handle of the command receiver handle at the beginning of the handle list
        // (i.e. at position 0).
        init_message
            .handles
            .insert(0, self.command_receiver.handle.handle);
        Ok(init_message)
    }
}

/// Implementation of [`Decodable`] for [`InitWrapper`] that decodes the handle of the command
/// receiver channel from the first handle of the provided [`Message`].
impl<Init: Decodable, Command: Decodable> Decodable for InitWrapper<Init, Command> {
    fn decode(message: &Message) -> Result<Self, OakError> {
        match message.handles.get(0) {
            Some(handle) => {
                let init_message = Message {
                    bytes: message.bytes.clone(),
                    handles: message.handles[1..].to_vec(),
                };
                Ok(Self {
                    init: Init::decode(&init_message)?,
                    command_receiver: Receiver::new(ReadHandle::from(*handle)),
                })
            }
            _ => Err(OakStatus::ErrInvalidArgs.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    use proptest::prelude::*;

    #[derive(Arbitrary, Debug, PartialEq)]
    struct TestStruct {
        receiver_0: Receiver<()>,
        field_0: u8,
        receiver_1: Receiver<()>,
        field_1: u8,
    }

    impl Encodable for TestStruct {
        fn encode(&self) -> Result<Message, OakError> {
            Ok(Message {
                bytes: vec![self.field_0, self.field_1],
                handles: vec![self.receiver_0.handle.handle, self.receiver_1.handle.handle],
            })
        }
    }
    impl Decodable for TestStruct {
        fn decode(message: &Message) -> Result<Self, OakError> {
            match (message.bytes.as_slice(), message.handles.as_slice()) {
                ([byte_0, byte_1], [handle_0, handle_1]) => Ok(TestStruct {
                    receiver_0: Receiver::from(ReadHandle::from(*handle_0)),
                    field_0: *byte_0,
                    receiver_1: Receiver::from(ReadHandle::from(*handle_1)),
                    field_1: *byte_1,
                }),
                _ => panic!("invalid Message for TestStruct"),
            }
        }
    }

    proptest!{
    #[test]
    fn test_struct_round_trip(original: TestStruct) {
        let encoded = original.encode().unwrap();
        // It is not clear that this assertion adds any value at all since it is so similar
        // to the definition of encode::<TestStruct>
        assert_eq!(
            Message {
                bytes: vec![original.field_0, original.field_1],
                handles: vec![original.receiver_0.handle.handle, original.receiver_1.handle.handle],
            },
            encoded
        );
        let decoded = TestStruct::decode(&encoded).unwrap();
        assert_eq!(original, decoded);
        // todo: should there round-trip take one further step to check that 
        //     encode(decode(encode(original))) == encode(original)
        // alternatively, should there be a second test that (forall x. encode(decode(x)) == x)
    }
    }

    type T = Either<u32, TestStruct>;

    // generate an arbitrary value of type T
    fn arb_t() -> impl Strategy<Value = T> {
        prop_oneof![
            any::<u32>().prop_map(Either::Left),
            any::<TestStruct>().prop_map(Either::Right),
        ].boxed()
    }

    proptest!{
        #[test]
        fn either_round_trip_u32(original: u32) {
            {
                let encoded = original.encode().unwrap();
                // omitted: check that the encoding is as expected
                let decoded = u32::decode(&encoded).unwrap();
                assert_eq!(original, decoded);
            }
        }
    }

    proptest!{
        #[test]
        fn either_round_trip_t(original in arb_t()) {
            {
                let encoded = original.encode().unwrap();
                // omitted: check that the encoding is as expected
                let decoded = T::decode(&encoded).unwrap();
                assert_eq!(original, decoded);
            }
        }
    }

    // todo: it is not clear how best to generalize this original test
    // Maybe it is good enough as it is?
    #[test]
    fn either_round_trip_invalid() {
        let invalid_variant = 42;
        let encoded_invalid = Message {
            bytes: vec![invalid_variant, 8, 196, 15],
            handles: vec![],
        };
        assert_matches!(
            T::decode(&encoded_invalid),
            Err(OakError::OakStatus(OakStatus::ErrInvalidArgs))
        );
    }

    type I = InitWrapper<TestStruct, ()>;

    proptest!{
        #[test]
        fn init_wrapper_round_trip(original: I) {
            {
                let encoded = original.encode().unwrap();
                // omitted: check that encoding is as expected
                let decoded = I::decode(&encoded).unwrap();
                assert_eq!(original, decoded);
            }
        }
    }
}
