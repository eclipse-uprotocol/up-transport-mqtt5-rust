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
    async fn send(&self, message: UMessage) -> Result<(), UStatus> {
        // validate message
        let attributes = message.attributes.as_ref().ok_or(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "uProtocol message has no attributes",
        ))?;

        // Extract payload from umessage to send
        // [impl->dsn~up-transport-mqtt5-payload-mapping~1]
        let payload = message.payload;

        self.send_message(attributes, payload).await
    }

    async fn register_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let topic = self
            .to_mqtt_topic_string(source_filter, sink_filter)
            .map_err(|e| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, e.to_string()))?;

        self.add_listener(&topic, listener).await
    }

    async fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let topic: String = self
            .to_mqtt_topic_string(source_filter, sink_filter)
            .map_err(|e| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, e.to_string()))?;

        self.remove_listener(&topic, listener).await
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use mockall::predicate::{always, eq};
    use up_rust::{
        ComparableListener, MockUListener, UMessageBuilder, UMessageType, UPayloadFormat, UUID,
    };

    use test_case::test_case;
    use tokio::sync::RwLock;

    use crate::{
        listener_registry::RegisteredListeners, mqtt_client::MockMqttClientOperations,
        TransportMode,
    };

    use super::*;

    // Helper function to construct UMessage object for testing.
    fn create_test_message(
        message_type: UMessageType,
        source: &str,
        sink: Option<&str>,
        payload: String,
    ) -> UMessage {
        let source_uri = UUri::from_str(source).expect("Expected a valid source value");

        match message_type {
            UMessageType::UMESSAGE_TYPE_PUBLISH => UMessageBuilder::publish(source_uri)
                .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
                .unwrap(),
            UMessageType::UMESSAGE_TYPE_REQUEST => {
                let sink_uri =
                    UUri::from_str(sink.expect("Expected a sink value for request message"))
                        .expect("Expected a valid sink value");

                UMessageBuilder::request(sink_uri, source_uri, 3600)
                    .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
                    .unwrap()
            }
            UMessageType::UMESSAGE_TYPE_RESPONSE => {
                let sink_uri =
                    UUri::from_str(sink.expect("Expected a sink value for request message"))
                        .expect("Expected a valid sink value");

                UMessageBuilder::response(sink_uri, UUID::build(), source_uri)
                    .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
                    .unwrap()
            }
            UMessageType::UMESSAGE_TYPE_NOTIFICATION => {
                let sink_uri =
                    UUri::from_str(sink.expect("Expected a sink value for notification message"))
                        .expect("Expected a valid sink value");

                UMessageBuilder::notification(source_uri, sink_uri)
                    .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
                    .unwrap()
            }
            _ => panic!("Invalid message type"),
        }
    }

    #[test_case(
        UMessageType::UMESSAGE_TYPE_PUBLISH,
        "//VIN.vehicles/A8000/2/8A50",
        None,
        "payload",
        "VIN.vehicles/8000/A/2/8A50",
        None;
        "succeeds for Publish message"
    )]
    #[test_case(
        UMessageType::UMESSAGE_TYPE_PUBLISH,
        "//VIN.vehicles/A8000/2/8A50",
        None,
        "payload",
        "VIN.vehicles/8000/A/2/8A50",
        Some(UCode::UNAVAILABLE);
        "fails if not connected to broker"
    )]
    #[test_case(
        UMessageType::UMESSAGE_TYPE_NOTIFICATION,
        "/A8000/2/1A50",
        Some("//VIN.vehicles/B8000/3/0"),
        "payload",
        "VIN.vehicles/8000/A/2/1A50/VIN.vehicles/8000/B/3/0",
        None;
        "succeeds for Notification message"
    )]
    #[test_case(
        UMessageType::UMESSAGE_TYPE_REQUEST,
        "//VIN.vehicles/A8000/2/0",
        Some("//VIN.vehicles/B8000/3/10AB"),
        "payload",
        "VIN.vehicles/8000/A/2/0/VIN.vehicles/8000/B/3/10AB",
        None;
        "succeeds for Request message"
    )]
    #[test_case(
        UMessageType::UMESSAGE_TYPE_RESPONSE,
        "//VIN.vehicles/B8000/3/10AB",
        Some("//VIN.vehicles/A8000/2/0"),
        "payload",
        "VIN.vehicles/8000/B/3/10AB/VIN.vehicles/8000/A/2/0",
        None;
        "succeeds for Response message"
    )]
    #[tokio::test]
    async fn test_send(
        message_type: UMessageType,
        source: &str,
        sink: Option<&str>,
        payload: &str,
        expected_topic: &str,
        expected_error_code: Option<UCode>,
    ) {
        let expected_payload = payload.to_string();
        let owned_topic = expected_topic.to_string();
        let mut client_operations = MockMqttClientOperations::new();
        client_operations
            .expect_publish()
            .once()
            .withf(move |msg| {
                // [utest->dsn~up-transport-mqtt5-e2e-topic-names~1]
                msg.topic() == owned_topic &&
                // [utest->dsn~up-transport-mqtt5-payload-mapping~1]
                msg.payload() == expected_payload.as_bytes()
            })
            .returning(move |_msg| {
                expected_error_code.map_or(Ok(()), |code| {
                    Err(UStatus::fail_with_code(code, "failed to send message"))
                })
            });
        let client = Mqtt5Transport {
            mqtt_client: Arc::new(client_operations),
            registered_listeners: Arc::new(RwLock::new(RegisteredListeners::new(10))),
            authority_name: "VIN.vehicles".to_string(),
            mode: TransportMode::InVehicle,
            message_callback_handle: None,
        };

        let message_to_send = create_test_message(message_type, source, sink, payload.to_string());
        let send_result = client.send(message_to_send).await;

        if let Some(error_code) = expected_error_code {
            assert!(send_result.is_err_and(|err| err.get_code() == error_code));
        } else {
            assert!(send_result.is_ok());
        }
    }

    #[test_case(
        "//VIN.vehicles/A8000/2/8A50",
        None,
        "VIN.vehicles/8000/A/2/8A50".to_string(),
        None;
        "succeeds for source filter"
    )]
    #[test_case(
        "//VIN.vehicles/A8000/2/8A50",
        None,
        "VIN.vehicles/8000/A/2/8A50".to_string(),
        Some(UCode::UNAVAILABLE);
        "fails if not connected to broker"
    )]
    #[test_case(
        "//VIN.vehicles/FFFF8000/2/8A50",
        Some("//VIN.vehicles/B8000/3/0"),
        "VIN.vehicles/8000/+/2/8A50/VIN.vehicles/8000/B/3/0".to_string(),
        None;
        "succeeds for source and sink filter"
    )]
    #[tokio::test]
    async fn test_register_listener(
        source_filter: &str,
        sink_filter: Option<&str>,
        expected_topic_filter: String,
        expected_error_code: Option<UCode>,
    ) {
        let mut client_operations = MockMqttClientOperations::new();
        client_operations
            .expect_subscribe()
            .once()
            // [utest->dsn~up-transport-mqtt5-e2e-topic-names~1]
            .with(eq(expected_topic_filter.clone()), always())
            .returning(move |_topic_filter, _subscription_id| {
                expected_error_code.map_or(Ok(()), |code| {
                    Err(UStatus::fail_with_code(code, "failed to send message"))
                })
            });

        let registered_listeners = Arc::new(RwLock::new(RegisteredListeners::new(10)));
        let client = Mqtt5Transport {
            mqtt_client: Arc::new(client_operations),
            registered_listeners: registered_listeners.clone(),
            authority_name: "VIN.vehicles".to_string(),
            mode: TransportMode::InVehicle,
            message_callback_handle: None,
        };

        let listener = Arc::new(MockUListener::new());
        let source_uri = UUri::from_str(source_filter).expect("Expected a valid source value");
        let sink_uri = sink_filter.map(|s| UUri::from_str(s).expect("Expected a valid sink value"));

        let send_result = client
            .register_listener(&source_uri, sink_uri.as_ref(), listener.clone())
            .await;

        let listeners_for_expected_topic = registered_listeners
            .read()
            .await
            .determine_listeners_for_topic(&expected_topic_filter);

        if let Some(error_code) = expected_error_code {
            assert!(send_result.is_err_and(|err| err.get_code() == error_code));
            assert!(listeners_for_expected_topic.is_empty());
        } else {
            assert!(send_result.is_ok());
            assert!(listeners_for_expected_topic.contains(&ComparableListener::new(listener)));
        }
    }

    #[test_case(
        "//VIN.vehicles/A8000/2/8A50",
        None,
        "VIN.vehicles/8000/A/2/8A50".to_string(),
        None;
        "succeeds for source filter"
    )]
    #[test_case(
        "//VIN.vehicles/A8000/2/8A50",
        None,
        "VIN.vehicles/8000/A/2/8A50".to_string(),
        Some(UCode::UNAVAILABLE);
        "fails if not connected to broker"
    )]
    #[test_case(
        "//VIN.vehicles/FFFF8000/2/8A50",
        Some("//VIN.vehicles/B8000/3/0"),
        "VIN.vehicles/8000/+/2/8A50/VIN.vehicles/8000/B/3/0".to_string(),
        None;
        "succeeds for source and sink filter"
    )]
    #[tokio::test]
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
        let registered_listeners = Arc::new(RwLock::new(RegisteredListeners::new(10)));
        assert!(registered_listeners
            .write()
            .await
            .add_listener(&expected_topic_filter, listener.clone())
            .is_ok());

        let client = Mqtt5Transport {
            mqtt_client: Arc::new(client_operations),
            registered_listeners: registered_listeners.clone(),
            authority_name: "VIN.vehicles".to_string(),
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
}
