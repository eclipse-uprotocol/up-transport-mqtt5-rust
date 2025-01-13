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

    use paho_mqtt::{self as mqtt, AsyncReceiver, Message};
    use up_rust::{
        ComparableListener, UListener, UMessageBuilder, UMessageType, UPayloadFormat, UUID,
    };

    use test_case::test_case;
    use tokio::sync::RwLock;

    use crate::{MockableMqttClient, MqttConfig, UPClientMqttType};

    use super::*;

    // Simple listener for testing.
    pub struct SimpleListener {}

    #[async_trait]
    impl UListener for SimpleListener {
        async fn on_receive(&self, message: UMessage) {
            println!("Received message: {:?}", message);
        }
    }

    // Mock Mqtt client for testing.
    pub struct MockMqttClient {}

    #[async_trait]
    impl MockableMqttClient for MockMqttClient {
        async fn new_client(
            _config: MqttConfig,
            _client_id: UUID,
        ) -> Result<(Self, AsyncReceiver<Option<Message>>), UStatus>
        where
            Self: Sized,
        {
            let (_tx, rx) = async_channel::bounded(1);
            Ok((Self {}, rx))
        }

        async fn publish(&self, _mqtt_message: mqtt::Message) -> Result<(), UStatus> {
            Ok(())
        }

        async fn subscribe(&self, _topic: &str, _id: i32) -> Result<(), UStatus> {
            Ok(())
        }

        async fn unsubscribe(&self, _topic: &str) -> Result<(), UStatus> {
            Ok(())
        }
    }

    // Helper function to construct UMessage object for testing.
    fn create_test_message(
        message_type: UMessageType,
        source: &str,
        sink: Option<&str>,
        payload: String,
    ) -> Result<UMessage, UStatus> {
        let source_uri = UUri::from_str(source).expect("Expected a valid source value");

        match message_type {
            UMessageType::UMESSAGE_TYPE_PUBLISH => Ok(UMessageBuilder::publish(source_uri)
                .build_with_payload(payload.clone(), UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
                .unwrap()),
            UMessageType::UMESSAGE_TYPE_REQUEST => {
                let sink_uri =
                    UUri::from_str(sink.expect("Expected a sink value for request message"))
                        .expect("Exoected a valid sink value");

                Ok(UMessageBuilder::request(source_uri, sink_uri, 3600)
                    .build_with_payload(payload.clone(), UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
                    .unwrap())
            }
            UMessageType::UMESSAGE_TYPE_RESPONSE => {
                let sink_uri =
                    UUri::from_str(sink.expect("Expected a sink value for request message"))
                        .expect("Exoected a valid sink value");

                Ok(
                    UMessageBuilder::response(source_uri, UUID::build(), sink_uri)
                        .build_with_payload(payload.clone(), UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
                        .unwrap(),
                )
            }
            UMessageType::UMESSAGE_TYPE_NOTIFICATION => {
                let sink_uri =
                    UUri::from_str(sink.expect("Expected a sink value for notification message"))
                        .expect("Exoected a valid sink value");

                Ok(UMessageBuilder::notification(source_uri, sink_uri)
                    .build_with_payload(payload, UPayloadFormat::UPAYLOAD_FORMAT_TEXT)
                    .unwrap())
            }
            _ => Err(UStatus::fail_with_code(
                UCode::INVALID_ARGUMENT,
                "Invalid message type",
            )),
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
        expected_error: Option<UStatus>,
    ) {
        let client = UPClientMqtt {
            mqtt_client: Arc::new(MockMqttClient {}),
            subscription_topic_map: Arc::new(RwLock::new(HashMap::new())),
            topic_listener_map: Arc::new(RwLock::new(HashMap::new())),
            authority_name: "VIN.vehicles".to_string(),
            client_type: UPClientMqttType::Device,
            free_subscription_ids: Arc::new(RwLock::new((1..10).collect())),
            cb_message_handle: None,
        };

        let message = create_test_message(message_type, source, sink, payload.to_string()).unwrap();

        let result = client.send(message).await;

        if result.is_err() {
            assert_eq!(result.err().unwrap(), expected_error.unwrap());
        } else {
            assert!(result.is_ok());
            assert!(expected_error.is_none());
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
        expected_error: Option<UStatus>,
    ) {
        let topic_listener_map = Arc::new(RwLock::new(HashMap::new()));

        let client = UPClientMqtt {
            mqtt_client: Arc::new(MockMqttClient {}),
            subscription_topic_map: Arc::new(RwLock::new(HashMap::new())),
            topic_listener_map,
            authority_name: "VIN.vehicles".to_string(),
            client_type: UPClientMqttType::Device,
            free_subscription_ids: Arc::new(RwLock::new((1..10).collect())),
            cb_message_handle: None,
        };

        let listener = Arc::new(SimpleListener {});

        let source_uri = UUri::from_str(source_filter).expect("Expected a valid source value");

        let sink_uri = sink_filter.map(|s| UUri::from_str(s).expect("Expected a valid sink value"));

        let result = client
            .register_listener(&source_uri, sink_uri.as_ref(), listener.clone())
            .await;

        if result.is_err() {
            assert_eq!(result.err().unwrap(), expected_error.unwrap());
        } else {
            assert!(result.is_ok());
            assert!(expected_error.is_none());
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
        expected_error: Option<UStatus>,
    ) {
        let topic_listener_map = Arc::new(RwLock::new(HashMap::new()));

        let listener = Arc::new(SimpleListener {});
        let comparable_listener = ComparableListener::new(listener.clone());

        topic_listener_map.write().await.insert(
            expected_topic.to_string(),
            [comparable_listener.clone()].iter().cloned().collect(),
        );

        let client = UPClientMqtt {
            mqtt_client: Arc::new(MockMqttClient {}),
            subscription_topic_map: Arc::new(RwLock::new(HashMap::new())),
            topic_listener_map,
            authority_name: "VIN.vehicles".to_string(),
            client_type: UPClientMqttType::Device,
            free_subscription_ids: Arc::new(RwLock::new((1..10).collect())),
            cb_message_handle: None,
        };

        let source_uri = UUri::from_str(source_filter).expect("Expected a valid source value");

        let sink_uri = sink_filter.map(|s| UUri::from_str(s).expect("Expected a valid sink value"));

        let result = client
            .unregister_listener(&source_uri, sink_uri.as_ref(), listener.clone())
            .await;

        if result.is_err() {
            assert_eq!(result.err().unwrap(), expected_error.unwrap());
        } else {
            assert!(result.is_ok());
            assert!(expected_error.is_none());
        }

        {
            let topic_map = client.topic_listener_map.read().await;
            assert!(!topic_map.contains_key(expected_topic));
        }

        let empty_result = client
            .unregister_listener(&source_uri, sink_uri.as_ref(), listener.clone())
            .await;

        assert!(empty_result.is_err());
        assert_eq!(
            empty_result.err().unwrap(),
            UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("Topic '{expected_topic}' is not registered.")
            )
        );
    }
}
