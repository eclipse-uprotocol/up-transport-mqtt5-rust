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

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, RwLock},
    time::Duration,
};

use async_channel::Receiver;
use async_trait::async_trait;
use backon::Retryable;
#[cfg(feature = "cli")]
use clap::Args;
use log::{debug, trace};
use up_rust::{UCode, UStatus};

use crate::{listener_registry::SubscribedTopicProvider, SubscriptionIdentifier};

#[cfg(feature = "cli")]
const PARAM_MQTT_BUFFER_SIZE: &str = "mqtt-buffer-size";
#[cfg(feature = "cli")]
const PARAM_MQTT_CA_PATH: &str = "mqtt-ca-path";
#[cfg(feature = "cli")]
const PARAM_MQTT_CLEAN_START: &str = "mqtt-clean-start";
#[cfg(feature = "cli")]
const PARAM_MQTT_CLIENT_ID: &str = "mqtt-client-id";
#[cfg(feature = "cli")]
const PARAM_MQTT_ENABLE_HOSTNAME_VERIFICATION: &str = "mqtt-enable-hostname-verification";
#[cfg(feature = "cli")]
const PARAM_MQTT_KEY_STORE_PATH: &str = "mqtt-key-store-path";
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

const DEFAULT_BROKER_URI: &str = "mqtt://localhost:1883";
const DEFAULT_CLEAN_START: bool = false;
const DEFAULT_MAX_BUFFERED_MESSAGES: u16 = 0;
const DEFAULT_SESSION_EXPIRY_INTERVAL: u32 = 0;

static SUBSCRIPTION_RECREATION_IN_PROGRESS_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

#[cfg_attr(feature = "cli", derive(Args))]
/// Configuration options for the MQTT client to use for connecting to the broker.
pub struct MqttClientOptions {
    /// The client identifier to use in the MQTT CONNECT Packet.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_CLIENT_ID, value_name = "ID", env = "MQTT_CLIENT_ID"))]
    pub client_id: Option<String>,

    /// The URI of the MQTT broker to connect to.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_URI, value_name = "URI", env = "MQTT_BROKER_URI", default_value = DEFAULT_BROKER_URI))]
    pub broker_uri: String,

    /// The maximum number of outbound messages that the transport can buffer locally.
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_BUFFER_SIZE, value_name = "SIZE", env = "MQTT_BUFFER_SIZE", default_value_t = DEFAULT_MAX_BUFFERED_MESSAGES))]
    pub max_buffered_messages: u16,

    /// Indicates if the MQTT broker should start a new session (`true`) or resume an existing session
    /// when a connection has been established.
    // [impl->req~up-transport-mqtt5-session-config~1]
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_CLEAN_START, value_name = "FLAG", env = "MQTT_CLEAN_START", default_value_t = DEFAULT_CLEAN_START))]
    pub clean_start: bool,

    /// The number of seconds after which the MQTT broker should discard all (client) session state.
    // [impl->req~up-transport-mqtt5-session-config~1]
    #[cfg_attr(feature = "cli", arg(long = PARAM_MQTT_SESSION_EXPIRY, value_name = "SECONDS", env = "MQTT_SESSION_EXPIRY", default_value_t = DEFAULT_SESSION_EXPIRY_INTERVAL))]
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

impl Default for MqttClientOptions {
    /// Creates new default options.
    ///
    /// # Examples
    ///
    /// ```
    /// use up_transport_mqtt5::MqttClientOptions;
    ///
    /// let options = MqttClientOptions::default();
    /// assert!(options.client_id.is_none());
    /// assert_eq!(options.broker_uri, "mqtt://localhost:1883");
    /// assert!(!options.clean_start);
    /// assert_eq!(options.max_buffered_messages, 0);
    /// assert_eq!(options.session_expiry_interval, 0);
    /// assert!(options.username.is_none());
    /// assert!(options.password.is_none());
    /// assert!(options.ssl_options.is_none());
    /// ```
    fn default() -> Self {
        Self {
            broker_uri: DEFAULT_BROKER_URI.to_string(),
            clean_start: DEFAULT_CLEAN_START,
            client_id: None,
            max_buffered_messages: DEFAULT_MAX_BUFFERED_MESSAGES,
            password: None,
            session_expiry_interval: DEFAULT_SESSION_EXPIRY_INTERVAL,
            ssl_options: None,
            username: None,
        }
    }
}

impl TryFrom<&MqttClientOptions> for paho_mqtt::ConnectOptions {
    type Error = paho_mqtt::Error;
    fn try_from(options: &MqttClientOptions) -> Result<Self, Self::Error> {
        let ssl_options = paho_mqtt::SslOptions::try_from(options)?;
        let mut connect_options_builder = paho_mqtt::ConnectOptionsBuilder::new_v5();
        connect_options_builder
            // [impl->req~up-transport-mqtt5-session-config~1]
            .clean_start(options.clean_start)
            // session expiration as defined by client options
            // [impl->req~up-transport-mqtt5-session-config~1]
            .properties(paho_mqtt::properties![paho_mqtt::PropertyCode::SessionExpiryInterval => options.session_expiry_interval])
            // TODO: make this configiurable
            .connect_timeout(Duration::from_secs(10))
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

/// Configuration options for the MQTT client to use when connecting to a broker using TLS/SSL.
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
    /// Establishes the connection to the configured broker.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection cannot be established within the default
    /// connect timeout period.
    async fn connect(&self) -> Result<(), UStatus>;

    /// Triggers the reestablishment of a lost connection to the MQTT broker.
    ///
    /// If no connection to the broker had been established before, this function does nothing.
    ///
    /// Spawns a new task that tries to reestablish the connection using an exponential
    /// backoff algorithm. This means that the connection may not have been reestablished
    /// (yet), once the function returns.
    ///
    /// Once the connection has been reestablished, all subscriptions that had existed before
    /// the connection had been lost, will be resumed, either automatically based on a resumed
    /// session or explicitly by subscribing again to the topics of the registered listeners.
    async fn reconnect(&self);

    /// Disconnects from the broker.
    fn disconnect(&self);

    /// Checks if the client is currently connected to the broker.
    fn is_connected(&self) -> bool;

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

#[derive(Default)]
struct ConnectionState {
    subscription_ids_supported: bool,
    session_present: bool,
}

pub(crate) struct PahoBasedMqttClientOperations {
    inner_mqtt_client: Arc<paho_mqtt::AsyncClient>,
    inbound_messages: Option<Receiver<Option<paho_mqtt::Message>>>,
    subscribed_topic_provider: Arc<tokio::sync::RwLock<dyn SubscribedTopicProvider>>,
    client_options: MqttClientOptions,
}

impl PahoBasedMqttClientOperations {
    fn ustatus_from_paho_error(paho_error: paho_mqtt::Error) -> UStatus {
        match paho_error {
            paho_mqtt::Error::Disconnected => {
                UStatus::fail_with_code(UCode::UNAVAILABLE, "not connected to MQTT broker")
            }
            paho_mqtt::Error::TcpTlsConnectFailure => {
                UStatus::fail_with_code(UCode::UNAVAILABLE, "failed to connect to MQTT broker")
            }
            _ => UStatus::fail_with_code(UCode::UNKNOWN, paho_error.to_string()),
        }
    }

    /// Creates new MQTT client.
    ///
    /// # Arguments
    /// * `options` - Configuration for the MQTT client. These configuration options
    ///   are getting stored with the client and used again when reestablishing a lost
    ///   connection to the broker.
    /// * `subscribed_topic_provider` - A component that knows about the topic filters for which
    ///   listeners have been registered.
    ///
    /// # Returns
    ///
    /// A newly created MQTT client that is not connected to the broker yet (see `Self::connect`).
    pub(crate) fn new_client(
        options: MqttClientOptions,
        subscribed_topic_provider: Arc<tokio::sync::RwLock<dyn SubscribedTopicProvider>>,
    ) -> Result<Self, UStatus> {
        paho_mqtt::CreateOptionsBuilder::new()
            .server_uri(&options.broker_uri)
            .client_id(options.client_id.clone().unwrap_or_default())
            .max_buffered_messages(options.max_buffered_messages as i32)
            .user_data(Box::new(RwLock::new(ConnectionState::default())))
            .create_client()
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Failed to create MQTT client: {e:?}"),
                )
            })
            .map(|mut async_client| {
                let inbound_message_stream = async_client.get_stream(100);
                Self {
                    inner_mqtt_client: Arc::new(async_client),
                    inbound_messages: Some(inbound_message_stream),
                    subscribed_topic_provider,
                    client_options: options,
                }
            })
    }

    /// Gets the receiver side of the stream of messages coming in from the broker.
    ///
    /// It is good practice to set up the handling of messages before connecting to
    /// the broker because the messages may start flowing even before the call
    /// to `Self::connect` returns.
    ///
    /// # Errors
    ///
    /// Returns an error if the stream has already been retrieved before.
    pub(crate) fn get_message_stream(
        &mut self,
    ) -> Result<Receiver<Option<paho_mqtt::Message>>, UStatus> {
        self.inbound_messages.take().ok_or_else(|| {
            UStatus::fail_with_code(
                UCode::FAILED_PRECONDITION,
                "Inbound message stream has already been retrieved",
            )
        })
    }

    fn is_subscription_ids_supported(&self) -> bool {
        if let Some(conn_props) = self
            .inner_mqtt_client
            .user_data()
            .and_then(|user_data| user_data.downcast_ref::<RwLock<ConnectionState>>())
        {
            if let Ok(locked_props) = conn_props.read() {
                return locked_props.subscription_ids_supported;
            }
        }
        false
    }

    fn is_session_present(user_data: &paho_mqtt::UserData) -> bool {
        if let Some(connection_properties) = user_data.downcast_ref::<RwLock<ConnectionState>>() {
            if let Ok(props) = connection_properties.read() {
                return props.session_present;
            }
        }
        false
    }

    /// Updates the MQTT client's [user data](`ConnectionState`) with the connection properties
    /// contained in the CONNACK packet returned by the MQTT broker.
    fn handle_connect_response(user_data: &paho_mqtt::UserData, token: paho_mqtt::ServerResponse) {
        if let Some(connection_properties) = user_data.downcast_ref::<RwLock<ConnectionState>>() {
            if let Ok(mut props) = connection_properties.write() {
                props.subscription_ids_supported = token
                    .properties()
                    .get(paho_mqtt::PropertyCode::SubscriptionIdentifiersAvailable)
                    .and_then(|p| p.get_byte())
                    .is_none_or(|v| v == 1);
                debug!(
                    "subscription IDs supported: {}",
                    props.subscription_ids_supported
                );

                if let Some(connect_response) = token.connect_response() {
                    props.session_present = connect_response.session_present;
                    debug!("session present: {}", props.session_present);
                }
            }
        }
    }

    fn create_subscription_id_properties(
        id: u16,
    ) -> Result<paho_mqtt::Properties, paho_mqtt::Error> {
        let mut properties = paho_mqtt::Properties::new();
        properties
            .push_int(paho_mqtt::PropertyCode::SubscriptionIdentifier, id as i32)
            .inspect_err(|e| {
                debug!("Failed to create MQTT 5 SubscriptionIdentifier property: {e}")
            })?;
        Ok(properties)
    }
    // [impl->req~up-transport-mqtt5-reconnection~1]
    async fn recreate_subscriptions(
        mqtt_client: Arc<paho_mqtt::AsyncClient>,
        subscribed_topics: HashMap<SubscriptionIdentifier, String>,
    ) -> Result<(), paho_mqtt::Error> {
        for (subscription_id, topic_filter) in subscribed_topics {
            // we ignore any potential errors when creating the properties because the worst
            // thing that can happen is that we subscribe without a subscription identifier
            // and thus will need to match incoming messages based on the topic only (which we
            // are prepared to do anyway)
            let properties = Self::create_subscription_id_properties(subscription_id).ok();
            if let Err(err) = mqtt_client
                .subscribe_with_options(&topic_filter, paho_mqtt::QOS_1, None, properties)
                .await
            {
                debug!(
                    "Failed to recreate subscription [id: {}, topic filter: {}]: {}",
                    subscription_id, topic_filter, err,
                );
                return Err(err);
            } else {
                debug!(
                    "Successfully recreated subscription [id: {}, topic filter: {}]",
                    subscription_id, topic_filter
                );
            }
        }
        Ok(())
    }
}

#[async_trait]
impl MqttClientOperations for PahoBasedMqttClientOperations {
    async fn connect(&self) -> Result<(), UStatus> {
        if self.inner_mqtt_client.is_connected() {
            return Ok(());
        }
        let connect_options = paho_mqtt::ConnectOptions::try_from(&self.client_options).map_err(
            |e: paho_mqtt::Error| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, e.to_string()),
        )?;
        self.inner_mqtt_client
            .connect(connect_options)
            .await
            .map(|response| {
                if let Some(user_data) = self.inner_mqtt_client.user_data() {
                    Self::handle_connect_response(user_data, response);
                }
            })
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Failed to connect to MQTT broker: {e:?}"),
                )
            })
    }

    fn is_connected(&self) -> bool {
        self.inner_mqtt_client.is_connected()
    }

    // [impl->req~up-transport-mqtt5-reconnection~1]
    async fn reconnect(&self) {
        if self.inner_mqtt_client.is_connected() {
            debug!("Skipping reconnection attempt, connection has already been reestablished...");
            return;
        }

        let mqtt_client = self.inner_mqtt_client.clone();
        let topic_provider = self.subscribed_topic_provider.clone();

        let backoff_builder = backon::ExponentialBuilder::new()
            .with_factor(2.0)
            .with_jitter()
            .with_min_delay(Duration::from_millis(500))
            .with_max_delay(Duration::from_secs(10))
            .without_max_times();
        match (|| {
            debug!("Attempting to reestablish connecting to broker...");
            mqtt_client.reconnect()
        })
        .retry(&backoff_builder)
        .when(|err| {
            debug!("Failed to reestablish connection to MQTT broker: {err}");
            true
        })
        .await
        {
            Ok(response) => {
                debug!("Successfully reestablished connection to MQTT broker");
                if let Some(user_data) = mqtt_client.user_data() {
                    // this will always be the case because we set the user data during
                    // construction of the AsyncClient
                    Self::handle_connect_response(user_data, response);
                    if !Self::is_session_present(user_data) {
                        // we only need to manually reestablish the subscriptions if
                        // the server has not used any session state for the new connection
                        let subscribed_topics = {
                            let topic_provider_read = topic_provider.read().await;
                            topic_provider_read.get_subscribed_topics()
                        };
                        // We try to recreate the subscribtions in the background with an infinite retry.
                        tokio::spawn(async move {
                            // Check if there is already a background job re-creating the subscribtions.
                            if SUBSCRIPTION_RECREATION_IN_PROGRESS_IN_PROGRESS
                                .load(std::sync::atomic::Ordering::Acquire)
                            {
                                return;
                            }
                            let backoff_builder = backon::ExponentialBuilder::new()
                                .with_factor(2.0)
                                .with_jitter()
                                .with_min_delay(Duration::from_millis(500))
                                .with_max_delay(Duration::from_secs(10))
                                .without_max_times();
                            // We can ignore the result since we will retry indefinitely
                            let _ = (|| {
                                Self::recreate_subscriptions(
                                    mqtt_client.clone(),
                                    subscribed_topics.clone(),
                                )
                            })
                            .retry(&backoff_builder)
                            .when(|err| {
                                debug!("Failed to recreate previously subscribed handlers: {err}");
                                true
                            })
                            .await;
                            SUBSCRIPTION_RECREATION_IN_PROGRESS_IN_PROGRESS
                                .store(false, std::sync::atomic::Ordering::Release);
                        });
                    }
                }
            }
            Err(_err) => {
                // we cannot reach this arm because we do not limit the number of attempts to connnect
            }
        }
    }

    fn disconnect(&self) {
        let _token = self.inner_mqtt_client.disconnect(None);
    }

    async fn publish(&self, mqtt_message: paho_mqtt::Message) -> Result<(), UStatus> {
        if SUBSCRIPTION_RECREATION_IN_PROGRESS_IN_PROGRESS
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(UStatus::fail_with_code(
                UCode::UNAVAILABLE,
                "Failed to publish since there is a subscription recreation running",
            ));
        }

        self.inner_mqtt_client
            .publish(mqtt_message)
            .await
            .map_err(Self::ustatus_from_paho_error)
            .map(|_| ())
    }

    async fn subscribe(&self, topic: &str, id: u16) -> Result<(), UStatus> {
        if SUBSCRIPTION_RECREATION_IN_PROGRESS_IN_PROGRESS
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(UStatus::fail_with_code(
                UCode::UNAVAILABLE,
                "Failed to subscribe since there is a subscription recreation running",
            ));
        }
        let subscription_properties = if self.is_subscription_ids_supported() {
            trace!("Creating subscription [topic: {}, ID: {}]", topic, id);
            Some(Self::create_subscription_id_properties(id).map_err(|_e| {
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Failed to create MQTT5 SubscriptionIdentifier property",
                )
            })?)
        } else {
            trace!("Creating subscription [topic: {}]", topic);
            None
        };

        self.inner_mqtt_client
            // QOS 1 - Delivered and received at least once
            .subscribe_with_options(topic, paho_mqtt::QOS_1, None, subscription_properties)
            .await
            .map_err(Self::ustatus_from_paho_error)
            .map(|_| ())
    }

    async fn unsubscribe(&self, topic: &str) -> Result<(), UStatus> {
        self.inner_mqtt_client
            .unsubscribe(topic)
            .await
            .map_err(Self::ustatus_from_paho_error)
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use paho_mqtt::ConnectOptions;

    use super::MqttClientOptions;

    #[test]
    // [utest->req~up-transport-mqtt5-session-config~1]
    fn test_config_parsing() {
        let options = MqttClientOptions {
            clean_start: true,
            session_expiry_interval: 60 * 60 * 24,
            ..Default::default()
        };
        let connect_options =
            ConnectOptions::try_from(&options).expect("failed to create ConenctOptions");
        assert!(connect_options.clean_start());
        // it is not possible to verify that the session expiry interval has been correctly set,
        // because the ConnectOptions struct does not (yet) provide access to the CONNECT packet
        // properties
    }
}
