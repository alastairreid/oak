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

use super::*;
use maplit::{hashmap, hashset};
use oak_abi::{
    label::{
        authorization_bearer_token_hmac_tag, confidentiality_label, public_key_identity_tag,
        tls_endpoint_tag, web_assembly_module_signature_tag, web_assembly_module_tag, Label,
    },
    proto::oak::application::{
        node_configuration::ConfigType, ApplicationConfiguration, GrpcServerConfiguration,
        LogConfiguration, NodeConfiguration,
    },
};
use std::sync::mpsc;
use proptest::prelude::*;

pub fn init_logging() {
    let _ = env_logger::builder().is_test(true).try_init();
}

type NodeBody = dyn FnOnce(RuntimeProxy) -> Result<(), OakStatus> + Send + Sync;

/// Runs the provided function as if it were the body of a [`Node`] implementation, which is
/// instantiated by the [`Runtime`] with the provided [`Label`].
fn run_node_body(node_label: &Label, node_privilege: &NodePrivilege, node_body: Box<NodeBody>) {
    init_logging();
    let configuration = ApplicationConfiguration {
        wasm_modules: hashmap! {},
        initial_node_configuration: None,
    };
    let signature_table = SignatureTable::default();
    info!("Create runtime for test");
    let proxy = crate::RuntimeProxy::create_runtime(
        &configuration,
        &SecureServerConfiguration {
            grpc_config: Some(GrpcConfiguration {
                grpc_server_tls_identity: Some(Identity::from_pem(
                    include_str!("../../examples/certs/local/local.pem"),
                    include_str!("../../examples/certs/local/local.key"),
                )),
                grpc_client_root_tls_certificate: Some(
                    crate::config::load_certificate(&include_str!(
                        "../../examples/certs/local/ca.pem"
                    ))
                    .unwrap(),
                ),
                oidc_client_info: None,
            }),
            http_config: None,
        },
        &signature_table,
    );

    struct TestNode {
        node_body: Box<NodeBody>,
        node_privilege: NodePrivilege,
        result_sender: mpsc::SyncSender<Result<(), OakStatus>>,
    };

    impl crate::node::Node for TestNode {
        fn node_type(&self) -> &'static str {
            "test"
        }
        fn isolation(&self) -> NodeIsolation {
            // Even though this node is not actually sandboxed, we are simulating a Wasm node during
            // testing.
            NodeIsolation::Sandboxed
        }
        fn run(
            self: Box<Self>,
            runtime: RuntimeProxy,
            _handle: oak_abi::Handle,
            _notify_receiver: oneshot::Receiver<()>,
        ) {
            // Run the test body.
            let result = (self.node_body)(runtime);
            // Make the result of the test visible outside of this thread.
            self.result_sender.send(result).unwrap();
        }

        fn get_privilege(&self) -> NodePrivilege {
            self.node_privilege.clone()
        }
    }

    let (result_sender, result_receiver) = mpsc::sync_channel(1);

    // Create a new Oak node.
    let node_instance = TestNode {
        node_body,
        node_privilege: node_privilege.clone(),
        result_sender,
    };
    let (_write_handle, read_handle) = proxy
        .channel_create("Initial", &Label::public_untrusted())
        .expect("Could not create init channel");

    proxy
        .node_register(Box::new(node_instance), "test", node_label, read_handle)
        .expect("Could not create Oak node!");

    // Wait for the test Node to complete execution before terminating the Runtime.
    let result_value = result_receiver
        .recv()
        .expect("test node disconnected, probably due to panic/assert fail in test");
    assert_eq!(result_value, Ok(()));

    info!("Stop runtime..");
    proxy.runtime.stop();
    info!("Stop runtime..done");
}

/// Returns a non-trivial label for testing.
fn test_label() -> Label {
    Label {
        confidentiality_tags: vec![oak_abi::label::authorization_bearer_token_hmac_tag(&[
            1, 1, 1,
        ])],
        integrity_tags: vec![],
    }
}

fn arb_authentication_tag() -> impl Strategy<Value = Tag> {
    any::<Vec<u8>>()
        .prop_filter("hmacs must be non-empty", |hmac| !hmac.is_empty())
        .prop_map(|hmac| authorization_bearer_token_hmac_tag(&hmac)).boxed()
}

fn arb_wasm_module_tag() -> impl Strategy<Value = Tag> {
    any::<Vec<u8>>()
        .prop_filter("shas must be non-empty", |sha| !sha.is_empty())
        .prop_map(|sha| web_assembly_module_tag(&sha)).boxed()
}

fn arb_wasm_module_signature_tag() -> impl Strategy<Value = Tag> {
    any::<Vec<u8>>()
        .prop_filter("shas must be non-empty", |sha| !sha.is_empty())
        .prop_map(|sha| web_assembly_module_signature_tag(&sha)).boxed()
}

fn arb_public_key_identity_tag() -> impl Strategy<Value = Tag> {
    any::<Vec<u8>>()
        .prop_filter("keys must be non-empty", |key| !key.is_empty())
        .prop_map(|key| public_key_identity_tag(key)).boxed()
}

fn arb_tls_endpoint_tag() -> impl Strategy<Value = Tag> {
   ".*" 
        .prop_map(|authority| tls_endpoint_tag(&authority)).boxed()
}

fn arb_tag() -> impl Strategy<Value = Tag> {
    prop_oneof![
        arb_wasm_module_tag(),
        arb_wasm_module_signature_tag(),
        arb_authentication_tag(),
        arb_public_key_identity_tag(),
        arb_tls_endpoint_tag(),
    ]
}

fn arb_tags(size: core::ops::Range<usize>) -> impl Strategy<Value = Vec<Tag>> {
    prop::collection::vec(arb_tag(), size)
}

fn arb_label(size: core::ops::Range<usize>) -> impl Strategy<Value = Label> {
    arb_tags(size).prop_map(|tags|
        Label {
            confidentiality_tags: tags,
            integrity_tags: vec![],
        }
    )
}

// generate a label with an arbitrary (non-empty) list of confidentiality tags
// and no integrity tags
fn arb_auth_label() -> impl Strategy<Value = Label> {
    // let ctags = prop::collection::vec(arb_authentication_tag(), 1..2); // must be non-empty
    let ctag = arb_authentication_tag();
    ctag.prop_map(|c|
        Label {
            confidentiality_tags: vec![c],
            integrity_tags: vec![],
        }).boxed()
}

fn arb_message() -> impl Strategy<Value = NodeMessage> {
    any::<Vec<u8>>()
        .prop_map(|bytes|
            NodeMessage {
                bytes: bytes,
                handles: vec![],
            }
        )
}

/// Checks that a panic in the node body actually causes the test case to fail, and does not
/// accidentally get ignored.
#[test]
#[ignore]
#[should_panic]
fn panic_check() {
    let label = test_label();
    run_node_body(
        &label,
        &NodePrivilege::default(),
        Box::new(|_runtime| {
            panic!("testing that panic works");
        }),
    );
}

proptest!{
/// Create a test Node with a non-public confidentiality label and no downgrading privilege that
/// creates a Channel with the same label and fails.
///
/// Only Nodes with a public confidentiality label may create other Nodes and Channels.
#[test]
fn create_channel_same_label_err(
        label in arb_auth_label(),
    ) {
    let label_clone = label.clone();
    run_node_body(
        &label,
        &NodePrivilege::default(),
        Box::new(move |runtime| {
            // Attempt to perform an operation that requires the [`Runtime`] to have created an
            // appropriate [`NodeInfo`] instance.
            let result = runtime.channel_create("", &label_clone);
            assert_eq!(Err(OakStatus::ErrPermissionDenied), result);
            Ok(())
        }),
    );
}
}

proptest!{
/// Create a test Node with a non-public confidentiality label and no downgrading privilege that
/// creates a Channel with a less confidential label and fails.
///
/// Only Nodes with a public confidentiality label may create other Nodes and Channels.
#[test]
fn create_channel_less_confidential_label_err(tag_0 in arb_authentication_tag(), tag_1 in arb_authentication_tag()) {
    // todo: could make this better by replacing tag_1 with a list of tags
    // todo: is it worth having a strategy for generating a label strictly
    //       weaker than some other strategy
    let initial_label = Label {
        confidentiality_tags: vec![tag_0, tag_1.clone()],
        integrity_tags: vec![],
    };
    let less_confidential_label = Label {
        confidentiality_tags: vec![tag_1],
        integrity_tags: vec![],
    };
    run_node_body(
        &initial_label,
        &NodePrivilege::default(),
        Box::new(move |runtime| {
            let result = runtime.channel_create("", &less_confidential_label);
            assert_eq!(Err(OakStatus::ErrPermissionDenied), result);
            Ok(())
        }),
    );
}
}

proptest!{
/// Create a test Node with a non-public confidentiality label and some downgrading privilege that
/// creates a Channel with a less confidential label and fails.
///
/// Only Nodes with a public confidentiality label may create other Nodes and Channels.
#[test]
fn create_channel_less_confidential_label_declassification_err(
        tag_0 in arb_authentication_tag(),
        tag_1 in arb_authentication_tag(),
        other_tag in arb_authentication_tag(), // todo: probably != tag_0/1
    ) {
    let initial_label = Label {
        confidentiality_tags: vec![tag_0.clone(), tag_1.clone()],
        integrity_tags: vec![],
    };
    let less_confidential_label = Label {
        confidentiality_tags: vec![tag_1],
        integrity_tags: vec![],
    };
    run_node_body(
        &initial_label,
        // Grant this node the privilege to declassify `tag_0` and another unrelated tag, and
        // endorse another unrelated tag.
        &NodePrivilege {
            can_declassify_confidentiality_tags: hashset! { tag_0, other_tag.clone() },
            can_endorse_integrity_tags: hashset! { other_tag },
        },
        Box::new(move |runtime| {
            let result = runtime.channel_create("", &less_confidential_label);
            assert_eq!(Err(OakStatus::ErrPermissionDenied), result);
            Ok(())
        }),
    );
}
}

// Note: I suspect that error tests in general are like this test (and others
// in this file) in that the test will fail only if two values are the same.
// This is a poor match for blind fuzzing because random 64-bit values are
// really unlikely to collide.
// One way to fix this would be to have a small number of example values that
// are selected with very high probability.
proptest!{
/// Create a test Node with a non-public confidentiality label that creates a Channel with a less
/// confidential label and fails.
///
/// Only Nodes with a public confidentiality label may create other Nodes and Channels.
#[test]
fn create_channel_less_confidential_label_no_privilege_err(
        tag_0 in arb_authentication_tag(),
        tag_1 in arb_authentication_tag(),
    ) {
    prop_assume!(tag_0 != tag_1);
    // todo: it would be better to use two arbitrary labels instead of two arbitrary tags?
    // todo: should use an arbitrary set of integrity tags
    let initial_label = Label {
        confidentiality_tags: vec![tag_0.clone(), tag_1.clone()],
        integrity_tags: vec![],
    };
    let less_confidential_label = Label {
        confidentiality_tags: vec![tag_1],
        integrity_tags: vec![],
    };
    run_node_body(
        &initial_label,
        // Grant this node the privilege to endorse (rather than declassify) `tag_0`, which in this
        // case is useless, so it should still fail.
        &NodePrivilege {
            can_declassify_confidentiality_tags: hashset! {},
            can_endorse_integrity_tags: hashset! { tag_0 },
        },
        Box::new(move |runtime| {
            let result = runtime.channel_create("", &less_confidential_label);
            assert_eq!(Err(OakStatus::ErrPermissionDenied), result);
            Ok(())
        }),
    );
}
}

proptest!{
/// Create a test Node with public confidentiality label and no privilege that:
///
/// - creates a Channel with a more confidential label and succeeds
/// - writes to the newly created channel and succeeds
/// - reads from the newly created channel and fails
///
/// Data is always allowed to flow to more confidential labels.
#[test]
fn create_channel_with_more_confidential_label_from_public_untrusted_node_ok(
        tag_0 in arb_authentication_tag(),
        message in arb_message(),
    ) {
    // todo: better to generate a pair of arbitrary labels than just a tag
    let initial_label = &Label::public_untrusted();
    let more_confidential_label = Label {
        confidentiality_tags: vec![tag_0],
        integrity_tags: vec![],
    };
    run_node_body(
        &initial_label,
        &NodePrivilege::default(),
        Box::new(move |runtime| {
            let result = runtime.channel_create("", &more_confidential_label);
            assert_eq!(true, result.is_ok());

            let (write_handle, read_handle) = result.unwrap();

            {
                // Writing to a more confidential Channel is always allowed.
                let result = runtime.channel_write(write_handle, message);
                assert_eq!(Ok(()), result);
            }

            {
                // Reading from a more confidential Channel is not allowed.
                let result = runtime.channel_read(read_handle);
                assert_eq!(Err(OakStatus::ErrPermissionDenied), result);
            }

            Ok(())
        }),
    );
}
}

proptest!{
/// Create a test Node with public confidentiality label and downgrading privilege that:
///
/// - creates a Channel with a more confidential label and succeeds (same as previous test case)
/// - writes to the newly created channel and succeeds (same as previous test case)
/// - reads from the newly created channel and succeeds (different from previous test case, thanks
///   to the newly added privilege)
#[test]
fn create_channel_with_more_confidential_label_from_public_node_with_privilege_ok(
        // todo: better to create a pair of labels? but we need to refer to the tag in 
        // NodePrivilege - so not completely trivial.
        tag_0 in arb_authentication_tag(),
        message in arb_message(),
    ) {
    let initial_label = Label::public_untrusted();
    let more_confidential_label = Label {
        confidentiality_tags: vec![tag_0.clone()],
        integrity_tags: vec![],
    };
    run_node_body(
        &initial_label,
        &NodePrivilege {
            can_declassify_confidentiality_tags: hashset! { tag_0 },
            can_endorse_integrity_tags: hashset! {},
        },
        Box::new(move |runtime| {
            let result = runtime.channel_create("", &more_confidential_label);
            assert_eq!(true, result.is_ok());

            let (write_handle, read_handle) = result.unwrap();

            {
                // Writing to a more confidential Channel is always allowed.
                let result = runtime.channel_write(write_handle, message.clone());
                assert_eq!(Ok(()), result);
            }

            {
                // Reading from a more confidential Channel is allowed because of the privilege.
                let result = runtime.channel_read(read_handle);
                assert_eq!(Ok(Some(message)), result);
            }

            Ok(())
        }),
    );
}
}

proptest!{
/// Create a test Node with public confidentiality label and infinite privilege that:
///
/// - creates a Channel with a more confidential label and succeeds (same as previous test case)
/// - writes to the newly created channel and succeeds (same as previous test case)
/// - reads from the newly created channel and succeeds (same as previous test case, this time
///   thanks to the infinite privilege)
#[test]
fn create_channel_with_more_confidential_label_from_public_node_with_top_privilege_ok(
        // todo: better to create a pair of labels? but we need to refer to the tag in 
        // NodePrivilege - so not completely trivial.
        more_confidential_label in arb_label(1..2),
        message in arb_message(),
    ) {
    let initial_label = Label::public_untrusted();
    run_node_body(
        &initial_label,
        &NodePrivilege::top_privilege(),
        Box::new(move |runtime| {
            let result = runtime.channel_create("", &more_confidential_label);
            assert_eq!(true, result.is_ok());

            let (write_handle, read_handle) = result.unwrap();

            {
                // Writing to a more confidential Channel is always allowed.
                let result = runtime.channel_write(write_handle, message.clone());
                assert_eq!(Ok(()), result);
            }

            {
                // Reading from a more confidential Channel is allowed because of the privilege.
                let result = runtime.channel_read(read_handle);
                assert_eq!(Ok(Some(message)), result);
            }

            Ok(())
        }),
    );
}
}

proptest!{
#[test]
fn create_channel_with_more_confidential_label_from_non_public_node_with_privilege_err(
        tag_0 in arb_authentication_tag(),
        tag_1 in arb_authentication_tag(),
    ) {
    prop_assume!(tag_0 != tag_1);
    let initial_label = Label {
        confidentiality_tags: vec![tag_0.clone()],
        integrity_tags: vec![],
    };
    let more_confidential_label = Label {
        confidentiality_tags: vec![tag_0, tag_1.clone()],
        integrity_tags: vec![],
    };
    run_node_body(
        &initial_label,
        &NodePrivilege {
            can_declassify_confidentiality_tags: hashset! { tag_1 },
            can_endorse_integrity_tags: hashset! {},
        },
        Box::new(move |runtime| {
            let result = runtime.channel_create("", &more_confidential_label);
            assert_eq!(Err(OakStatus::ErrPermissionDenied), result);
            Ok(())
        }),
    );
}
}

/// Create a test Node that creates a Node with the same public untrusted label and succeeds.
#[test]
fn create_node_same_label_ok() {
    let label = Label::public_untrusted();
    let label_clone = label.clone();
    run_node_body(
        &label,
        &NodePrivilege::default(),
        Box::new(move |runtime| {
            let (_write_handle, read_handle) = runtime.channel_create("", &label_clone)?;
            let node_configuration = NodeConfiguration {
                config_type: Some(ConfigType::LogConfig(LogConfiguration {})),
            };
            let result =
                runtime.node_create("test", &node_configuration, &label_clone, read_handle);
            assert_eq!(Ok(()), result);
            Ok(())
        }),
    );
}

/// Create a test Node that creates a Node with an invalid configuration and fails.
#[test]
fn create_node_invalid_configuration_err() {
    let label = Label::public_untrusted();
    let label_clone = label.clone();
    run_node_body(
        &label,
        &NodePrivilege::default(),
        Box::new(move |runtime| {
            let (_write_handle, read_handle) = runtime.channel_create("", &label_clone)?;
            // Node configuration without config type.
            let node_configuration = NodeConfiguration { config_type: None };
            let result =
                runtime.node_create("test", &node_configuration, &label_clone, read_handle);
            assert_eq!(Err(OakStatus::ErrInvalidArgs), result);
            Ok(())
        }),
    );
}

proptest!{
/// Create a test Node with a non public_trusted label, which is then unable to create channels
/// of any sort, regardless of label.
#[test]
fn create_channel_by_nonpublic_node_err(
        tag_0 in arb_authentication_tag(),
        tag_1 in arb_authentication_tag(),
    ) {
    prop_assume!(tag_0 != tag_1);
    let initial_label = Label {
        confidentiality_tags: vec![tag_0.clone()],
        integrity_tags: vec![],
    };
    let less_confidential_label = Label {
        confidentiality_tags: vec![],
        integrity_tags: vec![],
    };
    let more_confidential_label = Label {
        confidentiality_tags: vec![tag_0, tag_1],
        integrity_tags: vec![],
    };
    let initial_label_clone = initial_label.clone();
    run_node_body(
        &initial_label,
        &NodePrivilege::default(),
        Box::new(move |runtime| {
            let result = runtime.channel_create("test-same-label", &initial_label_clone);
            assert_eq!(Err(OakStatus::ErrPermissionDenied), result);
            let result = runtime.channel_create("test-less-label", &less_confidential_label);
            assert_eq!(Err(OakStatus::ErrPermissionDenied), result);
            let result = runtime.channel_create("test-more-label", &more_confidential_label);
            assert_eq!(Err(OakStatus::ErrPermissionDenied), result);
            Ok(())
        }),
    );
}
}

proptest!{
/// Create a public_untrusted test Node that creates a Node with a more confidential label and
/// succeeds.
#[test]
fn create_node_more_confidential_label_ok(
        tag_0 in arb_authentication_tag(),
        tag_1 in arb_authentication_tag(),
    ) {
    prop_assume!(tag_0 != tag_1);
    let initial_label = Label::public_untrusted();
    let more_confidential_label = Label {
        confidentiality_tags: vec![tag_0.clone()],
        integrity_tags: vec![],
    };
    let even_more_confidential_label = Label {
        confidentiality_tags: vec![tag_0, tag_1],
        integrity_tags: vec![],
    };
    let initial_label_clone = initial_label.clone();
    run_node_body(
        &initial_label,
        &NodePrivilege::default(),
        Box::new(move |runtime| {
            let (_write_handle, read_handle) = runtime.channel_create("", &initial_label_clone)?;
            let node_configuration = NodeConfiguration {
                config_type: Some(ConfigType::GrpcServerConfig(GrpcServerConfiguration {
                    // todo: can/should we generalize this hardcoded string?
                    address: "[::]:6502".to_string(),
                })),
            };
            let result = runtime.node_create(
                "test",
                &node_configuration,
                &more_confidential_label,
                read_handle,
            );
            assert_eq!(Ok(()), result);
            let result = runtime.node_create(
                "test",
                &node_configuration,
                &even_more_confidential_label,
                read_handle,
            );
            assert_eq!(Ok(()), result);
            Ok(())
        }),
    );
}
}

// todo: what is the right way to generalize this?
// perhaps generate a sequence of N channel creates
// and a sequence of N bools to close them
// and then check that only the closed ones are Orphaned
#[test]
fn wait_on_channels_immediately_returns_if_any_channel_is_orphaned() {
    let label = Label::public_untrusted();
    let label_clone = label.clone();
    run_node_body(
        &label,
        &NodePrivilege::default(),
        Box::new(move |runtime| {
            let (write_handle_0, read_handle_0) = runtime.channel_create("", &label_clone)?;
            let (_write_handle_1, read_handle_1) = runtime.channel_create("", &label_clone)?;

            // Close the write_handle; this should make the channel Orphaned
            let result = runtime.channel_close(write_handle_0);
            assert_eq!(Ok(()), result);

            let result = runtime.wait_on_channels(&[read_handle_0, read_handle_1]);
            assert_eq!(
                Ok(vec![
                    ChannelReadStatus::Orphaned,
                    ChannelReadStatus::NotReady
                ]),
                result
            );
            Ok(())
        }),
    );
}

#[test]
fn wait_on_channels_blocks_if_all_channels_have_status_not_ready() {
    let label = Label::public_untrusted();
    let label_clone = label.clone();
    run_node_body(
        &label,
        &NodePrivilege::default(),
        Box::new(move |runtime| {
            let (write_handle, read_handle) = runtime.channel_create("", &label_clone)?;

            // Change the status of the channel concurrently, to unpark the waiting thread.
            let runtime_copy = runtime.clone();
            let start = std::time::Instant::now();
            std::thread::spawn(move || {
                let ten_millis = std::time::Duration::from_millis(10);
                thread::sleep(ten_millis);

                // Close the write_handle; this should make the channel Orphaned
                let result = runtime_copy.channel_close(write_handle);
                assert_eq!(Ok(()), result);
            });

            let result = runtime.wait_on_channels(&[read_handle]);
            assert!(start.elapsed() >= std::time::Duration::from_millis(10));
            assert_eq!(Ok(vec![ChannelReadStatus::Orphaned]), result);
            Ok(())
        }),
    );
}

#[test]
fn wait_on_channels_immediately_returns_if_any_channel_is_invalid() {
    let label = Label::public_untrusted();
    let label_clone = label.clone();
    run_node_body(
        &label,
        &NodePrivilege::default(),
        Box::new(move |runtime| {
            let (write_handle, _read_handle) = runtime.channel_create("", &label_clone)?;
            let (_write_handle, read_handle) = runtime.channel_create("", &label_clone)?;

            let result = runtime.wait_on_channels(&[write_handle, read_handle]);
            assert_eq!(
                Ok(vec![
                    ChannelReadStatus::InvalidChannel,
                    ChannelReadStatus::NotReady
                ]),
                result
            );
            Ok(())
        }),
    );
}

#[test]
fn wait_on_channels_immediately_returns_if_the_input_list_is_empty() {
    let label = Label::public_untrusted();
    run_node_body(
        &label,
        &NodePrivilege::default(),
        Box::new(|runtime| {
            let result = runtime.wait_on_channels(&[]);
            assert_eq!(Ok(Vec::<ChannelReadStatus>::new()), result);
            Ok(())
        }),
    );
}

proptest!{
#[test]
/// The top privilege can downgrade any label to "public".
fn downgrade_one_label_using_top_privilege(
        label in arb_tag().prop_map(confidentiality_label),
    ) {
    init_logging();

    assert!(NodePrivilege::top_privilege()
        .downgrade_label(&label)
        .flows_to(&Label::public_untrusted()));
}
}

proptest!{
#[test]
/// The top privilege can downgrade any label to "public".
fn downgrade_multiple_labels_using_top_privilege(
        mixed_label in arb_label(0..10),
    ) {
    init_logging();
    assert!(NodePrivilege::top_privilege()
        .downgrade_label(&mixed_label)
        .flows_to(&Label::public_untrusted()));
}
}

proptest!{
#[test]
fn downgrade_tls_label_using_tls_privilege(
        tls_endpoint_tag_1 in arb_tls_endpoint_tag(),
        tls_endpoint_tag_2 in arb_tls_endpoint_tag(),
    ) {
    prop_assume!(tls_endpoint_tag_1 != tls_endpoint_tag_2);
    init_logging();
    let tls_privilege = NodePrivilege {
        can_declassify_confidentiality_tags: hashset! { tls_endpoint_tag_1.clone() },
        can_endorse_integrity_tags: hashset! {},
    };

    let tls_endpoint_label_1 = confidentiality_label(tls_endpoint_tag_1.clone());
    let tls_endpoint_label_2 = confidentiality_label(tls_endpoint_tag_2.clone());
    let mixed_tls_endpoint_label = Label {
        confidentiality_tags: vec![tls_endpoint_tag_1, tls_endpoint_tag_2],
        integrity_tags: vec![],
    };

    // Can downgrade the label with the same TLS endpoint tag.
    assert!(tls_privilege
        .downgrade_label(&tls_endpoint_label_1)
        .flows_to(&Label::public_untrusted()));
    // Cannot downgrade the label with a different TLS endpoint tag.
    assert!(!tls_privilege
        .downgrade_label(&tls_endpoint_label_2)
        .flows_to(&Label::public_untrusted()));
    // Can partially downgrade the combined label.
    assert!(tls_privilege
        .downgrade_label(&mixed_tls_endpoint_label)
        .flows_to(&tls_endpoint_label_2));
    assert!(!tls_privilege
        .downgrade_label(&mixed_tls_endpoint_label)
        .flows_to(&tls_endpoint_label_1));
}
}

proptest!{
#[test]
fn downgrade_wasm_label_using_signature_privilege_does_not_do_anything(
        signature_tag in arb_wasm_module_signature_tag(),
        wasm_label in arb_wasm_module_tag().prop_map(confidentiality_label),
    ) {
    init_logging();
    let signature_privilege = NodePrivilege {
        can_declassify_confidentiality_tags: hashset! { signature_tag },
        can_endorse_integrity_tags: hashset! {},
    };

    // Signature privilege cannot downgrade a Wasm confidentiality label.
    assert!(!signature_privilege
        .downgrade_label(&wasm_label)
        .flows_to(&Label::public_untrusted()));
}
}
