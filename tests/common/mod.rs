use up_rust::UStatus;
use up_transport_mqtt5::{Mqtt5Transport, MqttClientOptions, TransportMode};

pub async fn create_up_transport_mqtt<S: Into<String>>(
    authority_name: S,
) -> Result<Mqtt5Transport, UStatus> {
    let config = MqttClientOptions {
        broker_uri: "mqtt://localhost:1883".to_string(),
        clean_start: false,
        client_id: None,
        max_buffered_messages: 100,
        max_subscriptions: 100,
        session_expiry_interval: 3600,
        ssl_options: None,
        username: None,
        password: None,
    };

    Mqtt5Transport::new(TransportMode::InVehicle, config, authority_name.into()).await
}
