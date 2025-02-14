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

use async_channel::Receiver;
use bytes::Bytes;
use futures::stream::StreamExt;
use listener_registry::{RegisteredListeners, SubscriptionIdentifier};
use log::{debug, trace};
use mqtt_client::MqttClientOperations;
pub use mqtt_client::{MqttClientOptions, SslOptions};
use paho_mqtt::{self as mqtt, Message, QOS_1};
use tokio::{sync::RwLock, task::JoinHandle};
use up_rust::{UAttributes, UCode, UMessage, UStatus, UUri, UUriError};

mod listener_registry;
mod mapping;
mod mqtt_client;
mod transport;

const MQTT_TOPIC_ANY_SEGMENT_WILDCARD: &str = "+";

/// The transport's mode of operation.
pub enum TransportMode {
    /// Indicates communication via an in-vehicle MQTT broker. This is used by uEntities within the same vehicle
    /// (uEntity-2-uEntity).
    InVehicle,
    /// Indicates communication via an off-vehicle MQTT broker. This is used by uProtocol streamers to connect a
    /// vehicle's uEntities to uEntities running on a (cloud based) back end (Device-2-Device).
    OffVehicle,
}

impl TransportMode {
    /// Creates an MQTT topic segment from the authority name of a uProtocol URI.
    // [impl->dsn~up-transport-mqtt5-d2d-topic-names~1]
    fn uri_to_authority_topic_segment(uri: &UUri, fallback_authority: &str) -> String {
        if uri.has_empty_authority() {
            fallback_authority.to_owned()
        } else if uri.has_wildcard_authority() {
            MQTT_TOPIC_ANY_SEGMENT_WILDCARD.to_string()
        } else {
            uri.authority_name()
        }
    }

    /// Converts a uProtocol URI to an MQTT topic.
    ///
    /// # Arguments
    ///
    /// * `fallback_authority` - The authority name to use if the given URI does not contain an authority.
    /// * `uri` - The URI to convert.
    // [impl->dsn~up-transport-mqtt5-e2e-topic-names~1]
    fn uri_to_e2e_mqtt_topic(uri: &UUri, fallback_authority: &str) -> String {
        let authority = Self::uri_to_authority_topic_segment(uri, fallback_authority);

        let ue_type_id = if uri.has_wildcard_entity_type() {
            MQTT_TOPIC_ANY_SEGMENT_WILDCARD.into()
        } else {
            format!("{:X}", uri.uentity_type_id())
        };

        let ue_instance_id = if uri.has_wildcard_entity_instance() {
            MQTT_TOPIC_ANY_SEGMENT_WILDCARD.into()
        } else {
            format!("{:X}", uri.uentity_instance_id())
        };

        let ue_ver = if uri.has_wildcard_version() {
            MQTT_TOPIC_ANY_SEGMENT_WILDCARD.into()
        } else {
            format!("{:X}", uri.uentity_major_version())
        };

        let res_id = if uri.has_wildcard_resource_id() {
            MQTT_TOPIC_ANY_SEGMENT_WILDCARD.into()
        } else {
            format!("{:X}", uri.resource_id())
        };

        format!("{authority}/{ue_type_id}/{ue_instance_id}/{ue_ver}/{res_id}")
    }

    /// Creates an MQTT topic for a source and sink uProtocol URI.
    ///
    /// # Arguments
    /// * `source` - Source URI.
    /// * `sink` - Sink URI.
    /// * `fallback_authority` - The authority name to use if any of the URIs do not contain an authority.
    pub(crate) fn to_mqtt_topic(
        &self,
        source: &UUri,
        sink: Option<&UUri>,
        fallback_authority: &str,
    ) -> Result<String, UUriError> {
        match self {
            // [impl->dsn~up-transport-mqtt5-e2e-topic-names~1]
            TransportMode::InVehicle => {
                let mut topic = String::new();
                topic.push_str(&Self::uri_to_e2e_mqtt_topic(source, fallback_authority));
                if let Some(uri) = sink {
                    topic.push('/');
                    topic.push_str(&Self::uri_to_e2e_mqtt_topic(uri, fallback_authority));
                }
                Ok(topic)
            }
            // [impl->dsn~up-transport-mqtt5-d2d-topic-names~1]
            TransportMode::OffVehicle => {
                if let Some(uri) = sink {
                    let mut topic = String::new();
                    topic.push_str(&Self::uri_to_authority_topic_segment(
                        source,
                        fallback_authority,
                    ));
                    topic.push('/');
                    topic.push_str(&Self::uri_to_authority_topic_segment(
                        uri,
                        fallback_authority,
                    ));
                    Ok(topic)
                } else {
                    Err(UUriError::serialization_error(
                        "Off-Vehicle transport requires sink URI for creating MQTT topic",
                    ))
                }
            }
        }
    }
}

/// An MQTT 5 based uProtocol transport implementation.
pub struct Mqtt5Transport {
    /// Client instance for connecting to mqtt broker.
    mqtt_client: Arc<dyn MqttClientOperations>,
    registered_listeners: Arc<RwLock<RegisteredListeners>>,
    /// My authority
    authority_name: String,
    /// The transport's mode of operation.
    mode: TransportMode,
    /// Handle to the message callback.
    message_callback_handle: Option<JoinHandle<()>>,
}

impl Mqtt5Transport {
    /// Creates a new transport.
    ///
    /// The connection to the MQTT broker needs to be established by means of the
    /// [`Self::connect`] function. This allows for clients to implement any particular
    /// connection strategy using e.g. an exponential backoff for subsequent connection
    /// attempts.
    ///
    /// # Arguments
    /// * `mode` - The transport's mode of operation.
    /// * `options` - Configuration options for connecting to the MQTT broker.
    /// * `authority_name` - Authority name of the local uEntity.
    pub async fn new(
        mode: TransportMode,
        options: MqttClientOptions,
        authority_name: String,
    ) -> Result<Self, UStatus> {
        let registered_listeners = Arc::new(RwLock::new(RegisteredListeners::new(
            options.max_subscriptions,
        )));

        // Create the MQTT client
        let mut client_operations =
            mqtt_client::PahoBasedMqttClientOperations::new_client(options)?;
        let inbound_message_stream = client_operations.get_message_stream()?;
        let mqtt_client = Arc::new(client_operations);

        // Create the callback for processing messages received from the broker
        let message_callback_handle = Some(Self::create_cb_message_handler(
            registered_listeners.clone(),
            inbound_message_stream,
        ));

        Ok(Self {
            mqtt_client,
            registered_listeners,
            authority_name,
            mode,
            message_callback_handle,
        })
    }

    /// Establishes the initial connection to the MQTT broker.
    ///
    /// In case the connection is lost, the transport will try to reestablish the connection
    /// automatically. The current connection status can be determined by means of
    /// [`Self::is_connected`].
    ///
    /// # Errors
    ///
    /// Returns an error if the connection cannot be established within the
    /// default timeout period.
    pub async fn connect(&self) -> Result<(), UStatus> {
        self.mqtt_client.connect().await
    }

    /// Checks if the transport is currently connected to the MQTT broker.
    pub fn is_connected(&self) -> bool {
        self.mqtt_client.is_connected()
    }

    /// Stops processing of incoming messages.
    pub fn shutdown(&self) {
        if let Some(cb_message_handle) = self.message_callback_handle.as_ref() {
            cb_message_handle.abort();
        }
        self.mqtt_client.disconnect()
        // TODO: clean up subscription state
    }

    /// Creates a callback message handler that listens for incoming messages and notifies listeners asynchronously.
    ///
    /// # Arguments
    /// * `subscription_topics` - Map of subscription identifiers to subscribed topic filters.
    /// * `topic_listeners` - Map of topic filters to listeners.
    /// * `message_stream` - Stream of incoming MQTT PUBLISH packets.
    fn create_cb_message_handler(
        registered_listeners: Arc<RwLock<RegisteredListeners>>,
        mut message_stream: Receiver<Option<Message>>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(msg_opt) = message_stream.next().await {
                let Some(msg) = msg_opt else {
                    //TODO: None means that the connection is dropped. This should be handled correctly.
                    trace!("Received empty message from stream.");
                    continue;
                };

                // extract uProtocol message from MQTT PUBLISH packet
                let umessage =
                    match mapping::create_uattributes_from_mqtt_properties(msg.properties()) {
                        Ok(uattributes) => UMessage {
                            attributes: Some(uattributes).into(),
                            payload: Some(Bytes::copy_from_slice(msg.payload())),
                            ..Default::default()
                        },
                        Err(e) => {
                            debug!(
                                "Failed to map MQTT PUBLISH packet to uProtocol message: {}",
                                e
                            );
                            continue;
                        }
                    };

                let subscription_ids: Vec<SubscriptionIdentifier> = msg
                    .properties()
                    .iter(paho_mqtt::PropertyCode::SubscriptionIdentifier)
                    .filter_map(|property| property.get_u16())
                    .collect();

                let listeners_to_invoke = {
                    let registered_listeners_read = registered_listeners.read().await;
                    if subscription_ids.is_empty() {
                        registered_listeners_read.determine_listeners_for_topic(msg.topic())
                    } else {
                        registered_listeners_read
                            .determine_listeners_for_subscription_ids(subscription_ids.as_slice())
                    }
                };

                for listener in listeners_to_invoke {
                    let msg = umessage.clone();
                    tokio::spawn(async move {
                        listener.on_receive(msg).await;
                    });
                }
            }
        })
    }

    /// Publishes a uProtocol message to an MQTT topic.
    ///
    /// Note that the ter _publish_ used here does not refer to the type
    /// of uProtocol message being sent.
    ///
    /// This function creates an MQTT PUBLISH packet from the given metadata,
    /// payload and topic name and transfers it to the MQTT broker.
    ///  
    /// # Arguments
    /// * `attributes` - The uProtocol message's metadata.
    /// * `payload` - The uProtocol message's payload.
    ///
    /// # Errors
    ///
    /// Returns an error if the given attributes are invalid or the
    /// message cannot be sent to the MQTT broker.
    async fn send_message(
        &self,
        attributes: &UAttributes,
        payload: Option<Bytes>,
    ) -> Result<(), UStatus> {
        // put metadata into MQTT 5 message properties
        let props = mapping::create_mqtt_properties_from_uattributes(attributes)?;

        // Get mqtt topic string from source and sink uuris
        let src_uri = attributes.source.as_ref().ok_or(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "uProtocol Message has no source URI",
        ))?;
        // [impl->dsn~up-transport-mqtt5-e2e-topic-names~1]
        // [impl->dsn~up-transport-mqtt5-d2d-topic-names~1]
        let topic = self
            .to_mqtt_topic_string(src_uri, attributes.sink.as_ref())
            .map_err(|e| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, e.to_string()))?;

        let mut msg_builder = mqtt::MessageBuilder::new()
            .topic(topic.clone())
            .properties(props)
            // The uProtocol spec does not mandate a particular MQTT QoS to use.
            // QoS 1 makes sure that the message is transferred to the MQTT broker
            // at least once.
            .qos(QOS_1);

        if let Some(data) = payload {
            // If there is payload to send, add it to the message unaltered.
            // [impl->dsn~up-transport-mqtt5-payload-mapping~1]
            msg_builder = msg_builder.payload(data);
        }
        let msg = msg_builder.finalize();

        self.mqtt_client
            .publish(msg)
            .await
            .inspect(|_| {
                debug!(
                    "Successfully sent uProtocol message [MQTT topic: {}]",
                    topic
                )
            })
            .inspect_err(|e| {
                debug!("Failed to send uProtocol message [MQTT topic: {topic}]: {e}");
            })
    }

    /// Adds a listener for an MQTT topic filter.
    ///
    /// # Arguments
    /// * `topic_filter` - The topic filter to add the listener for.
    /// * `listener` - The callback to invoke for each incoming message that matches the filter.
    async fn add_listener(
        &self,
        topic_filter: &str,
        listener: Arc<dyn up_rust::UListener>,
    ) -> Result<(), UStatus> {
        let mut registered_listeners_write = self.registered_listeners.write().await;
        if let Some(subscription_id) =
            registered_listeners_write.add_listener(topic_filter, listener)?
        {
            // Subscribe to topic.
            if let Err(sub_err) = self
                .mqtt_client
                .subscribe(topic_filter, subscription_id)
                .await
            {
                debug!("Failed to create new subscription for listener");
                // If subscribe fails, add subscription id back to free subscription ids.
                registered_listeners_write.release_subscription_id(subscription_id, topic_filter);
                return Err(sub_err);
            } else {
                debug!(
                    "Created new subscription [topic filter: {}, id: {}] for listener",
                    topic_filter, subscription_id
                );
            };
        }
        Ok(())
    }

    /// Removes a listener for an MQTT topic filter.
    ///
    /// # Arguments
    /// * `topic_filter` - The topic filter to remove the listener for.
    /// * `listener` - Listener to remove from the topic subscription list.
    async fn remove_listener(
        &self,
        topic_filter: &str,
        listener: Arc<dyn up_rust::UListener>,
    ) -> Result<(), UStatus> {
        let mut registered_listeners_write = self.registered_listeners.write().await;
        if registered_listeners_write.is_last_listener(topic_filter, listener.clone()) {
            // we are about to remove the last listener for the topic filter,
            // so we no longer want messages from the broker matching the filter
            if let Err(e) = self.mqtt_client.unsubscribe(topic_filter).await {
                debug!("Failed to unsubscribe from topic filter [{topic_filter}]");
                return Err(e);
            }
        }

        if registered_listeners_write.remove_listener(topic_filter, listener) {
            Ok(())
        } else {
            Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("No such listener registered for topic filter [{topic_filter}]"),
            ))
        }
    }

    /// Creates an MQTT topic for a source and sink uProtocol URI.
    ///
    /// # Arguments
    /// * `src_uri` - Source URI.
    /// * `sink_uri` - Sink URI.
    fn to_mqtt_topic_string(
        &self,
        src_uri: &UUri,
        sink_uri: Option<&UUri>,
    ) -> Result<String, UUriError> {
        self.mode
            .to_mqtt_topic(src_uri, sink_uri, &self.authority_name)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use mqtt_client::MockMqttClientOperations;
    use up_rust::MockUListener;

    use test_case::test_case;

    use super::*;

    #[tokio::test]
    async fn test_add_listener_subscribes_to_topic_filter() {
        let topic_filter = "+/local_authority";
        let listener = Arc::new(MockUListener::new());
        let expected_topic_filter = topic_filter.to_string();
        let mut client_operations = MockMqttClientOperations::new();
        client_operations.expect_subscribe().once().return_once(
            move |topic_filter, _subscription_id| {
                assert_eq!(topic_filter, expected_topic_filter);
                Ok(())
            },
        );

        let up_client = Mqtt5Transport {
            mqtt_client: Arc::new(client_operations),
            registered_listeners: Arc::new(RwLock::new(RegisteredListeners::new(10))),
            authority_name: "test".to_string(),
            mode: TransportMode::InVehicle,
            message_callback_handle: None,
        };

        assert!(up_client
            .add_listener(topic_filter, listener.clone())
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_remove_listener_unsubscribes_topic_filter() {
        let topic_filter = "+/local_authority";
        let expected_topic_filter = topic_filter.to_string();
        let mut registered_listeners = RegisteredListeners::new(10);
        let listener = Arc::new(MockUListener::new());

        assert!(registered_listeners
            .add_listener(topic_filter, listener.clone())
            .expect("Failed to add listener")
            .is_some());

        let mut client_operations = MockMqttClientOperations::new();
        client_operations
            .expect_unsubscribe()
            .return_once(move |topic_filter| {
                assert_eq!(topic_filter, expected_topic_filter);
                Ok(())
            });

        let up_client = Mqtt5Transport {
            mqtt_client: Arc::new(client_operations),
            registered_listeners: Arc::new(RwLock::new(registered_listeners)),
            authority_name: "test".to_string(),
            mode: TransportMode::InVehicle,
            message_callback_handle: None,
        };

        assert!(up_client
            .remove_listener(topic_filter, listener.clone())
            .await
            .is_ok());

        assert!(up_client
            .remove_listener(topic_filter, listener.clone())
            .await
            .is_err_and(|err| err.get_code() == UCode::NOT_FOUND));
    }

    #[test_case(
        "up://VIN.vehicles/A8000/2/8A50",
        "VIN.vehicles";
        "Valid UUri"
    )]
    #[test_case(
        "A8000/2/8A50",
        "local_authority";
        "Local UUri"
    )]
    #[test_case(
        "//*/A8000/2/8A50",
        "+";
        "Wildcard authority"
    )]
    // [utest->dsn~up-transport-mqtt5-d2d-topic-names~1]
    fn test_uri_to_authority_topic_segment(uri: &str, expected_segment: &str) {
        let uuri = UUri::from_str(uri).expect("failed to create UUri from URI");
        let actual_segment =
            TransportMode::uri_to_authority_topic_segment(&uuri, "local_authority");
        assert_eq!(&actual_segment, expected_segment);
    }

    #[test_case(
        "up://VIN.vehicles/A8000/2/8A50",
        "VIN.vehicles/8000/A/2/8A50";
        "Valid UUri"
    )]
    #[test_case(
        "A8000/2/8A50",
        "local_authority/8000/A/2/8A50";
        "Local UUri"
    )]
    #[test_case(
        "//*/A8000/2/8A50",
        "+/8000/A/2/8A50";
        "Wildcard authority"
    )]
    #[test_case(
        "//VIN.vehicles/FFFF/2/8A50",
        "VIN.vehicles/+/0/2/8A50";
        "Wildcard entity type id"
    )]
    #[test_case(
        "//VIN.vehicles/FFFF8000/2/8A50",
        "VIN.vehicles/8000/+/2/8A50";
        "Wildcard entity instance id"
    )]
    #[test_case(
        "//VIN.vehicles/A8000/FF/8A50",
        "VIN.vehicles/8000/A/+/8A50";
        "Wildcard entity version"
    )]
    #[test_case(
        "//VIN.vehicles/A8000/2/FFFF",
        "VIN.vehicles/8000/A/2/+";
        "Wildcard resource id"
    )]
    // [utest->dsn~up-transport-mqtt5-e2e-topic-names~1]
    fn test_uri_to_e2e_mqtt_topic(uuri: &str, expected_topic: &str) {
        let uuri = UUri::from_str(uuri).expect("failed to create UUri from URI");

        let actual_segment = TransportMode::uri_to_e2e_mqtt_topic(&uuri, "local_authority");
        assert_eq!(&actual_segment, expected_topic);
    }

    #[test_case(
        "//VIN.vehicles/A8000/2/8A50",
        None,
        TransportMode::InVehicle,
        "VIN.vehicles/8000/A/2/8A50";
        "Publish to a specific topic"
    )]
    #[test_case(
        "//VIN.vehicles/A8000/2/8A50",
        Some("//VIN.vehicles/B8000/3/0"),
        TransportMode::InVehicle,
        "VIN.vehicles/8000/A/2/8A50/VIN.vehicles/8000/B/3/0";
        "Send a notification"
    )]
    #[test_case(
        "/A8000/2/0",
        Some("/B8000/3/1B50"),
        TransportMode::InVehicle,
        "local_authority/8000/A/2/0/local_authority/8000/B/3/1B50";
        "Send a local RPC request"
    )]
    #[test_case(
        "//VIN.vehicles/B8000/3/1B50",
        Some("//VIN.vehicles/A8000/2/0"),
        TransportMode::InVehicle,
        "VIN.vehicles/8000/B/3/1B50/VIN.vehicles/8000/A/2/0";
        "Send an RPC Response"
    )]
    #[test_case(
        "//*/FFFFFFFF/FF/FFFF",
        Some("/AB34/1/12CD"),
        TransportMode::InVehicle,
        "+/+/+/+/+/local_authority/AB34/0/1/12CD";
        "Subscribe to incoming RPC requests for a specific method"
    )]
    #[test_case(
        "//*/FFFFFFFF/FF/FFFF",
        Some("//SERVICE.backend/FFFFFFFF/FF/FFFF"),
        TransportMode::OffVehicle,
        "+/SERVICE.backend";
        "Subscribe to all incoming messages for uEntities on a given authority in the back end"
    )]
    #[test_case(
        "//other_authority/FFFFFFFF/FF/FFFF",
        None,
        TransportMode::InVehicle,
        "other_authority/+/+/+/+";
        "Subscribe to all messages published to topics of a specific authority"
    )]
    #[test_case(
        "//*/FFFFFFFF/FF/FFFF",
        Some("/FFFFFFFF/FF/FFFF"),
        TransportMode::OffVehicle,
        "+/local_authority";
        "Streamer subscribes to all inbound messages from the cloud"
    )]
    #[test_case(
        "//*/FFFFFFFF/FF/FFFF",
        None,
        TransportMode::InVehicle,
        "+/+/+/+/+";
        "Subscribe to all publish messages from devices within the vehicle"
    )]
    #[test_case(
        "//other_authority/FFFFFFFF/FF/FFFF",
        Some("//*/FFFFFFFF/FF/FFFF"),
        TransportMode::InVehicle,
        "other_authority/+/+/+/+/+/+/+/+/+";
        "Subscribe to all message types but publish messages sent from a specific authority"
    )]
    // [utest->dsn~up-transport-mqtt5-e2e-topic-names~1]
    // [utest->dsn~up-transport-mqtt5-d2d-topic-names~1]
    fn test_to_mqtt_topic_string(
        src_uri: &str,
        sink_uri: Option<&str>,
        mode: TransportMode,
        expected_topic: &str,
    ) {
        let src_uri = UUri::from_str(src_uri).expect("failed to create source UUri from URI");
        let sink_uri =
            sink_uri.map(|uri| UUri::from_str(uri).expect("failed to create sink UUri from URI"));

        assert!(mode
            .to_mqtt_topic(&src_uri, sink_uri.as_ref(), "local_authority")
            .is_ok_and(|topic| topic == expected_topic));
    }

    #[tokio::test]
    async fn test_connect_invokes_mqtt_client() {
        let mut client_operations = MockMqttClientOperations::new();
        client_operations
            .expect_connect()
            .once()
            .return_const(Ok(()));
        let client = Mqtt5Transport {
            mqtt_client: Arc::new(client_operations),
            registered_listeners: Arc::new(RwLock::new(RegisteredListeners::new(10))),
            authority_name: "VIN.vehicles".to_string(),
            mode: TransportMode::InVehicle,
            message_callback_handle: None,
        };
        assert!(client.connect().await.is_ok());
    }

    #[tokio::test]
    async fn test_shutdown_disconnects_mqtt_client() {
        let mut client_operations = MockMqttClientOperations::new();
        client_operations
            .expect_disconnect()
            .once()
            .return_const(());
        let client = Mqtt5Transport {
            mqtt_client: Arc::new(client_operations),
            registered_listeners: Arc::new(RwLock::new(RegisteredListeners::new(10))),
            authority_name: "VIN.vehicles".to_string(),
            mode: TransportMode::InVehicle,
            message_callback_handle: None,
        };
        client.shutdown();
    }
}
