use up_rust::{UStatus, UUID};
use up_transport_mqtt5::{MqttConfig, MqttProtocol, UPClientMqtt, UPClientMqttType};

pub async fn create_up_transport_mqtt<S: Into<String>>(
    authority_name: S,
) -> Result<UPClientMqtt, UStatus> {
    let config = MqttConfig {
        mqtt_protocol: MqttProtocol::Mqtt,
        mqtt_hostname: "localhost".to_string(),
        mqtt_port: 1883,
        max_buffered_messages: 100,
        max_subscriptions: 100,
        session_expiry_interval: 3600,
        ssl_options: None,
        username: "testuser".to_string(),
    };

    UPClientMqtt::new(
        config,
        UUID::build(),
        authority_name.into(),
        UPClientMqttType::Device,
    )
    .await
}
