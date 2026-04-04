use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{
    ChannelCreated, ChannelId, ChannelMessage, ChannelMessageConsumed, ChannelMessageSent,
    ChannelRecord, ProjectKey, RuntimeEvent,
};
use cairn_store::projections::ChannelReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::channels::ChannelService;
use crate::error::RuntimeError;

static CHANNEL_COUNTER: AtomicU64 = AtomicU64::new(1);
static CHANNEL_MESSAGE_COUNTER: AtomicU64 = AtomicU64::new(1);

pub struct ChannelServiceImpl<S> {
    store: Arc<S>,
}

impl<S> ChannelServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn next_channel_id() -> ChannelId {
    ChannelId::new(format!(
        "channel_{}",
        CHANNEL_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn next_message_id() -> String {
    format!(
        "channel_msg_{}",
        CHANNEL_MESSAGE_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

#[async_trait]
impl<S> ChannelService for ChannelServiceImpl<S>
where
    S: EventLog + ChannelReadModel + 'static,
{
    async fn create(
        &self,
        project: &ProjectKey,
        name: String,
        capacity: u32,
    ) -> Result<ChannelRecord, RuntimeError> {
        let channel_id = next_channel_id();
        let event = make_envelope(RuntimeEvent::ChannelCreated(ChannelCreated {
            channel_id: channel_id.clone(),
            project: project.clone(),
            name,
            capacity,
            created_at_ms: now_ms(),
        }));
        self.store.append(&[event]).await?;
        ChannelReadModel::get_channel(self.store.as_ref(), &channel_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("channel not found after create".to_owned()))
    }

    async fn get(&self, channel_id: &ChannelId) -> Result<Option<ChannelRecord>, RuntimeError> {
        Ok(ChannelReadModel::get_channel(self.store.as_ref(), channel_id).await?)
    }

    async fn list_channels(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ChannelRecord>, RuntimeError> {
        Ok(ChannelReadModel::list_channels(self.store.as_ref(), project, limit, offset).await?)
    }

    async fn send(
        &self,
        channel_id: &ChannelId,
        sender_id: String,
        body: String,
    ) -> Result<String, RuntimeError> {
        let channel = ChannelReadModel::get_channel(self.store.as_ref(), channel_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "channel",
                id: channel_id.to_string(),
            })?;
        let message_id = next_message_id();
        let event = make_envelope(RuntimeEvent::ChannelMessageSent(ChannelMessageSent {
            channel_id: channel.channel_id.clone(),
            project: channel.project,
            sender_id,
            message_id: message_id.clone(),
            body,
            sent_at_ms: now_ms(),
        }));
        self.store.append(&[event]).await?;
        Ok(message_id)
    }

    async fn consume(
        &self,
        channel_id: &ChannelId,
        consumer_id: String,
    ) -> Result<Option<ChannelMessage>, RuntimeError> {
        let channel = ChannelReadModel::get_channel(self.store.as_ref(), channel_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "channel",
                id: channel_id.to_string(),
            })?;
        let next_message =
            ChannelReadModel::list_messages(self.store.as_ref(), channel_id, usize::MAX)
                .await?
                .into_iter()
                .find(|message| message.consumed_at_ms.is_none());

        let Some(message) = next_message else {
            return Ok(None);
        };

        let consumed_at_ms = now_ms();
        let event = make_envelope(RuntimeEvent::ChannelMessageConsumed(
            ChannelMessageConsumed {
                channel_id: channel.channel_id.clone(),
                project: channel.project,
                message_id: message.message_id.clone(),
                consumed_by: consumer_id.clone(),
                consumed_at_ms,
            },
        ));
        self.store.append(&[event]).await?;

        Ok(Some(ChannelMessage {
            consumed_by: Some(consumer_id),
            consumed_at_ms: Some(consumed_at_ms),
            ..message
        }))
    }

    async fn list_messages(
        &self,
        channel_id: &ChannelId,
        limit: usize,
    ) -> Result<Vec<ChannelMessage>, RuntimeError> {
        if ChannelReadModel::get_channel(self.store.as_ref(), channel_id)
            .await?
            .is_none()
        {
            return Err(RuntimeError::NotFound {
                entity: "channel",
                id: channel_id.to_string(),
            });
        }
        Ok(ChannelReadModel::list_messages(self.store.as_ref(), channel_id, limit).await?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::ProjectKey;
    use cairn_store::InMemoryStore;

    use crate::channels::ChannelService;

    use super::ChannelServiceImpl;

    #[tokio::test]
    async fn channel_create_send_consume_flow() {
        let store = Arc::new(InMemoryStore::new());
        let service = ChannelServiceImpl::new(store);
        let project = ProjectKey::new("tenant_acme", "ws_main", "project_alpha");

        let channel = service
            .create(&project, "ops".to_owned(), 10)
            .await
            .unwrap();
        assert_eq!(channel.name, "ops");

        service
            .send(&channel.channel_id, "alice".to_owned(), "one".to_owned())
            .await
            .unwrap();
        service
            .send(&channel.channel_id, "alice".to_owned(), "two".to_owned())
            .await
            .unwrap();
        service
            .send(&channel.channel_id, "alice".to_owned(), "three".to_owned())
            .await
            .unwrap();

        let consumed_1 = service
            .consume(&channel.channel_id, "bob".to_owned())
            .await
            .unwrap()
            .unwrap();
        let consumed_2 = service
            .consume(&channel.channel_id, "bob".to_owned())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(consumed_1.body, "one");
        assert_eq!(consumed_2.body, "two");

        let remaining = service
            .list_messages(&channel.channel_id, 10)
            .await
            .unwrap();
        assert_eq!(
            remaining
                .iter()
                .filter(|message| message.consumed_at_ms.is_none())
                .count(),
            1
        );
    }
}
