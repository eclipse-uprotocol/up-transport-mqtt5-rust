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

use crate::UPClientMqtt;

#[async_trait]
impl UTransport for UPClientMqtt {
    async fn send(&self, message: UMessage) -> Result<(), UStatus> {
        // validate message
        let attributes = message.attributes.as_ref().ok_or(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "Unable to parse uAttributes",
        ))?;

        // Get mqtt topic string from source and sink uuris
        let src_uri = attributes.source.as_ref().ok_or(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "Invalid source: expected a source value, none was found",
        ))?;
        let sink_uri = attributes.sink.as_ref();
        let topic = self.to_mqtt_topic_string(src_uri, sink_uri);

        // Extract payload from umessage to send
        let payload = message.payload;

        self.send_message(&topic, attributes, payload).await
    }

    async fn register_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let topic = self.to_mqtt_topic_string(source_filter, sink_filter);

        self.add_listener(&topic, listener).await
    }

    async fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let topic: String = self.to_mqtt_topic_string(source_filter, sink_filter);

        self.remove_listener(&topic, listener).await
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, str::FromStr};

    use up_rust::{
        ComparableListener, MockUListener, UMessageBuilder, UMessageType, UPayloadFormat, UUID,
    };

    use test_case::test_case;
    use tokio::sync::RwLock;

    use crate::{MockMqttClientOperations, UPClientMqttType};

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

                UMessageBuilder::request(source_uri, sink_uri, 3600)
                    .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
                    .unwrap()
            }
            UMessageType::UMESSAGE_TYPE_RESPONSE => {
                let sink_uri =
                    UUri::from_str(sink.expect("Expected a sink value for request message"))
                        .expect("Expected a valid sink value");

                UMessageBuilder::response(source_uri, UUID::build(), sink_uri)
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
        None;
        "Publish success"
    )]
    #[test_case(
        UMessageType::UMESSAGE_TYPE_NOTIFICATION,
        "//VIN.vehicles/A8000/2/1A50",
        Some("//VIN.vehicles/B8000/3/0"),
        "payload",
        None;
        "Notification success"
    )]
    #[test_case(
        UMessageType::UMESSAGE_TYPE_REQUEST,
        "//VIN.vehicles/A8000/2/1B50",
        Some("//VIN.vehicles/B8000/3/0"),
        "payload",
        None;
        "Request success"
    )]
    #[test_case(
        UMessageType::UMESSAGE_TYPE_RESPONSE,
        "//VIN.vehicles/B8000/3/0",
        Some("//VIN.vehicles/A8000/2/1B50"),
        "payload",
        None;
        "Response success"
    )]
    #[tokio::test]
    async fn test_send(
        message_type: UMessageType,
        source: &str,
        sink: Option<&str>,
        payload: &str,
        expected_error_code: Option<UCode>,
    ) {
        let mut client_operations = MockMqttClientOperations::new();
        client_operations
            .expect_publish()
            .once()
            .return_once(move |_msg| {
                expected_error_code.map_or(Ok(()), |code| {
                    Err(UStatus::fail_with_code(code, "failed to send message"))
                })
            });
        let client = UPClientMqtt {
            mqtt_client: Arc::new(client_operations),
            subscription_topic_map: Arc::new(RwLock::new(HashMap::new())),
            topic_listener_map: Arc::new(RwLock::new(HashMap::new())),
            authority_name: "VIN.vehicles".to_string(),
            client_type: UPClientMqttType::Device,
            free_subscription_ids: Arc::new(RwLock::new((1..10).collect())),
            cb_message_handle: None,
        };

        let message = create_test_message(message_type, source, sink, payload.to_string());
        let send_result = client.send(message).await;

        if let Some(error_code) = expected_error_code {
            assert!(send_result.is_err_and(|err| err.get_code() == error_code));
        } else {
            assert!(send_result.is_ok());
        }
    }

    #[test_case(
        "//VIN.vehicles/A8000/2/8A50",
        None,
        "d/VIN.vehicles/A8000/2/8A50",
        None;
        "Register listener success"
    )]
    #[test_case(
        "//VIN.vehicles/A8000/2/8A50",
        Some("//VIN.vehicles/B8000/3/0"),
        "d/VIN.vehicles/A8000/2/8A50/VIN.vehicles/B8000/3/0",
        None;
        "Register listener with sink success"
    )]
    #[tokio::test]
    async fn test_register_listener(
        source_filter: &str,
        sink_filter: Option<&str>,
        expected_topic: &str,
        expected_error_code: Option<UCode>,
    ) {
        let topic_listener_map = Arc::new(RwLock::new(HashMap::new()));
        let mut client_operations = MockMqttClientOperations::new();
        client_operations.expect_subscribe().once().return_once(
            move |_topic_filter, _subscription_id| {
                expected_error_code.map_or(Ok(()), |code| {
                    Err(UStatus::fail_with_code(code, "failed to send message"))
                })
            },
        );

        let client = UPClientMqtt {
            mqtt_client: Arc::new(client_operations),
            subscription_topic_map: Arc::new(RwLock::new(HashMap::new())),
            topic_listener_map,
            authority_name: "VIN.vehicles".to_string(),
            client_type: UPClientMqttType::Device,
            free_subscription_ids: Arc::new(RwLock::new((1..10).collect())),
            cb_message_handle: None,
        };

        let listener = Arc::new(MockUListener::new());

        let source_uri = UUri::from_str(source_filter).expect("Expected a valid source value");

        let sink_uri = sink_filter.map(|s| UUri::from_str(s).expect("Expected a valid sink value"));

        let send_result = client
            .register_listener(&source_uri, sink_uri.as_ref(), listener.clone())
            .await;

        if let Some(error_code) = expected_error_code {
            assert!(send_result.is_err_and(|err| err.get_code() == error_code));
        } else {
            assert!(send_result.is_ok());
        }

        let topic_map = client.topic_listener_map.read().await;

        assert!(topic_map.contains_key(expected_topic));

        let listeners = topic_map.get(expected_topic).unwrap();

        assert!(listeners.contains(&ComparableListener::new(listener)));
    }

    #[test_case(
        "//VIN.vehicles/A8000/2/8A50",
        None,
        "d/VIN.vehicles/A8000/2/8A50",
        None;
        "Unregister listener success"
    )]
    #[test_case(
        "//VIN.vehicles/A8000/2/8A50",
        Some("//VIN.vehicles/B8000/3/0"),
        "d/VIN.vehicles/A8000/2/8A50/VIN.vehicles/B8000/3/0",
        None;
        "Unregister listener with sink success"
    )]
    #[tokio::test]
    async fn test_unregister_listener(
        source_filter: &str,
        sink_filter: Option<&str>,
        expected_topic: &str,
        expected_error_code: Option<UCode>,
    ) {
        let topic_listener_map = Arc::new(RwLock::new(HashMap::new()));
        let mut client_operations = MockMqttClientOperations::new();
        client_operations
            .expect_unsubscribe()
            .once()
            .return_once(move |_topic_filter| {
                expected_error_code.map_or(Ok(()), |code| {
                    Err(UStatus::fail_with_code(code, "failed to send message"))
                })
            });

        let listener = Arc::new(MockUListener::new());
        let comparable_listener = ComparableListener::new(listener.clone());

        topic_listener_map.write().await.insert(
            expected_topic.to_string(),
            [comparable_listener.clone()].iter().cloned().collect(),
        );

        let client = UPClientMqtt {
            mqtt_client: Arc::new(client_operations),
            subscription_topic_map: Arc::new(RwLock::new(HashMap::new())),
            topic_listener_map,
            authority_name: "VIN.vehicles".to_string(),
            client_type: UPClientMqttType::Device,
            free_subscription_ids: Arc::new(RwLock::new((1..10).collect())),
            cb_message_handle: None,
        };

        let source_uri = UUri::from_str(source_filter).expect("Expected a valid source value");

        let sink_uri = sink_filter.map(|s| UUri::from_str(s).expect("Expected a valid sink value"));

        let send_result = client
            .unregister_listener(&source_uri, sink_uri.as_ref(), listener.clone())
            .await;

        if let Some(error_code) = expected_error_code {
            assert!(send_result.is_err_and(|err| err.get_code() == error_code));
        } else {
            assert!(send_result.is_ok());
        }

        {
            let topic_map = client.topic_listener_map.read().await;
            assert!(!topic_map.contains_key(expected_topic));
        }

        let empty_result = client
            .unregister_listener(&source_uri, sink_uri.as_ref(), listener.clone())
            .await;

        assert!(empty_result.is_err_and(|err| { err.get_code() == UCode::NOT_FOUND }));
    }
}
