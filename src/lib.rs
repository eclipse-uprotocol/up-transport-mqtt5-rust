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

use std::{
    collections::{HashMap, HashSet},
    ops::Deref,
    str::FromStr,
    sync::Arc,
};

use async_channel::Receiver;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::StreamExt;
use log::{debug, info, trace, warn};
use paho_mqtt::{
    self as mqtt, AsyncReceiver, Message, Properties, SslOptions, MQTT_VERSION_5, QOS_1,
};
use protobuf::MessageDyn;
use tokio::{sync::RwLock, task::JoinHandle};
use up_rust::{
    ComparableListener, UAttributes, UAttributesValidators, UCode, UMessage, UStatus, UUri, UUID,
};

pub mod transport;

// URI Wildcard consts
// TODO: Remove once up-rust contains/exposes these values
const WILDCARD_AUTHORITY: &str = "*";
const WILDCARD_ENTITY_ID: u32 = 0x0000_FFFF;
const WILDCARD_ENTITY_VERSION: u32 = 0x0000_00FF;
const WILDCARD_RESOURCE_ID: u32 = 0x0000_FFFF;

// Attribute field names
const UURI_NAME: &str = "uuri";
const UUID_NAME: &str = "uuid";

/// Trait that allows for a mockable mqtt client.
#[async_trait]
pub trait MockableMqttClient: Sync + Send {
    /// Create a new MockableMqttClient.
    ///
    /// # Arguments
    /// * `config` - Configuration for the mqtt client.
    /// * `client_id` - Client id for the mqtt client.
    async fn new_client(
        config: MqttConfig,
        client_id: UUID,
    ) -> Result<(Self, AsyncReceiver<Option<Message>>), UStatus>
    where
        Self: Sized;

    /// Publish an mqtt message to the mqtt broker.
    ///
    /// # Arguments
    /// * `topic` - Topic to subscribe to.
    async fn publish(&self, mqtt_message: mqtt::Message) -> Result<(), UStatus>;

    /// Subscribe the mqtt client to a topic.
    ///
    /// # Arguments
    /// * `topic` - Topic to subscribe to.
    /// * `id` - Subscription ID for the topic, used to prevent duplication.
    async fn subscribe(&self, topic: &str, id: i32) -> Result<(), UStatus>;

    /// Unsubscribe the mqtt client to a topic.
    ///
    /// # Arguments
    /// * `topic` - Topic to subscribe to.
    async fn unsubscribe(&self, topic: &str) -> Result<(), UStatus>;
}

pub struct AsyncMqttClient {
    inner_mqtt_client: Arc<mqtt::AsyncClient>,
    sub_identifier_available: bool,
}

fn check_subscription_identifier_available_in_response(props: &Properties) -> bool {
    let property = props.get(paho_mqtt::PropertyCode::SubscriptionIdentifiersAvailable);
    if let Some(property) = property {
        let property_value = property.get_byte();
        if let Some(value) = property_value {
            if value != 1 {
                debug!("Subscription Identifier not supported by broker");
                return false;
            }
        }
    }
    debug!("Subscription Identifier supported by broker");
    true
}

// Create a set of poperties with a single Subscription ID
fn sub_id(id: i32) -> mqtt::Properties {
    mqtt::properties![
        mqtt::PropertyCode::SubscriptionIdentifier => id
    ]
}

#[async_trait]
impl MockableMqttClient for AsyncMqttClient {
    /// Create a new MockableMqttClient.
    ///
    /// # Arguments
    /// * `config` - Configuration for the mqtt client.
    /// * `client_id` - Client id for the mqtt client.
    async fn new_client(
        config: MqttConfig,
        client_id: UUID,
    ) -> Result<(Self, AsyncReceiver<Option<Message>>), UStatus>
    where
        Self: Sized,
    {
        let mqtt_protocol = match config.mqtt_protocol {
            MqttProtocol::Mqtt => "mqtt",
            MqttProtocol::Mqtts => "mqtts",
        };

        let mqtt_uri = format!(
            "{}://{}:{}",
            mqtt_protocol, config.mqtt_hostname, config.mqtt_port
        );

        let mut mqtt_cli = mqtt::CreateOptionsBuilder::new()
            .server_uri(mqtt_uri)
            .client_id(client_id)
            .max_buffered_messages(config.max_buffered_messages)
            .create_client()
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Unable to create mqtt client: {e:?}"),
                )
            })?;

        let message_stream = mqtt_cli.get_stream(100);

        let conn_opts =
            mqtt::ConnectOptionsBuilder::with_mqtt_version(MQTT_VERSION_5)
            .clean_start(false)
            .properties(mqtt::properties![mqtt::PropertyCode::SessionExpiryInterval => config.session_expiry_interval])
            .ssl_options(config.ssl_options.or_else(|| Some(SslOptions::default())).unwrap())
            .user_name(config.username)
            .finalize();

        let token = mqtt_cli.connect(conn_opts).await.map_err(|e| {
            UStatus::fail_with_code(
                UCode::INTERNAL,
                format!("Unable to connect to mqtt broker: {e:?}"),
            )
        })?;

        let sub_identifier_available =
            check_subscription_identifier_available_in_response(token.properties());

        Ok((
            Self {
                inner_mqtt_client: Arc::new(mqtt_cli),
                sub_identifier_available,
            },
            message_stream,
        ))
    }

    /// Publish an mqtt message to the mqtt broker.
    ///
    /// # Arguments
    /// * `topic` - Topic to subscribe to.
    async fn publish(&self, mqtt_message: mqtt::Message) -> Result<(), UStatus> {
        self.inner_mqtt_client
            .publish(mqtt_message)
            .await
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Unable to publish message: {e:?}"),
                )
            })?;

        Ok(())
    }

    /// Subscribe the mqtt client to a topic.
    ///
    /// # Arguments
    /// * `topic` - Topic to subscribe to.
    async fn subscribe(&self, topic: &str, id: i32) -> Result<(), UStatus> {
        // QOS 1 - Delivered and received at least once
        let sub_id_prop = if self.sub_identifier_available {
            debug!(
                "Subcription identifier supported by broker. Subscribe with subscription id {}",
                id
            );
            Some(sub_id(id))
        } else {
            None
        };

        self.inner_mqtt_client
            .subscribe_with_options(topic, QOS_1, None, sub_id_prop)
            .await
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Unable to subscribe to topic: {e:?}"),
                )
            })?;

        Ok(())
    }

    /// Unsubscribe the mqtt client to a topic.
    ///
    /// # Arguments
    /// * `topic` - Topic to subscribe to.
    async fn unsubscribe(&self, topic: &str) -> Result<(), UStatus> {
        self.inner_mqtt_client
            .unsubscribe(topic)
            .await
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Unable to unsubscribe from topic: {e:?}"),
                )
            })?;

        Ok(())
    }
}

/// Configuration for the mqtt client.
pub struct MqttConfig {
    /// Schema of the mqtt broker (mqtt or mqtts)
    pub mqtt_protocol: MqttProtocol,
    /// Port of the mqtt broker to connect to.
    pub mqtt_port: u16,
    /// Hostname of the mqtt broker.
    pub mqtt_hostname: String,
    /// Max buffered messages for the mqtt client.
    pub max_buffered_messages: i32,
    /// Max subscriptions for the mqtt client.
    pub max_subscriptions: i32,
    /// Session Expiry Interval for the mqtt client.
    pub session_expiry_interval: i32,
    /// Optional SSL options for the mqtt connection.
    pub ssl_options: Option<mqtt::SslOptions>,
    /// Username
    pub username: String,
}

/// UP Client for mqtt.
pub struct UPClientMqtt {
    /// Client instance for connecting to mqtt broker.
    mqtt_client: Arc<dyn MockableMqttClient>,
    /// Map of subscription identifiers to subscribed topics.
    subscription_topic_map: Arc<RwLock<HashMap<i32, String>>>,
    /// Map of topics to listeners.
    topic_listener_map: Arc<RwLock<HashMap<String, HashSet<ComparableListener>>>>,
    /// My authority
    authority_name: String,
    /// Indicates where client instance is running.
    client_type: UPClientMqttType,
    /// List of free subscription identifiers to use for the client subscriptions.
    free_subscription_ids: Arc<RwLock<HashSet<i32>>>,
    /// Handle to the message callback.
    cb_message_handle: Option<JoinHandle<()>>,
}

/// Type of UPClientMqtt.
pub enum UPClientMqttType {
    Device,
    Cloud,
}

/// Type of MQTT protocol
pub enum MqttProtocol {
    Mqtt,
    Mqtts,
}

impl UPClientMqtt {
    /// Create a new UPClientMqtt.
    ///
    /// # Arguments
    /// * `config` - Configuration for the mqtt client.
    /// * `client_id` - Client id for the mqtt client.
    /// * `authority_name` - Authority name for the mqtt client.
    /// * `client_type` - Type of client instance.
    pub async fn new(
        config: MqttConfig,
        client_id: UUID,
        authority_name: String,
        client_type: UPClientMqttType,
    ) -> Result<Self, UStatus> {
        let subscription_topic_map = Arc::new(RwLock::new(HashMap::new()));
        let topic_listener_map = Arc::new(RwLock::new(HashMap::new()));
        let free_subscription_ids =
            Arc::new(RwLock::new((1..config.max_subscriptions + 1).collect()));

        let subscription_topic_map_handle = subscription_topic_map.clone();
        let topic_listener_map_handle = topic_listener_map.clone();

        // Create the mqtt client instance.
        let (mqtt_client, message_stream) = AsyncMqttClient::new_client(config, client_id).await?;

        // Create the callback message handler.
        let cb_message_handle = Some(Self::create_cb_message_handler(
            subscription_topic_map_handle,
            topic_listener_map_handle,
            message_stream,
        ));

        Ok(Self {
            mqtt_client: Arc::new(mqtt_client),
            subscription_topic_map,
            topic_listener_map,
            authority_name,
            client_type,
            free_subscription_ids,
            cb_message_handle,
        })
    }

    /// On exit, cleanup the callback message thread.
    pub fn cleanup(&self) {
        if let Some(cb_message_handle) = &self.cb_message_handle {
            cb_message_handle.abort();
        }
    }

    // Creates a callback message handler that listens for incoming messages and notifies listeners asyncronously.
    //
    // # Arguments
    // * `subscription_map` - Map of subscription identifiers to subscribed topics.
    // * `topic_map` - Map of topics to listeners.
    // * `message_stream` - Stream of incoming mqtt messages.
    fn create_cb_message_handler(
        subscription_map: Arc<RwLock<HashMap<i32, String>>>,
        topic_map: Arc<RwLock<HashMap<String, HashSet<ComparableListener>>>>,
        mut message_stream: Receiver<Option<Message>>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(msg_opt) = message_stream.next().await {
                let Some(msg) = msg_opt else {
                    trace!("Received empty message from stream.");
                    continue;
                };
                let topic = msg.topic();
                let sub_id = msg
                    .properties()
                    .get_int(mqtt::PropertyCode::SubscriptionIdentifier);

                // Get attributes from mqtt header.
                let umessage = if UPClientMqtt::get_uattributes_from_mqtt_properties(msg.properties()).is_err() {
                    debug!("empty properties");
                    protobuf::Message::parse_from_bytes(msg.payload()).unwrap()
                } else {
                    let uattributes = {
                        match UPClientMqtt::get_uattributes_from_mqtt_properties(msg.properties()) {
                            Ok(uattributes) => uattributes,
                            Err(e) => {
                                warn!("Unable to get UAttributes from mqtt properties: {}", e);
                                continue;
                            }
                        }
                    };
    
                    let payload = msg.payload();
                    let upayload = payload.to_vec();
    
                    // Create UMessage from UAttributes and UPayload.
                    UMessage {
                        attributes: Some(uattributes).into(),
                        payload: Some(upayload.into()),
                        ..Default::default()
                    }
                };
                

                let topic_map_read = topic_map.read().await;
                let subscription_map_read = subscription_map.read().await;

                // If subscription ID is present, only notify listeners for that subscription.
                if let Some(sub_id) = sub_id {
                    let Some(sub_topic) = subscription_map_read.get(&sub_id) else {
                        trace!(
                            "Received message with subscription id that is not registered: {}",
                            sub_id
                        );
                        continue;
                    };

                    let Some(listeners) = topic_map_read.get(sub_topic) else {
                        trace!("No listeners registered for topic: {}", sub_topic);
                        continue;
                    };

                    for listener in listeners.iter() {
                        listener.on_receive(umessage.clone()).await;
                    }
                } else {
                    // Filter the topic map for topics that match the received topic, including wildcards.
                    let topics_iter = topic_map_read
                        .iter()
                        .filter(|(key, _)| UPClientMqtt::compare_topic(topic, key));

                    for (_topic, listeners) in topics_iter {
                        for listener in listeners.iter() {
                            listener.on_receive(umessage.clone()).await;
                        }
                    }
                }
            }
        })
    }

    /// Compare a topic to a topic pattern. Supports single level wildcards.
    ///
    /// # Arguments
    /// * `topic` - Topic to compare.
    /// * `pattern` - Topic pattern to compare against.
    fn compare_topic(topic: &str, pattern: &str) -> bool {
        let topic_parts = topic.split('/');
        let pattern_parts = pattern.split('/');

        for (topic_part, pattern_part) in topic_parts.zip(pattern_parts) {
            if topic_part != pattern_part && pattern_part != "+" {
                return false;
            }
        }

        true
    }

    /// Get an available subscription id to use.
    async fn get_free_subscription_id(&self) -> Result<i32, UStatus> {
        // Get a random subscription id from the free subscription ids.
        let mut free_ids = self.free_subscription_ids.write().await;
        if let Some(&id) = free_ids.iter().next() {
            free_ids.remove(&id);
            Ok(id)
        } else {
            Err(UStatus::fail_with_code(
                UCode::INTERNAL,
                "Max number of subscriptions reached on this client.",
            ))
        }
    }

    /// Add an unused subscription id to the free subscription ids.
    ///
    /// # Arguments
    /// * `id` - Subscription id to add to the free subscription ids.
    async fn add_free_subscription_id(&self, id: i32) {
        let mut free_ids = self.free_subscription_ids.write().await;
        free_ids.insert(id);
    }

    /// Get the client indicator based on client type.
    fn get_client_indicator(&self) -> String {
        match self.client_type {
            UPClientMqttType::Device => "d",
            UPClientMqttType::Cloud => "c",
        }
        .to_string()
    }

    /// Send UMessage to mqtt topic.
    ///
    /// # Arguments
    /// * `topic` - Mqtt topic to send message to.
    /// * `attributes` - UAttributes to send with message.
    /// * `payload` - UMessage payload to send.
    async fn send_message(
        &self,
        topic: &str,
        attributes: &UAttributes,
        payload: Option<Bytes>,
    ) -> Result<(), UStatus> {
        info!("Sending message to topic: {}", topic);
        let props = UPClientMqtt::create_mqtt_properties_from_uattributes(attributes)?;

        let mut msg_builder = mqtt::MessageBuilder::new()
            .topic(topic)
            .properties(props)
            .qos(QOS_1); // QOS 1 - Delivered and received at least once

        // If there is payload to send, add it to the message.
        if let Some(data) = payload {
            msg_builder = msg_builder.payload(data);
        }

        let msg = msg_builder.finalize();

        info!("Sending message: {:?}", msg);

        self.mqtt_client.publish(msg).await.map_err(|e| {
            UStatus::fail_with_code(UCode::INTERNAL, format!("Unable to publish message: {e:?}"))
        })?;

        Ok(())
    }

    /// Add a UListener to an mqtt topic.
    ///
    /// # Arguments
    /// * `topic` - Topic to add the listener to.
    /// * `listener` - Listener to call when the topic recieves a message.
    async fn add_listener(
        &self,
        topic: &str,
        listener: Arc<dyn up_rust::UListener>,
    ) -> Result<(), UStatus> {
        info!("Adding listener to topic: {}", topic);

        let mut topic_listener_map = self.topic_listener_map.write().await;

        if !topic_listener_map.contains_key(topic) {
            let id = self.get_free_subscription_id().await?;
            let mut subscription_topic_map = self.subscription_topic_map.write().await;
            subscription_topic_map.insert(id, topic.to_string());
            // Subscribe to topic.
            if let Err(sub_err) = self.mqtt_client.subscribe(topic, id).await {
                // If subscribe fails, add subscription id back to free subscription ids.
                self.add_free_subscription_id(id).await;
                return Err(sub_err);
            };
        }

        let listeners = topic_listener_map
            .entry(topic.to_string())
            .or_insert(HashSet::new());

        // Add listener to hash set.
        let comp_listener = ComparableListener::new(listener);
        listeners.insert(comp_listener);

        Ok(())
    }

    /// Remove a UListener from an mqtt topic.
    ///
    /// # Arguments
    /// * `topic` - Topic to remove the listener from.
    /// * `listener` - Listener to remove from the topic subscription list.
    async fn remove_listener(
        &self,
        topic: &str,
        listener: Arc<dyn up_rust::UListener>,
    ) -> Result<(), UStatus> {
        let mut topic_listener_map = self.topic_listener_map.write().await;
        let mut subscription_topic_map = self.subscription_topic_map.write().await;

        if !topic_listener_map.contains_key(topic) {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("Topic '{topic}' is not registered."),
            ));
        }

        topic_listener_map
            .entry(topic.to_string())
            .and_modify(|listeners| {
                // Remove listener from hash set.
                let comp_listener = ComparableListener::new(listener);
                listeners.remove(&comp_listener);
            });

        // Remove topic if no listeners are left.
        if topic_listener_map.get(topic).unwrap().is_empty() {
            let sub_id = subscription_topic_map.iter_mut().find_map(|(k, v)| {
                if v == topic {
                    Some(*k)
                } else {
                    None
                }
            });

            if let Some(sub_id) = sub_id {
                // Remove subscription id from map.
                subscription_topic_map.remove(&sub_id);

                // Add subscription id back to free subscription ids.
                self.add_free_subscription_id(sub_id).await;
            }

            topic_listener_map.remove(topic);

            // Unsubscribe from topic.
            self.mqtt_client.unsubscribe(topic).await?;
        }

        Ok(())
    }

    /// Create mqtt header properties from UAttributes information.
    ///
    /// # Arguments
    /// * `attributes` - UAttributes to create properties from.
    fn create_mqtt_properties_from_uattributes(
        attributes: &UAttributes,
    ) -> Result<mqtt::Properties, UStatus> {
        let mut properties = mqtt::Properties::new();

        // Validate UAttributes before conversion.
        UAttributesValidators::get_validator_for_attributes(attributes)
            .validate(attributes)
            .map_err(|e| {
                UStatus::fail_with_code(UCode::INTERNAL, format!("Invalid uAttributes, err: {e:?}"))
            })?;

        // Add uAttributes version number to properties.
        properties
            .push_string_pair(mqtt::PropertyCode::UserProperty, "0", "1")
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Unable to create version mqtt property, err: {e:?}"),
                )
            })?;

        // Iterate over all fields in UAttributes and extract protobuf field numbers and values.
        for field in attributes.descriptor_dyn().fields() {
            // Get protobuf field number as string.
            let field_proto_number = field.number().to_string();

            // If field is not singular, log warning and skip field.
            if !field.is_singular() {
                warn!(
                    "Unable to process non-singular field type: {}",
                    field.name()
                );
                continue;
            }

            // log if field value is not present.
            let Some(field_val_wrapped) = field.get_singular(attributes) else {
                trace!("Field value not present for field: {}", field.name());
                continue;
            };

            match field_val_wrapped.get_type() {
                protobuf::reflect::RuntimeType::U32 => {
                    if let Some(u32_val) = field_val_wrapped.to_u32() {
                        properties
                            .push_string_pair(
                                mqtt::PropertyCode::UserProperty,
                                &field_proto_number,
                                &u32_val.to_string(),
                            )
                            .map_err(|e| {
                                UStatus::fail_with_code(
                                    UCode::INTERNAL,
                                    format!("Unable to create u32 mqtt property, err: {e:?}"),
                                )
                            })?;
                    }
                }
                protobuf::reflect::RuntimeType::String => {
                    if let Some(string_val) = field_val_wrapped.to_str() {
                        properties
                            .push_string_pair(
                                mqtt::PropertyCode::UserProperty,
                                &field_proto_number,
                                string_val,
                            )
                            .map_err(|e| {
                                UStatus::fail_with_code(
                                    UCode::INTERNAL,
                                    format!("Unable to create string mqtt property, err: {e:?}"),
                                )
                            })?;
                    }
                }
                protobuf::reflect::RuntimeType::Enum(_) => {
                    if let Some(enum_number) = field_val_wrapped.to_enum_value() {
                        properties
                            .push_string_pair(
                                mqtt::PropertyCode::UserProperty,
                                &field_proto_number,
                                &enum_number.to_string(),
                            )
                            .map_err(|e| {
                                UStatus::fail_with_code(
                                    UCode::INTERNAL,
                                    format!("Unable to create enum mqtt property, err: {e:?}"),
                                )
                            })?;
                    }
                }
                protobuf::reflect::RuntimeType::Message(descriptor) => {
                    // Get type name of message to use for downcasting.
                    let message_type_name: &str = &descriptor.name().to_ascii_lowercase();

                    // If field value can be unwrapped as a MessageRef, process the field value.
                    // Currently only set up to process `UUID` and `UURI` types. Add more as needed.
                    let Some(field_val) = field_val_wrapped.to_message() else {
                        return Err(UStatus::fail_with_code(
                            UCode::INTERNAL,
                            format!(
                                "Unable to process field value from uAttributes: {}",
                                field.name()
                            ),
                        ));
                    };

                    let val = field_val.deref();

                    match message_type_name {
                        UUID_NAME => {
                            let Some(uuid) = val.downcast_ref::<UUID>() else {
                                return Err(UStatus::fail_with_code(
                                    UCode::INTERNAL,
                                    format!(
                                        "Unable to downcast field value to UUID: {}",
                                        field.name()
                                    ),
                                ));
                            };

                            properties
                                .push_string_pair(
                                    mqtt::PropertyCode::UserProperty,
                                    &field_proto_number,
                                    &uuid.to_hyphenated_string(),
                                )
                                .map_err(|e| {
                                    UStatus::fail_with_code(
                                        UCode::INTERNAL,
                                        format!("Unable to create uuid mqtt property, err: {e:?}"),
                                    )
                                })?;
                        }
                        UURI_NAME => {
                            let Some(uuri) = val.downcast_ref::<UUri>() else {
                                return Err(UStatus::fail_with_code(
                                    UCode::INTERNAL,
                                    format!(
                                        "Unable to downcast field value to UUri: {}",
                                        field.name()
                                    ),
                                ));
                            };

                            let uuri_string: String = uuri.into();
                            properties
                                .push_string_pair(
                                    mqtt::PropertyCode::UserProperty,
                                    &field_proto_number,
                                    &uuri_string,
                                )
                                .map_err(|e| {
                                    UStatus::fail_with_code(
                                        UCode::INTERNAL,
                                        format!("Unable to create uuri mqtt property, err: {e:?}"),
                                    )
                                })?;
                        }
                        _ => {
                            return Err(UStatus::fail_with_code(
                                UCode::INTERNAL,
                                format!("Unsupported message type: {}", message_type_name),
                            ));
                        }
                    }
                }
                _ => {
                    return Err(UStatus::fail_with_code(
                        UCode::INTERNAL,
                        format!(
                            "Unsupported protobuf field type: {}",
                            field_val_wrapped.get_type()
                        ),
                    ));
                }
            }
        }

        Ok(properties)
    }

    /// Get UAttributes from mqtt header properties.
    ///
    /// # Arguments
    /// * `props` - Mqtt properties to get UAttributes from.
    fn get_uattributes_from_mqtt_properties(
        props: &mqtt::Properties,
    ) -> Result<UAttributes, UStatus> {
        let mut attributes = UAttributes::default();

        for (key, value) in props.user_iter() {
            let protobuf_field_number: u32 = key.parse().map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Unable to parse attribute field number, err: {e:?}"),
                )
            })?;

            if protobuf_field_number == 0 {
                let _attributes_version: u32 = value.parse().map_err(|e| {
                    UStatus::fail_with_code(
                        UCode::INTERNAL,
                        format!("Unable to parse uAttribute version number, err: {e:?}"),
                    )
                })?;

                //TODO: Add version check here if needed.
                continue;
            }

            let Some(field) = attributes
                .descriptor_dyn()
                .field_by_number(protobuf_field_number)
            else {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Unable to map user property to uAttributes: {key:?} - {value:?}"),
                ));
            };

            // Need to get the reflect value to properly set the field.
            let field_value = field.get_singular_field_or_default(&attributes);
            let field_type = field_value.get_type();

            let value_box = match field_type {
                protobuf::reflect::RuntimeType::U32 => {
                    let u32_val = value.parse::<u32>().map_err(|e| {
                        UStatus::fail_with_code(
                            UCode::INTERNAL,
                            format!("Unable to parse attribute field {protobuf_field_number} to value, err: {e:?}"),
                        )
                    })?;
                    Ok(protobuf::reflect::ReflectValueBox::U32(u32_val))
                }
                protobuf::reflect::RuntimeType::String => {
                    Ok(protobuf::reflect::ReflectValueBox::String(value))
                }
                protobuf::reflect::RuntimeType::Enum(descriptor) => {
                    let enum_val = value.parse::<i32>().map_err(|e| {
                        UStatus::fail_with_code(
                            UCode::INTERNAL,
                            format!("Unable to parse attribute field {protobuf_field_number} to enum, err: {e:?}"),
                        )
                    })?;
                    Ok(protobuf::reflect::ReflectValueBox::Enum(
                        descriptor.clone(),
                        enum_val,
                    ))
                }
                protobuf::reflect::RuntimeType::Message(descriptor) => {
                    // Get type name of message to use for downcasting.
                    let message_type_name: &str = &descriptor.name().to_ascii_lowercase();

                    // If field value can be unwrapped as a MessageRef, process the field value.
                    // Currently only set up to process `UUID` and `UURI` types. Add more as needed.
                    match message_type_name {
                        UUID_NAME => {
                            let uuid = UUID::from_str(&value).map_err(|e| {
                                UStatus::fail_with_code(
                                    UCode::INTERNAL,
                                    format!("Unable to parse attribute field {protobuf_field_number} to uuid message, err: {e:?}"),
                                )
                            })?;
                            Ok(protobuf::reflect::ReflectValueBox::Message(Box::new(uuid)))
                        }
                        UURI_NAME => {
                            let uuri = UUri::from_str(&value).map_err(|e| {
                                UStatus::fail_with_code(
                                    UCode::INTERNAL,
                                    format!("Unable to parse attribute field {protobuf_field_number} to uuri message, err: {e:?}"),
                                )
                            })?;
                            Ok(protobuf::reflect::ReflectValueBox::Message(Box::new(uuri)))
                        }
                        _ => Err(UStatus::fail_with_code(
                            UCode::INTERNAL,
                            format!(
                                "Unsupported message type for field {protobuf_field_number}: {}",
                                message_type_name
                            ),
                        )),
                    }
                }
                _ => Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!(
                        "Unsupported protobuf field type for field {protobuf_field_number}: {}",
                        field_type
                    ),
                )),
            }?;

            field.set_singular_field(&mut attributes, value_box)
        }

        // Validate the reconstructed attributes
        UAttributesValidators::get_validator_for_attributes(&attributes)
            .validate(&attributes)
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Unable to construct uAttributes, err: {e:?}"),
                )
            })?;

        Ok(attributes)
    }

    /// Convert a UUri to a valid mqtt topic segment.
    ///
    /// # Arguments
    /// * `uri` - UUri to convert to mqtt topic segment.
    fn uri_to_mqtt_topic_segment(&self, uri: &UUri) -> String {
        let authority = if uri.authority_name.is_empty() {
            &self.authority_name
        } else if uri.authority_name == WILDCARD_AUTHORITY {
            "+"
        } else {
            &uri.authority_name
        };

        let ue_id = if uri.ue_id == WILDCARD_ENTITY_ID {
            "+".into()
        } else {
            format!("{:X}", uri.ue_id)
        };

        let ue_ver = if uri.ue_version_major == WILDCARD_ENTITY_VERSION {
            "+".into()
        } else {
            format!("{:X}", uri.ue_version_major)
        };

        let res_id = if uri.resource_id == WILDCARD_RESOURCE_ID {
            "+".into()
        } else {
            format!("{:X}", uri.resource_id)
        };

        format!("{authority}/{ue_id}/{ue_ver}/{res_id}")
    }

    /// Create a valid mqtt topic based on a source and sink UUri.
    ///
    /// # Arguments
    /// * `src_uri` - Source UUri.
    /// * `sink_uri` - Optional sink UUri.
    fn to_mqtt_topic_string(&self, src_uri: &UUri, sink_uri: Option<&UUri>) -> String {
        let cli_indicator = &self.get_client_indicator();
        let src_segment = &self.uri_to_mqtt_topic_segment(src_uri);
        if let Some(sink) = sink_uri {
            let sink_segment = self.uri_to_mqtt_topic_segment(sink);
            return format!("{cli_indicator}/{src_segment}/{sink_segment}");
        }

        format!("{cli_indicator}/{src_segment}")
    }
}

#[cfg(test)]
mod tests {
    use protobuf::Enum;
    use up_rust::{UListener, UMessageType, UPayloadFormat, UPriority, UUID};

    use test_case::test_case;

    use super::*;

    /// Constants defining the protobuf field numbers for UAttributes.
    pub const ID_NUM: &str = "1";
    pub const TYPE_NUM: &str = "2";
    pub const SOURCE_NUM: &str = "3";
    pub const SINK_NUM: &str = "4";
    pub const PRIORITY_NUM: &str = "5";
    pub const TTL_NUM: &str = "6";
    pub const PERM_LEVEL_NUM: &str = "7";
    pub const COMMSTATUS_NUM: &str = "8";
    pub const REQID_NUM: &str = "9";
    pub const TOKEN_NUM: &str = "10";
    pub const TRACEPARENT_NUM: &str = "11";
    pub const PAYLOAD_NUM: &str = "12";

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

    // Helper function used to create a UAttributes object and mqtt properties object for testing and comparison.
    #[allow(clippy::too_many_arguments)]
    fn create_test_uattributes_and_properties(
        id: Option<UUID>,
        type_: Option<UMessageType>,
        source: Option<&str>,
        sink: Option<&str>,
        priority: Option<UPriority>,
        ttl: Option<u32>,
        perm_level: Option<u32>,
        commstatus: Option<UCode>,
        reqid: Option<UUID>,
        token: Option<&str>,
        traceparent: Option<&str>,
        payload_format: Option<UPayloadFormat>,
    ) -> (UAttributes, mqtt::Properties) {
        let uattributes = create_uattributes(
            id.clone(),
            type_,
            source,
            sink,
            priority,
            ttl,
            perm_level,
            commstatus,
            reqid.clone(),
            token,
            traceparent,
            payload_format,
        );

        let properties = create_mqtt_properties(
            id,
            type_,
            source,
            sink,
            priority,
            ttl,
            perm_level,
            commstatus,
            reqid,
            token,
            traceparent,
            payload_format,
        );

        (uattributes, properties)
    }

    // Helper function to construct UAttributes object for testing.
    #[allow(clippy::too_many_arguments)]
    fn create_uattributes(
        id: Option<UUID>,
        type_: Option<UMessageType>,
        source: Option<&str>,
        sink: Option<&str>,
        priority: Option<UPriority>,
        ttl: Option<u32>,
        perm_level: Option<u32>,
        commstatus: Option<UCode>,
        reqid: Option<UUID>,
        token: Option<&str>,
        traceparent: Option<&str>,
        payload_format: Option<UPayloadFormat>,
    ) -> UAttributes {
        let mut attributes = UAttributes::default();

        if let Some(id) = id {
            attributes.id = Some(id).into();
        }

        if let Some(type_) = type_ {
            attributes.type_ = type_.into();
        }

        if let Some(source) = source {
            attributes.source =
                Some(UUri::from_str(source).expect("expected valid source UUri string.")).into();
        }

        if let Some(sink) = sink {
            attributes.sink =
                Some(UUri::from_str(sink).expect("expected valid sink UUri string.")).into();
        }

        if let Some(priority) = priority {
            attributes.priority = priority.into();
        }

        if let Some(ttl) = ttl {
            attributes.ttl = Some(ttl);
        }

        if let Some(perm_level) = perm_level {
            attributes.permission_level = Some(perm_level);
        }

        if let Some(commstatus) = commstatus {
            attributes.commstatus = Some(commstatus.into());
        }

        if let Some(reqid) = reqid {
            attributes.reqid = Some(reqid).into();
        }

        if let Some(token) = token {
            attributes.token = Some(token.to_string());
        }

        if let Some(traceparent) = traceparent {
            attributes.traceparent = Some(traceparent.to_string());
        }

        if let Some(payload_format) = payload_format {
            attributes.payload_format = payload_format.into();
        }

        attributes
    }

    // Helper function to create mqtt properties for testing.
    #[allow(clippy::too_many_arguments)]
    fn create_mqtt_properties(
        id: Option<UUID>,
        type_: Option<UMessageType>,
        source: Option<&str>,
        sink: Option<&str>,
        priority: Option<UPriority>,
        ttl: Option<u32>,
        perm_level: Option<u32>,
        commstatus: Option<UCode>,
        reqid: Option<UUID>,
        token: Option<&str>,
        traceparent: Option<&str>,
        payload_format: Option<UPayloadFormat>,
    ) -> mqtt::Properties {
        let mut properties = mqtt::Properties::new();

        // Add uAttributes version number.
        properties
            .push_string_pair(mqtt::PropertyCode::UserProperty, "0", "1")
            .unwrap();

        if let Some(id_val) = id {
            properties
                .push_string_pair(
                    mqtt::PropertyCode::UserProperty,
                    ID_NUM,
                    &id_val.to_hyphenated_string(),
                )
                .unwrap();
        }
        if let Some(type_val) = type_ {
            properties
                .push_string_pair(
                    mqtt::PropertyCode::UserProperty,
                    TYPE_NUM,
                    &type_val.value().to_string(),
                )
                .unwrap();
        }
        if let Some(source_val) = source {
            properties
                .push_string_pair(mqtt::PropertyCode::UserProperty, SOURCE_NUM, source_val)
                .unwrap();
        }
        if let Some(sink_val) = sink {
            properties
                .push_string_pair(mqtt::PropertyCode::UserProperty, SINK_NUM, sink_val)
                .unwrap();
        }
        if let Some(priority_val) = priority {
            properties
                .push_string_pair(
                    mqtt::PropertyCode::UserProperty,
                    PRIORITY_NUM,
                    &priority_val.value().to_string(),
                )
                .unwrap();
        }
        if let Some(ttl_val) = ttl {
            properties
                .push_string_pair(
                    mqtt::PropertyCode::UserProperty,
                    TTL_NUM,
                    &ttl_val.to_string(),
                )
                .unwrap();
        }
        if let Some(perm_level_val) = perm_level {
            properties
                .push_string_pair(
                    mqtt::PropertyCode::UserProperty,
                    PERM_LEVEL_NUM,
                    &perm_level_val.to_string(),
                )
                .unwrap();
        }
        if let Some(commstatus_val) = commstatus {
            properties
                .push_string_pair(
                    mqtt::PropertyCode::UserProperty,
                    COMMSTATUS_NUM,
                    &commstatus_val.value().to_string(),
                )
                .unwrap();
        }
        if let Some(reqid_val) = reqid {
            properties
                .push_string_pair(
                    mqtt::PropertyCode::UserProperty,
                    REQID_NUM,
                    &reqid_val.to_hyphenated_string(),
                )
                .unwrap();
        }
        if let Some(token_val) = token {
            properties
                .push_string_pair(mqtt::PropertyCode::UserProperty, TOKEN_NUM, token_val)
                .unwrap();
        }
        if let Some(traceparent_val) = traceparent {
            properties
                .push_string_pair(
                    mqtt::PropertyCode::UserProperty,
                    TRACEPARENT_NUM,
                    traceparent_val,
                )
                .unwrap();
        }
        if let Some(payload_format_val) = payload_format {
            properties
                .push_string_pair(
                    mqtt::PropertyCode::UserProperty,
                    PAYLOAD_NUM,
                    &payload_format_val.value().to_string(),
                )
                .unwrap();
        }

        properties
    }

    #[tokio::test]
    async fn test_get_free_subscription_id() {
        let up_client = UPClientMqtt {
            mqtt_client: Arc::new(MockMqttClient {}),
            subscription_topic_map: Arc::new(RwLock::new(HashMap::new())),
            topic_listener_map: Arc::new(RwLock::new(HashMap::new())),
            authority_name: "test".to_string(),
            client_type: UPClientMqttType::Device,
            free_subscription_ids: Arc::new(RwLock::new((1..3).collect())),
            cb_message_handle: None,
        };

        let expected_vals: Vec<i32> = up_client
            .free_subscription_ids
            .read()
            .await
            .iter()
            .cloned()
            .collect();
        let mut collected_vals = Vec::<i32>::new();

        let result = up_client.get_free_subscription_id().await;

        assert!(result.is_ok());
        collected_vals.push(result.unwrap());

        let result = up_client.get_free_subscription_id().await;

        assert!(result.is_ok());
        collected_vals.push(result.unwrap());

        assert!(collected_vals.len() == 2);
        assert!(collected_vals.iter().all(|x| expected_vals.contains(x)));
        assert!(up_client.free_subscription_ids.read().await.is_empty());

        let result = up_client.get_free_subscription_id().await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_add_free_subscription_id() {
        let up_client = UPClientMqtt {
            mqtt_client: Arc::new(MockMqttClient {}),
            subscription_topic_map: Arc::new(RwLock::new(HashMap::new())),
            topic_listener_map: Arc::new(RwLock::new(HashMap::new())),
            authority_name: "test".to_string(),
            client_type: UPClientMqttType::Device,
            free_subscription_ids: Arc::new(RwLock::new((1..3).collect())),
            cb_message_handle: None,
        };

        let expected_id = 7;

        up_client.add_free_subscription_id(expected_id).await;

        let free_ids = up_client.free_subscription_ids.read().await;

        assert!(free_ids.contains(&expected_id));
    }

    #[tokio::test]
    async fn test_add_listener() {
        let listener = Arc::new(SimpleListener {});
        let expected_listener = ComparableListener::new(listener.clone());
        let sub_map = Arc::new(RwLock::new(HashMap::new()));
        let topic_map = Arc::new(RwLock::new(HashMap::new()));

        let up_client = UPClientMqtt {
            mqtt_client: Arc::new(MockMqttClient {}),
            subscription_topic_map: sub_map.clone(),
            topic_listener_map: topic_map.clone(),
            authority_name: "test".to_string(),
            client_type: UPClientMqttType::Device,
            free_subscription_ids: Arc::new(RwLock::new((1..10).collect())),
            cb_message_handle: None,
        };

        assert!(topic_map.read().await.is_empty());

        let result = up_client.add_listener("test_topic", listener.clone()).await;

        assert!(result.is_ok());

        let actual_topic_map = topic_map.read().await;

        assert!(actual_topic_map.contains_key("test_topic"));
        let actual_listeners = actual_topic_map.get("test_topic").unwrap();
        assert_eq!(actual_listeners.len(), 1);
        assert!(actual_listeners.contains(&expected_listener));
    }

    #[tokio::test]
    async fn test_remove_listener() {
        let listener_1 = Arc::new(SimpleListener {});
        let comparable_listener_1 = ComparableListener::new(listener_1.clone());
        let listener_2 = Arc::new(SimpleListener {});
        let comparable_listener_2 = ComparableListener::new(listener_2.clone());
        let sub_map = Arc::new(RwLock::new(HashMap::new()));
        let topic_map = Arc::new(RwLock::new(HashMap::new()));

        topic_map.write().await.insert(
            "test_topic".to_string(),
            [comparable_listener_1.clone(), comparable_listener_2.clone()]
                .iter()
                .cloned()
                .collect(),
        );

        let up_client = UPClientMqtt {
            mqtt_client: Arc::new(MockMqttClient {}),
            subscription_topic_map: sub_map.clone(),
            topic_listener_map: topic_map.clone(),
            authority_name: "test".to_string(),
            client_type: UPClientMqttType::Device,
            free_subscription_ids: Arc::new(RwLock::new((1..10).collect())),
            cb_message_handle: None,
        };

        assert!(!topic_map.read().await.is_empty());

        let result = up_client
            .remove_listener("test_topic", listener_1.clone())
            .await;

        assert!(result.is_ok());

        {
            let actual_topic_map = topic_map.read().await;

            assert!(actual_topic_map.contains_key("test_topic"));
            let actual_listeners = actual_topic_map.get("test_topic").unwrap();
            assert_eq!(actual_listeners.len(), 1);
            assert!(!actual_listeners.contains(&comparable_listener_1));
            assert!(actual_listeners.contains(&comparable_listener_2));
        }

        let result = up_client
            .remove_listener("test_topic", listener_2.clone())
            .await;

        assert!(result.is_ok());
        assert!(topic_map.read().await.is_empty());

        let result = up_client
            .remove_listener("test_topic", listener_2.clone())
            .await;

        assert!(result.is_err());
        assert!(result.err().unwrap().code == UCode::NOT_FOUND.into());
    }

    #[test_case(
        create_test_uattributes_and_properties(
            Some(UUID::build()),
            Some(UMessageType::UMESSAGE_TYPE_PUBLISH),
            Some("//VIN.vehicles/A8000/2/8A50"),
            None, None, None, None, None, None, None, None, None
        ),
        4,
        None;
        "Publish success"
    )]
    #[test_case(
        create_test_uattributes_and_properties(
            Some(UUID::build()),
            Some(UMessageType::UMESSAGE_TYPE_NOTIFICATION),
            Some("//VIN.vehicles/A8000/2/1A50"),
            Some("//VIN.vehicles/B8000/3/0"),
            None, None, None, None, None, None, None, None
        ),
        5,
        None;
        "Notification success"
    )]
    #[test_case(
        create_test_uattributes_and_properties(
            Some(UUID::build()),
            Some(UMessageType::UMESSAGE_TYPE_REQUEST),
            Some("//VIN.vehicles/A8000/2/0"),
            Some("//VIN.vehicles/B8000/3/1B50"),
            Some(UPriority::UPRIORITY_CS4),
            Some(3600),
            None, None, None, None, None, None
        ),
        7,
        None;
        "Request success"
    )]
    #[test_case(
        create_test_uattributes_and_properties(
            Some(UUID::build()),
            Some(UMessageType::UMESSAGE_TYPE_RESPONSE),
            Some("//VIN.vehicles/B8000/3/1B50"),
            Some("//VIN.vehicles/A8000/2/0"),
            Some(UPriority::UPRIORITY_CS4),
            None, None, None,
            Some(UUID::build()),
            None, None, None
        ),
        7,
        None;
        "Response success"
    )]
    #[test_case(
        create_test_uattributes_and_properties(
            Some(UUID::build()),
            Some(UMessageType::UMESSAGE_TYPE_PUBLISH),
            Some("//VIN.vehicles/A8000/2/1A50"),
            None, None, None, None, None, None, None, None, None
        ),
        4,
        Some(UStatus::fail_with_code(UCode::INTERNAL, "Invalid uAttributes, err: ValidationError(\"Validation failure: Invalid source URI: Validation error: Resource ID must be >= 0x8000\")".to_string()));
        "Publish failure with validation error"
    )]
    fn test_create_mqtt_properties_from_uattributes(
        (attributes, properties): (UAttributes, mqtt::Properties),
        expected_attributes_num: usize,
        expected_error: Option<UStatus>,
    ) {
        let props = UPClientMqtt::create_mqtt_properties_from_uattributes(&attributes);

        if props.is_ok() {
            let actual_props = props.unwrap();
            assert_eq!(actual_props.len(), expected_attributes_num);
            actual_props.user_iter().for_each(|(key, value)| {
                let expected_prop = properties.find_user_property(&key);
                assert_eq!(Some(value), expected_prop);
            });
        } else {
            assert_eq!(props.err(), expected_error);
        }
    }

    #[test_case(
        create_test_uattributes_and_properties(
            Some(UUID::build()),
            Some(UMessageType::UMESSAGE_TYPE_PUBLISH),
            Some("//VIN.vehicles/A8000/2/8A50"),
            None, None, None, None, None, None, None, None, None
        ),
        None;
        "Publish success"
    )]
    #[test_case(
        create_test_uattributes_and_properties(
            Some(UUID::build()),
            Some(UMessageType::UMESSAGE_TYPE_NOTIFICATION),
            Some("//VIN.vehicles/A8000/2/1A50"),
            Some("//VIN.vehicles/B8000/3/0"),
            None, None, None, None, None, None, None, None
        ),
        None;
        "Notification success"
    )]
    #[test_case(
        create_test_uattributes_and_properties(
            Some(UUID::build()),
            Some(UMessageType::UMESSAGE_TYPE_REQUEST),
            Some("//VIN.vehicles/A8000/2/0"),
            Some("//VIN.vehicles/B8000/3/1B50"),
            Some(UPriority::UPRIORITY_CS4),
            Some(3600),
            None, None, None, None, None, None
        ),
        None;
        "Request success"
    )]
    #[test_case(
        create_test_uattributes_and_properties(
            Some(UUID::build()),
            Some(UMessageType::UMESSAGE_TYPE_RESPONSE),
            Some("//VIN.vehicles/B8000/3/1B50"),
            Some("//VIN.vehicles/A8000/2/0"),
            Some(UPriority::UPRIORITY_CS4),
            None, None, None,
            Some(UUID::build()),
            None, None, None
        ),
        None;
        "Response success"
    )]
    #[test_case(
        create_test_uattributes_and_properties(
            Some(UUID::build()),
            Some(UMessageType::UMESSAGE_TYPE_PUBLISH),
            Some("//VIN.vehicles/A8000/2/1A50"),
            None, None, None, None, None, None, None, None, None
        ),
        Some(UStatus::fail_with_code(UCode::INTERNAL, "Unable to construct uAttributes, err: ValidationError(\"Validation failure: Invalid source URI: Validation error: Resource ID must be >= 0x8000\")".to_string()));
        "Publish failure with validation error"
    )]
    fn test_get_uattributes_from_mqtt_properties(
        (attributes, properties): (UAttributes, mqtt::Properties),
        expected_error: Option<UStatus>,
    ) {
        let attributes_result = UPClientMqtt::get_uattributes_from_mqtt_properties(&properties);

        if attributes_result.is_ok() {
            let actual_attributes = attributes_result.unwrap();
            assert_eq!(actual_attributes, attributes);
        } else {
            assert_eq!(attributes_result.err(), expected_error);
        }
    }

    #[test_case(
        "//VIN.vehicles/A8000/2/8A50",
        "VIN.vehicles/A8000/2/8A50";
        "Valid uuri"
    )]
    #[test_case(
        "A8000/2/8A50",
        "VIN.vehicles/A8000/2/8A50";
        "Local uuri"
    )]
    #[test_case(
        &format!("//{WILDCARD_AUTHORITY}/A8000/2/8A50"),
        "+/A8000/2/8A50";
        "Wildcard authority"
    )]
    #[test_case(
        &format!("//VIN.vehicles/{WILDCARD_ENTITY_ID:X}/2/8A50"),
        "VIN.vehicles/+/2/8A50";
        "Wildcard entity id"
    )]
    #[test_case(
        &format!("//VIN.vehicles/A8000/{WILDCARD_ENTITY_VERSION:X}/8A50"),
        "VIN.vehicles/A8000/+/8A50";
        "Wildcard entity version"
    )]
    #[test_case(
        &format!("//VIN.vehicles/A8000/2/{WILDCARD_RESOURCE_ID:X}"),
        "VIN.vehicles/A8000/2/+";
        "Wildcard resource id"
    )]
    fn test_uri_to_mqtt_topic_segment(uuri: &str, expected_segment: &str) {
        let uuri = UUri::from_str(uuri).expect("expected valid UUri string.");

        let up_client = UPClientMqtt {
            mqtt_client: Arc::new(MockMqttClient {}),
            subscription_topic_map: Arc::new(RwLock::new(HashMap::new())),
            topic_listener_map: Arc::new(RwLock::new(HashMap::new())),
            authority_name: "VIN.vehicles".to_string(),
            client_type: UPClientMqttType::Device,
            free_subscription_ids: Arc::new(RwLock::new((1..10).collect())),
            cb_message_handle: None,
        };

        let actual_segment = up_client.uri_to_mqtt_topic_segment(&uuri);

        assert_eq!(&actual_segment, expected_segment);
    }

    #[test_case(
        "//VIN.vehicles/A8000/2/8A50",
        None,
        UPClientMqttType::Device,
        "d/VIN.vehicles/A8000/2/8A50";
        "Subscribe to a specific publish topic"
    )]
    #[test_case(
        "//VIN.vehicles/A8000/2/8A50",
        Some("//VIN.vehicles/B8000/3/0"),
        UPClientMqttType::Device,
        "d/VIN.vehicles/A8000/2/8A50/VIN.vehicles/B8000/3/0";
        "Subscribe to a specific notification topic"
    )]
    #[test_case(
        "//VIN.vehicles/A8000/2/0",
        Some("//VIN.vehicles/B8000/3/1B50"),
        UPClientMqttType::Device,
        "d/VIN.vehicles/A8000/2/0/VIN.vehicles/B8000/3/1B50";
        "Request from device"
    )]
    #[test_case(
        "//VIN.vehicles/B8000/3/1B50",
        Some("//VIN.vehicles/A8000/2/0"),
        UPClientMqttType::Device,
        "d/VIN.vehicles/B8000/3/1B50/VIN.vehicles/A8000/2/0";
        "Response from device"
    )]
    #[test_case(
        &format!("//{WILDCARD_AUTHORITY}/{WILDCARD_ENTITY_ID:X}/{WILDCARD_ENTITY_VERSION:X}/{WILDCARD_RESOURCE_ID:X}"),
        Some("//VIN.vehicles/AB34/1/12CD"),
        UPClientMqttType::Device,
        "d/+/+/+/+/VIN.vehicles/AB34/1/12CD";
        "Subscribe to incoming requests for a specific method"
    )]
    #[test_case(
        &format!("//{WILDCARD_AUTHORITY}/{WILDCARD_ENTITY_ID:X}/{WILDCARD_ENTITY_VERSION:X}/{WILDCARD_RESOURCE_ID:X}"),
        Some(&format!("//VIN.vehicles/{WILDCARD_ENTITY_ID:X}/{WILDCARD_ENTITY_VERSION:X}/{WILDCARD_RESOURCE_ID:X}")),
        UPClientMqttType::Cloud,
        "c/+/+/+/+/VIN.vehicles/+/+/+";
        "Subscribe to all incoming messages to a UAuthority in the cloud"
    )]
    #[test_case(
        &format!("//VIN.vehicles/{WILDCARD_ENTITY_ID:X}/{WILDCARD_ENTITY_VERSION:X}/{WILDCARD_RESOURCE_ID:X}"),
        None,
        UPClientMqttType::Device,
        "d/VIN.vehicles/+/+/+";
        "Subscribe to all publish messages from a different UAuthority"
    )]
    #[test_case(
        &format!("//{WILDCARD_AUTHORITY}/{WILDCARD_ENTITY_ID:X}/{WILDCARD_ENTITY_VERSION:X}/{WILDCARD_RESOURCE_ID:X}"),
        Some(&format!("//VIN.vehicles/{WILDCARD_ENTITY_ID:X}/{WILDCARD_ENTITY_VERSION:X}/0")),
        UPClientMqttType::Cloud,
        "c/+/+/+/+/VIN.vehicles/+/+/0";
        "Streamer subscribe to all notifications, requests and responses to its device from the cloud"
    )]
    #[test_case(
        &format!("//{WILDCARD_AUTHORITY}/{WILDCARD_ENTITY_ID:X}/{WILDCARD_ENTITY_VERSION:X}/{WILDCARD_RESOURCE_ID:X}"),
        None,
        UPClientMqttType::Device,
        "d/+/+/+/+";
        "Subscribe to all publish messages from devices"
    )]
    #[test_case(
        &format!("//VIN.vehicles/{WILDCARD_ENTITY_ID:X}/{WILDCARD_ENTITY_VERSION:X}/{WILDCARD_RESOURCE_ID:X}"),
        Some(&format!("//{WILDCARD_AUTHORITY}/{WILDCARD_ENTITY_ID:X}/{WILDCARD_ENTITY_VERSION:X}/{WILDCARD_RESOURCE_ID:X}")),
        UPClientMqttType::Device,
        "d/VIN.vehicles/+/+/+/+/+/+/+";
        "Subscribe to all message types but publish messages sent from a UAuthority"
    )]
    fn test_to_mqtt_topic_string(
        src_uri: &str,
        sink_uri: Option<&str>,
        client_type: UPClientMqttType,
        expected_topic: &str,
    ) {
        let src_uri = UUri::from_str(src_uri).expect("expected valid source UUri string.");
        let sink_uri =
            sink_uri.map(|uri| UUri::from_str(uri).expect("expected valid sink UUri string."));

        let up_client = UPClientMqtt {
            mqtt_client: Arc::new(MockMqttClient {}),
            subscription_topic_map: Arc::new(RwLock::new(HashMap::new())),
            topic_listener_map: Arc::new(RwLock::new(HashMap::new())),
            authority_name: "VIN.vehicles".to_string(),
            client_type,
            free_subscription_ids: Arc::new(RwLock::new((1..10).collect())),
            cb_message_handle: None,
        };

        let actual_topic = up_client.to_mqtt_topic_string(&src_uri, sink_uri.as_ref());

        assert_eq!(actual_topic, expected_topic);
    }

    #[test_case("d/VIN.vehicles/A8000/2/8A50", "d/VIN.vehicles/A8000/2/8A50", true; "Exact match")]
    #[test_case("d/VIN.vehicles/A8000/2/8A50", "d/+/+/+/+", true; "Wildcard pattern")]
    #[test_case("d/VIN.vehicles/A8000/2/8A50", "d/VIN.vehicles/B8000/2/8A50", false; "Mismatched entity id")]
    #[test_case("d/VIN.vehicles/A8000/2/8A50", "d/+/A8000/2/8A50", true; "Single wildcard matchs")]
    fn test_compare_topic(topic: &str, pattern: &str, expected_result: bool) {
        assert_eq!(UPClientMqtt::compare_topic(topic, pattern), expected_result);
    }

    #[test_case(UPClientMqttType::Device, "d"; "Device indicator")]
    #[test_case(UPClientMqttType::Cloud, "c"; "Client indicator")]
    fn test_get_client_identifier(client_type: UPClientMqttType, expected_str: &str) {
        let client = UPClientMqtt {
            mqtt_client: Arc::new(MockMqttClient {}),
            subscription_topic_map: Arc::new(RwLock::new(HashMap::new())),
            topic_listener_map: Arc::new(RwLock::new(HashMap::new())),
            authority_name: "test".to_string(),
            client_type,
            free_subscription_ids: Arc::new(RwLock::new((1..3).collect())),
            cb_message_handle: None,
        };

        assert_eq!(client.get_client_indicator(), expected_str);
    }
}
