/********************************************************************************
 * Copyright (c) 2025 Contributors to the Eclipse Foundation
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

use up_rust::UStatus;
use up_transport_mqtt5::{Mqtt5Transport, Mqtt5TransportOptions, MqttClientOptions, TransportMode};

pub async fn create_up_transport_mqtt<S: Into<String>>(
    authority_name: S,
) -> Result<Mqtt5Transport, UStatus> {
    let config = MqttClientOptions {
        broker_uri: "mqtt://localhost:1883".to_string(),
        clean_start: false,
        client_id: None,
        max_buffered_messages: 100,
        session_expiry_interval: 3600,
        ssl_options: None,
        username: None,
        password: None,
    };
    let options = Mqtt5TransportOptions {
        mode: TransportMode::InVehicle,
        max_filters: 10,
        max_listeners_per_filter: 5,
        mqtt_client_options: config,
    };

    Mqtt5Transport::new(options, authority_name.into()).await
}
