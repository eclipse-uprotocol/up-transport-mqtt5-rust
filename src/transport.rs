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

use async_trait::async_trait;

use up_rust::{
    transport::datamodel::UTransport,
    uprotocol::{UMessage, UStatus, UUri},
    uuid::builder::UUIDBuilder,
};

use crate::UPClientMqtt;

#[async_trait]
impl UTransport for UPClientMqtt {
    async fn send(&self, message: UMessage) -> Result<(), UStatus> {
        // implementation goes here
        println!("Sending message: {:?}", message);

        Ok(())
    }

    async fn register_listener(
        &self,
        topic: UUri,
        listener: Box<dyn Fn(Result<UMessage, UStatus>) + Send + Sync + 'static>,
    ) -> Result<String, UStatus> {
        // implementation goes here
        println!("Registering listener for topic: {:?}", topic);

        listener(Ok(UMessage::new()));

        let listener_id = UUIDBuilder::new().build().to_string();

        Ok(listener_id)
    }

    async fn unregister_listener(&self, topic: UUri, listener: &str) -> Result<(), UStatus> {
        // implementation goes here
        println!("Unregistering listener: {listener} for topic: {:?}", topic);

        Ok(())
    }

    async fn receive(&self, _topic: UUri) -> Result<UMessage, UStatus> {
        Err(UStatus::fail_with_code(
            up_rust::uprotocol::UCode::UNIMPLEMENTED,
            "This method is not implemented for mqtt. Use register_listener instead.",
        ))
    }
}
