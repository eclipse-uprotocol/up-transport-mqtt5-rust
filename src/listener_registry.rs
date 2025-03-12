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
    collections::{HashMap, HashSet},
    sync::Arc,
};

use log::debug;
use up_rust::{ComparableListener, UCode, UStatus};

pub(crate) type SubscriptionIdentifier = u16;
type SubscriptionTopics = HashMap<SubscriptionIdentifier, String>;
type TopicListeners = paho_mqtt::TopicMatcher<HashSet<ComparableListener>>;

pub(crate) struct RegisteredListeners {
    /// Mapping of subscription identifiers to topic filters.
    subscription_topics: SubscriptionTopics,
    /// Mapping of topic filters to listeners.
    topic_listeners: TopicListeners,
    max_subscriptions: u16,
    /// List of free subscription identifiers to use for the client subscriptions.
    free_subscription_ids: HashSet<SubscriptionIdentifier>,
}

impl RegisteredListeners {
    fn new_subscription_ids(size: u16) -> HashSet<SubscriptionIdentifier> {
        (1..(size) + 1).collect()
    }

    pub(crate) fn new(max_subscriptions: u16) -> Self {
        Self {
            subscription_topics: SubscriptionTopics::new(),
            topic_listeners: TopicListeners::new(),
            max_subscriptions,
            free_subscription_ids: Self::new_subscription_ids(max_subscriptions),
        }
    }

    /// Resets this registry to its initial state.
    /// This includes removing all registered listeners, topic filters and subscription identifiers.
    pub(crate) fn clear(&mut self) {
        self.topic_listeners.clear();
        self.subscription_topics.clear();
        self.free_subscription_ids = Self::new_subscription_ids(self.max_subscriptions);
    }

    fn find_subscription_id(&self, topic_filter: &str) -> Option<SubscriptionIdentifier> {
        self.subscription_topics.iter().find_map(
            |(k, v)| {
                if v == topic_filter {
                    Some(*k)
                } else {
                    None
                }
            },
        )
    }

    /// Get an available subscription id to use.
    fn get_free_subscription_id(&mut self) -> Result<SubscriptionIdentifier, UStatus> {
        // Get a random subscription id from the free subscription ids.
        if let Some(&id) = self.free_subscription_ids.iter().next() {
            self.free_subscription_ids.remove(&id);
            Ok(id)
        } else {
            Err(UStatus::fail_with_code(
                UCode::INTERNAL,
                "Max number of subscriptions reached on this client.",
            ))
        }
    }

    /// Returns a subscription ID back to the pool of free/unused subscription IDs.
    ///
    /// Also removes all listeners registered for the given topic filter as well as
    /// the mapping of the subscription identifier to the topic filter.
    ///
    /// This function should be invoked after a failed attempt to subscribe to the
    /// topic filter with the subscription identifier.
    ///
    /// # Arguments
    /// * `id` - The subscription ID to release.
    /// * `topic_filter` - The topic filter to remove all listeners for.
    pub(crate) fn release_subscription_id(
        &mut self,
        id: SubscriptionIdentifier,
        topic_filter: &str,
    ) {
        self.topic_listeners.remove(topic_filter);
        self.subscription_topics.remove(&id);
        self.free_subscription_ids.insert(id);
    }

    /// Adds a listener for a given topic filter.
    ///
    /// The same listener instance can be registered using multiple topic filters.
    /// The listener will be invoked once per message for each distinct topic filter that
    /// the listener has been registered for and which matches the topic that the message
    /// has been published to.
    ///
    /// # Returns
    ///
    /// A newly assigned subscription identifier to be used for subscribing to the topic
    /// filter with the MQTT broker, or `None` if the listener has been added to an existing
    /// subscription for the topic filter.
    ///
    /// Note that the [`Self::release_subscription_id`] function must be invoked, if subscribing
    /// to the topic filter with the MQTT broker fails. Otherwise, the pool of available
    /// subscription IDs might exhaust early.
    pub(crate) fn add_listener(
        &mut self,
        topic_filter: &str,
        listener: Arc<dyn up_rust::UListener>,
    ) -> Result<Option<SubscriptionIdentifier>, UStatus> {
        let comp_listener = ComparableListener::new(listener);

        // [impl->dsn~utransport-registerlistener-idempotent~1]
        // [impl->dsn~utransport-registerlistener-listener-reuse~1]
        // [impl->dsn~utransport-registerlistener-number-of-listeners~1]
        if let Some(listeners) = self.topic_listeners.get_mut(topic_filter) {
            debug!(
                "Adding listener to existing subscription [topic filter: {}",
                topic_filter
            );
            listeners.insert(comp_listener);
            Ok(None)
        } else {
            let subscription_id = self.get_free_subscription_id()?;
            self.subscription_topics
                .insert(subscription_id, topic_filter.to_string());
            let mut listeners = HashSet::new();
            listeners.insert(comp_listener);
            self.topic_listeners.insert(topic_filter, listeners);
            Ok(Some(subscription_id))
        }
    }

    /// Checks if a given listener is the last one registered for a topic filter.
    ///
    /// # Returns
    ///
    /// `true` if the set of registered listeners for the topic filter only contains
    /// the given listener.
    pub(crate) fn is_last_listener(
        &self,
        topic_filter: &str,
        listener: Arc<dyn up_rust::UListener>,
    ) -> bool {
        self.topic_listeners
            .get(topic_filter)
            .is_some_and(|registered_listeners| {
                registered_listeners.len() == 1
                    && registered_listeners.contains(&ComparableListener::new(listener))
            })
    }

    /// Removes a listener for an MQTT topic filter.
    ///
    /// # Arguments
    /// * `topic_filter` - The topic filter to remove the listener for.
    /// * `listener` - Listener to remove from the topic subscription list.
    ///
    /// # Returns
    ///
    /// `true` if the listener had been registered for the topic filter.
    pub(crate) fn remove_listener(
        &mut self,
        topic_filter: &str,
        listener: Arc<dyn up_rust::UListener>,
    ) -> bool {
        let Some(registered_listeners) = self.topic_listeners.get_mut(topic_filter) else {
            return false;
        };

        if !registered_listeners.remove(&ComparableListener::new(listener)) {
            return false;
        }

        if registered_listeners.is_empty() {
            // find subscription ID for topic filter and release the subscription ID
            if let Some(sub_id) = self.find_subscription_id(topic_filter) {
                self.release_subscription_id(sub_id, topic_filter);
            }
        }
        true
    }

    /// Determines listeners registered for subscription IDs.
    pub(crate) fn determine_listeners_for_topic(&self, topic: &str) -> HashSet<ComparableListener> {
        let mut listeners_to_invoke = HashSet::new();
        self.topic_listeners
            .matches(topic)
            .for_each(|(_topic_filter, listeners)| {
                listeners.iter().for_each(|listener| {
                    listeners_to_invoke.insert(listener.to_owned());
                });
            });
        listeners_to_invoke
    }

    /// Determines listeners registered for subscription IDs.
    pub(crate) fn determine_listeners_for_subscription_ids(
        &self,
        subscription_ids: &[SubscriptionIdentifier],
    ) -> HashSet<ComparableListener> {
        let mut listeners_to_invoke = HashSet::new();
        subscription_ids.iter().for_each(|sub_id| {
            if let Some(listeners) = self
                .subscription_topics
                .get(sub_id)
                .and_then(|topic_filter| self.topic_listeners.get(topic_filter))
            {
                listeners.iter().for_each(|listener| {
                    listeners_to_invoke.insert(listener.to_owned());
                });
            }
        });
        listeners_to_invoke
    }
}

pub(crate) trait SubscribedTopicProvider: Send + Sync {
    fn get_subscribed_topics(&self) -> HashMap<SubscriptionIdentifier, String>;
}

impl SubscribedTopicProvider for RegisteredListeners {
    fn get_subscribed_topics(&self) -> HashMap<SubscriptionIdentifier, String> {
        self.subscription_topics
            .iter()
            .map(|(subscription_id, topic_filter)| (*subscription_id, topic_filter.to_owned()))
            .collect()
    }
}

#[cfg(test)]
mod tests {

    use up_rust::MockUListener;

    use super::*;

    #[test]
    fn test_add_listener() {
        let topic_filter = "+/local_authority";
        let topic = "remote_authority/local_authority";
        let listener = Arc::new(MockUListener::new());
        let expected_listener = ComparableListener::new(listener.clone());
        let mut registered_listeners = RegisteredListeners::new(2);

        assert!(registered_listeners
            .determine_listeners_for_topic(topic)
            .is_empty());

        let subscription_id = registered_listeners
            .add_listener(topic_filter, listener.clone())
            .expect("Failed to register listener")
            .expect("Did not create new subscription ID");

        let listeners =
            registered_listeners.determine_listeners_for_subscription_ids(&[subscription_id]);
        assert!(listeners.len() == 1 && listeners.contains(&expected_listener));
        let listeners = registered_listeners.determine_listeners_for_topic(topic);
        assert!(listeners.len() == 1 && listeners.contains(&expected_listener));

        // [utest->dsn~utransport-registerlistener-idempotent~1]
        assert!(registered_listeners
            .add_listener(topic_filter, listener.clone())
            .expect("Failed to register listener")
            .is_none());
        assert!(registered_listeners
            .add_listener(topic_filter, listener.clone())
            .expect("Failed to register listener")
            .is_none());

        let listeners = registered_listeners.determine_listeners_for_topic(topic);
        assert!(listeners.len() == 1 && listeners.contains(&expected_listener));

        // [utest->dsn~utransport-registerlistener-number-of-listeners~1]
        let other_listener = Arc::new(MockUListener::new());
        let expected_other_listener = ComparableListener::new(other_listener.clone());
        assert_ne!(expected_listener, expected_other_listener);
        assert!(registered_listeners
            .add_listener(topic_filter, other_listener)
            .expect("Failed to register listener")
            .is_none());
        let listeners = registered_listeners.determine_listeners_for_topic(topic);
        assert!(
            listeners.len() == 2
                && listeners.contains(&expected_listener)
                && listeners.contains(&expected_other_listener)
        );
    }

    #[test]
    // [utest->dsn~utransport-registerlistener-listener-reuse~1]
    fn test_add_listener_supports_multi_topic_registration() {
        let topic_1 = "remote_authority_1/local_authority";
        let topic_2 = "remote_authority_2/local_authority";

        let listener = Arc::new(MockUListener::new());
        let expected_listener = ComparableListener::new(listener.clone());
        let mut registered_listeners = RegisteredListeners::new(2);

        assert!(registered_listeners
            .determine_listeners_for_topic(topic_1)
            .is_empty());
        assert!(registered_listeners
            .determine_listeners_for_topic(topic_2)
            .is_empty());

        let subscription_id_1 = registered_listeners
            .add_listener(topic_1, listener.clone())
            .expect("Failed to register listener")
            .expect("Did not create new subscription ID");
        let subscription_id_2 = registered_listeners
            .add_listener(topic_2, listener.clone())
            .expect("Failed to register listener")
            .expect("Did not create new subscription ID");

        let listeners = registered_listeners
            .determine_listeners_for_subscription_ids(&[subscription_id_1, subscription_id_2]);
        assert!(listeners.len() == 1 && listeners.contains(&expected_listener));
        let listeners = registered_listeners.determine_listeners_for_topic(topic_1);
        assert!(listeners.len() == 1 && listeners.contains(&expected_listener));
        let listeners = registered_listeners.determine_listeners_for_topic(topic_2);
        assert!(listeners.len() == 1 && listeners.contains(&expected_listener));
    }

    #[test]
    fn test_remove_listener() {
        let topic_filter = "+/local_authority";
        let listener_1 = Arc::new(MockUListener::new());
        let comparable_listener_1 = ComparableListener::new(listener_1.clone());
        let listener_2 = Arc::new(MockUListener::new());
        let comparable_listener_2 = ComparableListener::new(listener_2.clone());
        let mut registered_listeners = RegisteredListeners::new(10);

        let subscription_id = registered_listeners
            .add_listener(topic_filter, listener_1.clone())
            .expect("Failed to register listener 1")
            .expect("Did not create new subscription ID");

        assert!(registered_listeners
            .add_listener(topic_filter, listener_2.clone())
            .expect("Failed to register listener 2")
            .is_none());

        let listeners =
            registered_listeners.determine_listeners_for_subscription_ids(&[subscription_id]);
        assert!(
            listeners.len() == 2
                && listeners.contains(&comparable_listener_1)
                && listeners.contains(&comparable_listener_2)
        );

        assert!(!registered_listeners.is_last_listener(topic_filter, listener_1.clone()));
        assert!(registered_listeners.remove_listener(topic_filter, listener_1.clone()));

        let listeners =
            registered_listeners.determine_listeners_for_subscription_ids(&[subscription_id]);
        assert!(
            listeners.len() == 1
                && !listeners.contains(&comparable_listener_1)
                && listeners.contains(&comparable_listener_2)
        );

        assert!(registered_listeners.is_last_listener(topic_filter, listener_2.clone()));
        assert!(registered_listeners.remove_listener(topic_filter, listener_2.clone()));

        assert!(registered_listeners
            .determine_listeners_for_subscription_ids(&[subscription_id])
            .is_empty());

        assert!(!registered_listeners.remove_listener(topic_filter, listener_2.clone()));
    }

    #[test]
    fn test_get_free_subscription_id() {
        let mut registered_listeners = RegisteredListeners::new(2);

        let expected_vals: Vec<SubscriptionIdentifier> = registered_listeners
            .free_subscription_ids
            .iter()
            .cloned()
            .collect();
        let mut collected_vals = Vec::<u16>::new();

        let result = registered_listeners.get_free_subscription_id();

        assert!(result.is_ok());
        collected_vals.push(result.unwrap());

        let result = registered_listeners.get_free_subscription_id();

        assert!(result.is_ok());
        collected_vals.push(result.unwrap());

        assert!(collected_vals.len() == 2);
        assert!(collected_vals.iter().all(|x| expected_vals.contains(x)));
        assert!(registered_listeners.free_subscription_ids.is_empty());

        assert!(registered_listeners.get_free_subscription_id().is_err());
    }

    #[test]
    fn test_release_subscription_id() {
        let mut registered_listeners = RegisteredListeners::new(2);
        let subscription_id = registered_listeners
            .get_free_subscription_id()
            .expect("Failed to get subscription ID");
        assert!(!registered_listeners
            .free_subscription_ids
            .contains(&subscription_id));

        registered_listeners.release_subscription_id(subscription_id, "topic");

        assert!(registered_listeners
            .free_subscription_ids
            .contains(&subscription_id));
    }
}
