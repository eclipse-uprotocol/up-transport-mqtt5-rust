/********************************************************************************
 * Copyright (c) 2023 Contributors to the Eclipse Foundation
 *
 * See the NOTICE file(s) distributed with this work for additional
 * information regarding copyright ownership.
 *
 * This program and the accompanying materials are made available under the
 * terms of the Apache License Version 2.0 which is available at
 * https://www.apache.org/licenses/LICENSE-2.0
 *
 * SPDX-License-Identifier: Apache-2.0
 ********************************************************************************/

use std::sync::Arc;

use async_trait::async_trait;

use up_rust::{UCode, UListener, UMessage, UStatus, UTransport, UUri};

use crate::Mqtt5Transport;

#[async_trait]
impl UTransport for Mqtt5Transport {
    async fn receive(
        &self,
        _source_filter: &UUri,
        _sink_filter: Option<&UUri>,
    ) -> Result<UMessage, UStatus> {
        // [impl->dsn~utransport-receive-error-unimplemented~1]
        Err(UStatus::fail_with_code(
            UCode::UNIMPLEMENTED,
            "not implemented",
        ))
    }

    async fn send(&self, message: UMessage) -> Result<(), UStatus> {
        // validate message
        // [impl->dsn~utransport-send-error-invalid-parameter~1]
        let attributes = message.attributes.as_ref().ok_or_else(|| {
            UStatus::fail_with_code(
                UCode::INVALID_ARGUMENT,
                "uProtocol message has no attributes",
            )
        })?;

        // Extract payload from umessage to send
        // [impl->dsn~up-transport-mqtt5-payload-mapping~1]
        let payload = message.payload;

        self.send_message(attributes, payload).await
    }

    // [impl->dsn~utransport-supported-message-delivery~1]
    // [impl->dsn~utransport-registerlistener-error-unimplemented~1]
    async fn register_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        // [impl->dsn~utransport-registerlistener-error-invalid-parameter~1]
        up_rust::verify_filter_criteria(source_filter, sink_filter)?;
        let topic = self
            .to_mqtt_topic_string(source_filter, sink_filter)
            .map_err(|e| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, e.to_string()))?;

        self.add_listener(&topic, listener).await
    }

    // [impl->dsn~utransport-supported-message-delivery~1]
    // [impl->dsn~utransport-unregisterlistener-error-unimplemented~1]
    async fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        // [impl->dsn~utransport-unregisterlistener-error-invalid-parameter~1]
        up_rust::verify_filter_criteria(source_filter, sink_filter)?;
        let topic: String = self
            .to_mqtt_topic_string(source_filter, sink_filter)
            .map_err(|e| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, e.to_string()))?;

        self.remove_listener(&topic, listener).await
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bytes::Bytes;
    use mockall::predicate::{always, eq};
    use protobuf::{EnumOrUnknown, MessageField};
    use up_rust::{
        ComparableListener, MockUListener, UAttributes, UMessageBuilder, UMessageType,
        UPayloadFormat, UUID,
    };

    use test_case::test_case;
    use tokio::sync::RwLock;

    use crate::{
        listener_registry::{ListenerRegistry, RegisteredListeners},
        mapping::MessageMapper,
        mqtt_client::MockMqttClientOperations,
        TransportMode,
    };

    use super::*;

    // Helper function to construct UMessage object for testing.
    fn create_test_message(
        message_type: UMessageType,
        source: &str,
        sink: Option<&str>,
        payload: &str,
    ) -> UMessage {
        let source_uri = UUri::from_str(source).expect("Expected a valid source value");

        match message_type {
            UMessageType::UMESSAGE_TYPE_PUBLISH => UMessageBuilder::publish(source_uri)
                .with_priority(up_rust::UPriority::UPRIORITY_CS1)
                .build_with_payload(payload.to_string(), UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
                .unwrap(),
            UMessageType::UMESSAGE_TYPE_REQUEST => {
                let sink_uri =
                    UUri::from_str(sink.expect("Expected a sink value for request message"))
                        .expect("Expected a valid sink value");

                UMessageBuilder::request(sink_uri, source_uri, 3600)
                    .with_priority(up_rust::UPriority::UPRIORITY_CS4)
                    .build_with_payload(payload.to_string(), UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
                    .unwrap()
            }
            UMessageType::UMESSAGE_TYPE_RESPONSE => {
                let sink_uri =
                    UUri::from_str(sink.expect("Expected a sink value for request message"))
                        .expect("Expected a valid sink value");

                UMessageBuilder::response(sink_uri, UUID::build(), source_uri)
                    .with_priority(up_rust::UPriority::UPRIORITY_CS4)
                    .build_with_payload(payload.to_string(), UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
                    .unwrap()
            }
            UMessageType::UMESSAGE_TYPE_NOTIFICATION => {
                let sink_uri =
                    UUri::from_str(sink.expect("Expected a sink value for notification message"))
                        .expect("Expected a valid sink value");

                UMessageBuilder::notification(source_uri, sink_uri)
                    .with_priority(up_rust::UPriority::UPRIORITY_CS1)
                    .build_with_payload(payload.to_string(), UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
                    .unwrap()
            }
            _ => panic!("Invalid message type"),
        }
    }

    #[test_case(
        create_test_message(
            UMessageType::UMESSAGE_TYPE_PUBLISH,
            "//vin.vehicles/A8000/2/8A50",
            None,
            "payload",
        ),
        "vin.vehicles/8000/A/2/8A50",
        None,
        None;
        "succeeds for Publish message"
    )]
    // [utest->dsn~utransport-send-error-invalid-parameter~1]
    #[test_case(
        UMessage {
            attributes: Some(UAttributes {
                type_: EnumOrUnknown::from(UMessageType::UMESSAGE_TYPE_PUBLISH),
                // Publish message must have source field
                source: MessageField::none(),
                ..Default::default()
            }).into(),
            ..Default::default()
        },
        "",
        None,
        Some(UCode::INVALID_ARGUMENT);
        "fails for invalid message"
    )]
    #[test_case(
        create_test_message(
            UMessageType::UMESSAGE_TYPE_PUBLISH,
            "//vin.vehicles/A8000/2/8A50",
            None,
            "payload",
        ),
        "vin.vehicles/8000/A/2/8A50",
        Some(UCode::UNAVAILABLE),
        Some(UCode::UNAVAILABLE);
        "fails if not connected to broker"
    )]
    #[test_case(
        create_test_message(
            UMessageType::UMESSAGE_TYPE_NOTIFICATION,
            "/A8000/2/1A50",
            Some("//vin.vehicles/B8000/3/0"),
            "payload",
        ),
        "vin.vehicles/8000/A/2/1A50/vin.vehicles/8000/B/3/0",
        None,
        None;
        "succeeds for Notification message"
    )]
    #[test_case(
        create_test_message(
            UMessageType::UMESSAGE_TYPE_REQUEST,
            "//vin.vehicles/A8000/2/0",
            Some("//vin.vehicles/B8000/3/10AB"),
            "payload",
        ),
        "vin.vehicles/8000/A/2/0/vin.vehicles/8000/B/3/10AB",
        None,
        None;
        "succeeds for Request message"
    )]
    #[test_case(
        create_test_message(
            UMessageType::UMESSAGE_TYPE_RESPONSE,
            "//vin.vehicles/B8000/3/10AB",
            Some("//vin.vehicles/A8000/2/0"),
            "payload",
        ),
        "vin.vehicles/8000/B/3/10AB/vin.vehicles/8000/A/2/0",
        None,
        None;
        "succeeds for Response message"
    )]
    #[tokio::test]
    async fn test_send(
        message_to_send: UMessage,
        expected_topic: &str,
        mqtt_client_error: Option<UCode>,
        expected_error_code: Option<UCode>,
    ) {
        let sent_message = message_to_send.clone();
        let owned_topic = expected_topic.to_string();
        let mut client_operations = MockMqttClientOperations::new();
        if expected_error_code.is_some() && mqtt_client_error.is_some()
            || expected_error_code.is_none() && mqtt_client_error.is_none()
        {
            client_operations
                .expect_publish()
                .once()
                .withf(move |msg| {
                    // [utest->dsn~up-transport-mqtt5-e2e-topic-names~1]
                    assert_eq!(
                        msg.topic(),
                        owned_topic,
                        "MQTT message has unexpected topic"
                    );
                    let mapper = crate::mapping::DefaultMessageMapper;
                    // now check if we can recreate the original message from the MQTT message
                    let attributes = mapper
                        .create_uattributes_from_mqtt_properties(msg.properties())
                        .ok();
                    // [utest->dsn~up-transport-mqtt5-payload-mapping~1]
                    let payload = if msg.payload().is_empty() {
                        None
                    } else {
                        Some(Bytes::copy_from_slice(msg.payload()))
                    };
                    let umessage = UMessage {
                        attributes: attributes.into(),
                        payload,
                        ..Default::default()
                    };
                    assert_eq!(umessage, sent_message);
                    true
                })
                .returning(move |_msg| {
                    mqtt_client_error.map_or(Ok(()), |code| {
                        Err(UStatus::fail_with_code(code, "failed to send message"))
                    })
                });
        }
        let client = Mqtt5Transport {
            mqtt_client: Arc::new(client_operations),
            registered_listeners: Arc::new(RwLock::new(RegisteredListeners::default())),
            message_mapper: Arc::new(crate::mapping::DefaultMessageMapper),
            authority_name: "vin.vehicles".to_string(),
            mode: TransportMode::InVehicle,
            message_callback_handle: None,
        };

        let send_result = client.send(message_to_send).await;

        if let Some(error_code) = expected_error_code {
            assert!(send_result.is_err_and(|err| err.get_code() == error_code));
        } else {
            assert!(send_result.is_ok());
        }
    }

    // [utest->dsn~utransport-registerlistener-error-unimplemented~1]
    #[test_case(
        "//vin.vehicles/A8000/2/8A50",
        None,
        "vin.vehicles/8000/A/2/8A50".to_string(),
        None,
        None;
        "succeeds for source filter"
    )]
    #[test_case(
        "//vin.vehicles/A8000/2/8A50",
        None,
        "vin.vehicles/8000/A/2/8A50".to_string(),
        Some(UCode::UNAVAILABLE),
        Some(UCode::UNAVAILABLE);
        "fails if not connected to broker"
    )]
    #[test_case(
        "//vin.vehicles/FFFF8000/2/8A50",
        Some("//vin.vehicles/B8000/3/0"),
        "vin.vehicles/8000/+/2/8A50/vin.vehicles/8000/B/3/0".to_string(),
        None,
        None;
        "succeeds for source and sink filter"
    )]
    #[tokio::test]
    // [utest->dsn~utransport-supported-message-delivery~1]
    async fn test_register_listener(
        source_filter: &str,
        sink_filter: Option<&str>,
        expected_topic_filter: String,
        mqtt_client_error: Option<UCode>,
        expected_error_code: Option<UCode>,
    ) {
        let mut client_operations = MockMqttClientOperations::new();
        client_operations
            .expect_subscribe()
            .once()
            // [utest->dsn~up-transport-mqtt5-e2e-topic-names~1]
            .with(eq(expected_topic_filter.clone()), always())
            .returning(move |_topic_filter, _subscription_id| {
                mqtt_client_error.map_or(Ok(()), |code| {
                    Err(UStatus::fail_with_code(code, "failed to send message"))
                })
            });

        let registered_listeners = Arc::new(RwLock::new(RegisteredListeners::default()));
        let client = Mqtt5Transport {
            mqtt_client: Arc::new(client_operations),
            registered_listeners: registered_listeners.clone(),
            message_mapper: Arc::new(crate::mapping::DefaultMessageMapper),
            authority_name: "vin.vehicles".to_string(),
            mode: TransportMode::InVehicle,
            message_callback_handle: None,
        };

        let listener = Arc::new(MockUListener::new());
        let source_uri = UUri::from_str(source_filter).expect("Expected a valid source value");
        let sink_uri = sink_filter.map(|s| UUri::from_str(s).expect("Expected a valid sink value"));

        let register_result = client
            .register_listener(&source_uri, sink_uri.as_ref(), listener.clone())
            .await;

        let listeners_for_expected_topic = registered_listeners
            .read()
            .await
            .determine_listeners_for_topic(&expected_topic_filter);

        if let Some(error_code) = expected_error_code {
            assert!(register_result.is_err_and(|err| err.get_code() == error_code));
            assert!(listeners_for_expected_topic.is_empty());
        } else {
            assert!(
                register_result.is_ok(),
                "registration failed: {register_result:?}"
            );
            assert!(listeners_for_expected_topic.contains(&ComparableListener::new(listener)));
        }
    }

    #[test_case(
        "//vin.vehicles/FFFF8000/2/0",
        None;
        "for empty sink filter with source resource ID 0"
    )]
    #[test_case(
        "//vin.vehicles/FFFFFFFF/2/7FFF",
        None;
        "for empty sink filter with RPC method source resource ID"
    )]
    #[test_case(
        "//vin.vehicles/FFFFA000/2/0",
        Some("//vin.vehicles/FFFFFFFF/3/0");
        "for source and sink filter having resource ID 0"
    )]
    #[test_case(
        "//vin.vehicles/A000/2/5555",
        Some("//vin.vehicles/B8000/3/1A00");
        "for RPC sink and source having RPC method resource ID"
    )]
    #[test_case(
        "//vin.vehicles/FFFFFFFF/2/B100",
        Some("//vin.vehicles/B8000/3/1A00");
        "for RPC sink and source having event resource ID"
    )]
    #[tokio::test]
    async fn test_un_register_listener_fails_with_invalid_argument(
        source_filter: &str,
        sink_filter: Option<&str>,
    ) {
        let mut client_operations = MockMqttClientOperations::new();
        client_operations.expect_subscribe().never();

        let client = Mqtt5Transport {
            mqtt_client: Arc::new(client_operations),
            registered_listeners: Arc::new(RwLock::new(RegisteredListeners::default())),
            message_mapper: Arc::new(crate::mapping::DefaultMessageMapper),
            authority_name: "vin.vehicles".to_string(),
            mode: TransportMode::InVehicle,
            message_callback_handle: None,
        };

        // [utest->dsn~utransport-registerlistener-error-invalid-parameter~1]
        assert!(client
            .register_listener(
                &UUri::from_str(source_filter).expect("Expected a valid source value"),
                sink_filter
                    .map(|s| UUri::from_str(s).expect("Expected a valid sink value"))
                    .as_ref(),
                Arc::new(MockUListener::new())
            )
            .await
            .is_err_and(|err| err.get_code() == UCode::INVALID_ARGUMENT));

        // [utest->dsn~utransport-unregisterlistener-error-invalid-parameter~1]
        assert!(client
            .unregister_listener(
                &UUri::from_str(source_filter).expect("Expected a valid source value"),
                sink_filter
                    .map(|s| UUri::from_str(s).expect("Expected a valid sink value"))
                    .as_ref(),
                Arc::new(MockUListener::new())
            )
            .await
            .is_err_and(|err| err.get_code() == UCode::INVALID_ARGUMENT));
    }

    // [utest->dsn~utransport-unregisterlistener-error-unimplemented~1]
    #[test_case(
        "//vin.vehicles/A8000/2/8A50",
        None,
        "vin.vehicles/8000/A/2/8A50".to_string(),
        None;
        "succeeds for source filter"
    )]
    #[test_case(
        "//vin.vehicles/A8000/2/8A50",
        None,
        "vin.vehicles/8000/A/2/8A50".to_string(),
        Some(UCode::UNAVAILABLE);
        "fails if not connected to broker"
    )]
    #[test_case(
        "//vin.vehicles/FFFF8000/2/8A50",
        Some("//vin.vehicles/B8000/3/0"),
        "vin.vehicles/8000/+/2/8A50/vin.vehicles/8000/B/3/0".to_string(),
        None;
        "succeeds for source and sink filter"
    )]
    #[tokio::test]
    // [utest->dsn~utransport-supported-message-delivery~1]
    async fn test_unregister_listener(
        source_filter: &str,
        sink_filter: Option<&str>,
        expected_topic_filter: String,
        expected_error_code: Option<UCode>,
    ) {
        let mut client_operations = MockMqttClientOperations::new();
        client_operations
            .expect_unsubscribe()
            .once()
            // [utest->dsn~up-transport-mqtt5-e2e-topic-names~1]
            .with(eq(expected_topic_filter.clone()))
            .returning(move |_topic_filter| {
                expected_error_code.map_or(Ok(()), |code| {
                    Err(UStatus::fail_with_code(code, "failed to send message"))
                })
            });

        let listener = Arc::new(MockUListener::new());
        let registered_listeners = Arc::new(RwLock::new(RegisteredListeners::default()));
        assert!(registered_listeners
            .write()
            .await
            .add_listener(&expected_topic_filter, listener.clone())
            .is_ok());

        let client = Mqtt5Transport {
            mqtt_client: Arc::new(client_operations),
            registered_listeners: registered_listeners.clone(),
            message_mapper: Arc::new(crate::mapping::DefaultMessageMapper),
            authority_name: "vin.vehicles".to_string(),
            mode: TransportMode::InVehicle,
            message_callback_handle: None,
        };

        let source_uri = UUri::from_str(source_filter).expect("Expected a valid source value");
        let sink_uri = sink_filter.map(|s| UUri::from_str(s).expect("Expected a valid sink value"));

        let unregister_result = client
            .unregister_listener(&source_uri, sink_uri.as_ref(), listener.clone())
            .await;

        if let Some(error_code) = expected_error_code {
            assert!(unregister_result.is_err_and(|err| err.get_code() == error_code));
            let listeners_for_topic_filter = registered_listeners
                .read()
                .await
                .determine_listeners_for_topic(&expected_topic_filter);
            assert!(
                listeners_for_topic_filter.contains(&ComparableListener::new(listener)),
                "listener should have not been removed if unsubscribe operation fails"
            );
        } else {
            assert!(unregister_result.is_ok());
            {
                let listeners_for_topic_filter = registered_listeners
                    .read()
                    .await
                    .determine_listeners_for_topic(&expected_topic_filter);
                assert!(listeners_for_topic_filter.is_empty());
            }
            let empty_result = client
                .unregister_listener(&source_uri, sink_uri.as_ref(), listener.clone())
                .await;
            assert!(empty_result.is_err_and(|err| { err.get_code() == UCode::NOT_FOUND }));
        }
    }

    #[tokio::test]
    async fn test_receive_fails() {
        let client_operations = MockMqttClientOperations::new();

        let client = Mqtt5Transport {
            mqtt_client: Arc::new(client_operations),
            registered_listeners: Arc::new(RwLock::new(RegisteredListeners::default())),
            message_mapper: Arc::new(crate::mapping::DefaultMessageMapper),
            authority_name: "vin.vehicles".to_string(),
            mode: TransportMode::InVehicle,
            message_callback_handle: None,
        };
        // [utest->dsn~utransport-receive-error-unimplemented~1]
        assert!(client
            .receive(
                &"up://some_device/A100/1/9100"
                    .parse()
                    .expect("failed to create source UUri"),
                None
            )
            .await
            .is_err_and(|err| err.get_code() == UCode::UNIMPLEMENTED));
    }
}
