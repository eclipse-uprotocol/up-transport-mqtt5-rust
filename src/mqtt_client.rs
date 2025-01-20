/********************************************************************************
 * Copyright (c) 2024 Contributors to the Eclipse Foundation
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

use std::{path::PathBuf, time::Duration};

use async_channel::Receiver;
use async_trait::async_trait;
#[cfg(feature = "cli")]
use clap::Args;
use log::trace;
use up_rust::{UCode, UStatus};

#[cfg(feature = "cli")]
const PARAM_MQTT_BUFFER_SIZE: &str = "mqtt-buffer-size";
#[cfg(feature = "cli")]
const PARAM_MQTT_CA_PATH: &str = "mqtt-ca-path";
#[cfg(feature = "cli")]
const PARAM_MQTT_CLIENT_ID: &str = "mqtt-client-id";
#[cfg(feature = "cli")]
const PARAM_MQTT_ENABLE_HOSTNAME_VERIFICATION: &str = "mqtt-enable-hostname-verification";
#[cfg(feature = "cli")]
const PARAM_MQTT_KEY_STORE_PATH: &str = "mqtt-key-store-path";
#[cfg(feature = "cli")]
const PARAM_MQTT_MAX_SUBSCRIPTIONS: &str = "mqtt-max-subscriptions";
#[cfg(feature = "cli")]
const PARAM_MQTT_URI: &str = "mqtt-broker-uri";
#[cfg(feature = "cli")]
const PARAM_MQTT_USERNAME: &str = "mqtt-username";
#[cfg(feature = "cli")]
const PARAM_MQTT_PASSWORD: &str = "mqtt-password";
#[cfg(feature = "cli")]
const PARAM_MQTT_PRIVATE_KEY_PATH: &str = "mqtt-private-key-path";
#[cfg(feature = "cli")]
const PARAM_MQTT_PRIVATE_KEY_PWD: &str = "mqtt-private-key-pwd";
#[cfg(feature = "cli")]
const PARAM_MQTT_SESSION_EXPIRY: &str = "mqtt-session-expiry";
#[cfg(feature = "cli")]
const PARAM_MQTT_TRUST_STORE_PATH: &str = "mqtt-trust-store-path";

#[cfg_attr(feature = "cli", derive(Args))]
/// Configuration options for the MQTT client.
pub struct MqttClientOptions {
    /// The client identifier to use in the MQTT CONNECT Packet.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_CLIENT_ID, value_name = "ID", env = "MQTT_CLIENT_ID"))]
    pub client_id: Option<String>,

    /// The URI of the MQTT broker to connect to.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_URI, value_name = "URI", env = "MQTT_BROKER_URI", default_value = "mqtt://localhost:1883"))]
    pub broker_uri: String,

    /// The maximum number of outbound messages that the transport can buffer locally.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_BUFFER_SIZE, value_name = "SIZE", env = "MQTT_BUFFER_SIZE", default_value = "20"))]
    pub max_buffered_messages: u16,

    /// The maximum number of distinct topic filters that the transport supports.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_MAX_SUBSCRIPTIONS, value_name = "NUMBER", env = "MQTT_MAX_SUBSCRIPTIONS", default_value = "50"))]
    pub max_subscriptions: u16,

    /// The number of seconds after which the MQTT broker should discard all (client) session state.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_SESSION_EXPIRY, value_name = "SECONDS", env = "MQTT_SESSION_EXPIRY", default_value = "0"))]
    pub session_expiry_interval: u32,

    /// The username to use for authenticating to the MQTT endpoint.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_USERNAME, value_name = "USERNAME", env = "MQTT_USERNAME"))]
    pub username: Option<String>,

    /// The password to use for authenticating to the MQTT endpoint.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_PASSWORD, value_name = "PWD", env = "MQTT_PASSWORD"))]
    pub password: Option<String>,

    /// Options for using TLS when connecting to the broker.
    #[cfg_attr(feature = "cli", command(flatten))]
    pub ssl_options: Option<SslOptions>,
}

impl TryFrom<&MqttClientOptions> for paho_mqtt::ConnectOptions {
    type Error = paho_mqtt::Error;
    fn try_from(options: &MqttClientOptions) -> Result<Self, Self::Error> {
        let ssl_options = paho_mqtt::SslOptions::try_from(options)?;
        let mut connect_options_builder = paho_mqtt::ConnectOptionsBuilder::new_v5();
        connect_options_builder
            .automatic_reconnect(Duration::from_secs(1), Duration::from_secs(16));
        connect_options_builder
            // always try to resume existing session
            .clean_start(false)
            // session expiration as defined by client options
            .properties(paho_mqtt::properties![paho_mqtt::PropertyCode::SessionExpiryInterval => options.session_expiry_interval])
            .ssl_options(ssl_options);
        if let Some(v) = options.username.as_ref() {
            connect_options_builder.user_name(v);
        }
        if let Some(v) = options.password.as_ref() {
            connect_options_builder.password(v);
        }
        Ok(connect_options_builder.finalize())
    }
}

impl TryFrom<&MqttClientOptions> for paho_mqtt::SslOptions {
    type Error = paho_mqtt::Error;
    fn try_from(config: &MqttClientOptions) -> Result<Self, Self::Error> {
        config
            .ssl_options
            .as_ref()
            .map_or(Ok(Self::default()), Self::try_from)
    }
}

#[cfg_attr(feature = "cli", derive(Args))]
#[derive(Clone)]
pub struct SslOptions {
    /// The path to a folder that contains PEM files for trusted certificate authorities.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_CA_PATH, value_name = "PATH", env = "CA_PATH", value_parser = clap::builder::PathBufValueParser::new()))]
    pub ca_path: Option<PathBuf>,

    /// The path to a file that contains PEM encoded trusted certificates.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_TRUST_STORE_PATH, value_name = "PATH", env = "TRUST_STORE_PATH", value_parser = clap::builder::PathBufValueParser::new()))]
    pub trust_store_path: Option<PathBuf>,

    /// The file in PEM format containing the public X.509 certificate chain to use for authenticating to a broker.
    /// May also contain the client’s private key.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_KEY_STORE_PATH, value_name = "PATH", env = "KEY_STORE_PATH", value_parser = clap::builder::PathBufValueParser::new()))]
    pub key_store_path: Option<PathBuf>,

    /// The file in PEM format containing the client’s private key (if not included in the Key Store).
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_PRIVATE_KEY_PATH, value_name = "PATH", env = "PRIVATE_KEY_PATH", value_parser = clap::builder::PathBufValueParser::new()))]
    pub private_key_path: Option<PathBuf>,

    /// The password to load the client’s private key if it’s encrypted.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_PRIVATE_KEY_PWD, value_name = "PWD"))]
    pub private_key_pwd: Option<String>,

    /// Indicates whether server certificates should be matched against the
    /// hostname/IP address used by a client to connect to the broker.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_ENABLE_HOSTNAME_VERIFICATION, value_name = "FLAG", default_value = "true", env = "ENABLE_HOSTNAME_VERIFICATION"))]
    pub enable_hostname_verification: bool,
}

impl TryFrom<&SslOptions> for paho_mqtt::SslOptions {
    type Error = paho_mqtt::Error;
    fn try_from(options: &SslOptions) -> Result<Self, Self::Error> {
        let mut builder = paho_mqtt::SslOptionsBuilder::new();
        builder.enable_server_cert_auth(options.enable_hostname_verification);
        if let Some(path) = options.ca_path.as_ref() {
            builder.ca_path(path)?;
        }
        if let Some(path) = options.trust_store_path.as_ref() {
            builder.trust_store(path)?;
        }
        if let Some(path) = options.key_store_path.as_ref() {
            builder.key_store(path)?;
        }
        if let Some(path) = options.private_key_path.as_ref() {
            builder.private_key(path)?;
        }
        if let Some(pwd) = options.private_key_pwd.as_ref() {
            builder.private_key_password(pwd);
        }
        Ok(builder.finalize())
    }
}

/// Basic operations that an MQTT client performs.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub(crate) trait MqttClientOperations: Sync + Send {
    /// Publishes a message to a topic.
    ///
    /// # Arguments
    ///
    /// * `mqtt_message` - The message to be published.
    async fn publish(&self, mqtt_message: paho_mqtt::Message) -> Result<(), UStatus>;

    /// Subscribes to a topic.
    ///
    /// # Arguments
    ///
    /// * `topic` - Topic to subscribe to.
    /// * `id` - Subscription ID for the topic, used to prevent duplication.
    async fn subscribe(&self, topic: &str, id: u16) -> Result<(), UStatus>;

    /// Unsubscribes from a topic.
    ///
    /// # Arguments
    /// * `topic` - Topic to unsubscribe from.
    async fn unsubscribe(&self, topic: &str) -> Result<(), UStatus>;
}

pub(crate) struct PahoBasedMqttClientOperations {
    inner_mqtt_client: paho_mqtt::AsyncClient,
    subscription_ids_supported: bool,
}

impl PahoBasedMqttClientOperations {
    /// Creates new MQTT client.
    ///
    /// # Arguments
    /// * `options` - Configuration for the MQTT client.
    ///
    /// # Returns
    ///
    /// A newly created MQTT client that is not connected to the broker yet (see `Self::connect`).
    pub(crate) fn new_client(options: &MqttClientOptions) -> Result<Self, UStatus> {
        paho_mqtt::CreateOptionsBuilder::new()
            .server_uri(&options.broker_uri)
            .client_id(options.client_id.clone().unwrap_or_default())
            .max_buffered_messages(options.max_buffered_messages as i32)
            .create_client()
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Failed to create MQTT client: {e:?}"),
                )
            })
            .map(|client| Self {
                inner_mqtt_client: client,
                subscription_ids_supported: true,
            })
    }

    /// Gets the receiver side of the stream of messages coming in from the broker.
    ///
    /// It is good practice to set up the handling of messages before connecting to
    /// the broker because the messages may start flowing even before the call
    /// to `Self::connect` returns.
    pub(crate) fn get_message_stream(&mut self) -> Receiver<Option<paho_mqtt::Message>> {
        self.inner_mqtt_client.get_stream(100)
    }

    /// Establishes the connection to the configured broker.
    pub(crate) async fn connect(&mut self, options: &MqttClientOptions) -> Result<(), UStatus> {
        if self.inner_mqtt_client.is_connected() {
            return Ok(());
        }
        let connect_options =
            paho_mqtt::ConnectOptions::try_from(options).map_err(|e: paho_mqtt::Error| {
                UStatus::fail_with_code(UCode::INVALID_ARGUMENT, e.to_string())
            })?;

        self.inner_mqtt_client
            .connect(connect_options)
            .await
            .map(|token| {
                self.subscription_ids_supported =
                    Self::check_subscription_identifiers_supported(token.properties());
            })
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Failed to connect to MQTT broker: {e:?}"),
                )
            })
    }

    fn check_subscription_identifiers_supported(props: &paho_mqtt::Properties) -> bool {
        props
            .get(paho_mqtt::PropertyCode::SubscriptionIdentifiersAvailable)
            .and_then(|p| p.get_byte())
            .map_or(true, |v| v == 1)
    }

    // Create a set of poperties with a single Subscription ID
    fn get_properties_for_subscription_id(id: u16) -> paho_mqtt::Properties {
        paho_mqtt::properties![
            paho_mqtt::PropertyCode::SubscriptionIdentifier => id as i32
        ]
    }
}

#[async_trait]
impl MqttClientOperations for PahoBasedMqttClientOperations {
    async fn publish(&self, mqtt_message: paho_mqtt::Message) -> Result<(), UStatus> {
        self.inner_mqtt_client
            .publish(mqtt_message)
            .await
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Unable to publish message: {e:?}"),
                )
            })
            .map(|_response| Ok(()))?
    }

    async fn subscribe(&self, topic: &str, id: u16) -> Result<(), UStatus> {
        // QOS 1 - Delivered and received at least once
        let subscription_properties = if self.subscription_ids_supported {
            trace!(
                "Subcription identifier supported by broker. Subscribe with subscription id {}",
                id
            );
            Some(Self::get_properties_for_subscription_id(id))
        } else {
            None
        };

        self.inner_mqtt_client
            .subscribe_with_options(topic, paho_mqtt::QOS_1, None, subscription_properties)
            .await
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Unable to subscribe to topic: {e:?}"),
                )
            })
            .map(|_response| Ok(()))?
    }

    async fn unsubscribe(&self, topic: &str) -> Result<(), UStatus> {
        self.inner_mqtt_client
            .unsubscribe(topic)
            .await
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Unable to unsubscribe from topic: {e:?}"),
                )
            })
            .map(|_response| Ok(()))?
    }
}
