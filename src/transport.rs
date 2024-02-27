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

use up_rust::{transport::datamodel::UTransport, uprotocol::{UMessage, UStatus, UUri}};

pub struct MqttTransport {}

impl UTransport for MqttTransport {
    fn send(&self, message: UMessage) -> Result<(), UStatus> {
        // implementation goes here
        Ok(())
    }

    fn receive(&self, topic: UUri) -> Result<UMessage, UStatus> {
        // implementation goes here
        Ok(UMessage::new())
    }

    fn register_listener(
        &self,
        topic: UUri,
        listener: Box<dyn Fn(Result<UMessage, UStatus>) + Send + Sync + 'static>,
    ) -> Result<String, UStatus> {
        // implementation goes here
        Ok("".to_string())
    }

    fn unregister_listener(&self, topic: UUri, listener: &str) -> Result<(), UStatus> {
        // implementation goes here
        Ok(())
    }
}