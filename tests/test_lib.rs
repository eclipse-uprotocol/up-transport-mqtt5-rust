use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use up_client_mqtt5_rust::{MqttConfig, MqttProtocol, UPClientMqtt, UPClientMqttType};
use up_rust::{UListener, UMessage, UStatus, UUID};

pub struct TestListener {
    pub recv_data: Arc<Mutex<String>>,
}

impl TestListener {
    pub fn get_recv_data(&self) -> String {
        self.recv_data.lock().unwrap().to_string()
    }
}

#[async_trait]
impl UListener for TestListener {
    async fn on_receive(&self, message: UMessage) {
        let data = message.payload.unwrap();
        let value = data.into_iter().map(|c| c as char).collect::<String>();
        *self.recv_data.lock().unwrap() = value;
    }
}

pub async fn create_up_transport_mqtt(authority_name: &str) -> Result<UPClientMqtt, UStatus> {
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

    let client = UPClientMqtt::new(
        config,
        UUID::build(),
        authority_name.to_string(),
        UPClientMqttType::Device,
    )
    .await?;

    Ok(client)
}
