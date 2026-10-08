use std::ops::{Deref, DerefMut};

use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard, watch};

use crate::{dto::Snapshot, random_hex};

pub const PROTOCOL: u32 = 2;

/// A single publication boundary for related fields. Subscribers retain only the
/// latest revision, so slow browsers cannot accumulate an unbounded event queue.
pub struct PublishedState {
    value: RwLock<Snapshot>,
    changed: watch::Sender<u64>,
    instance: String,
}

impl PublishedState {
    pub fn new(mut value: Snapshot) -> anyhow::Result<Self> {
        let instance = random_hex::<16>()?;
        value.protocol = PROTOCOL;
        value.instance = instance.clone();
        value.revision = 1;
        Ok(Self {
            value: RwLock::new(value),
            changed: watch::channel(1).0,
            instance,
        })
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub async fn read(&self) -> RwLockReadGuard<'_, Snapshot> {
        self.value.read().await
    }

    pub async fn write(&self) -> Publication<'_> {
        Publication {
            value: self.value.write().await,
            owner: self,
        }
    }

    pub fn blocking_write(&self) -> Publication<'_> {
        Publication {
            value: self.value.blocking_write(),
            owner: self,
        }
    }
}

pub struct Publication<'a> {
    value: RwLockWriteGuard<'a, Snapshot>,
    owner: &'a PublishedState,
}

impl Deref for Publication<'_> {
    type Target = Snapshot;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}
impl DerefMut for Publication<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}
impl Drop for Publication<'_> {
    fn drop(&mut self) {
        self.value.protocol = PROTOCOL;
        self.value.instance.clone_from(&self.owner.instance);
        self.owner.changed.send_modify(|revision| {
            *revision += 1;
            self.value.revision = *revision;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publication_is_atomic_coalesced_and_keeps_process_identity() {
        let state = PublishedState::new(Snapshot::default()).unwrap();
        let mut observer = state.subscribe();
        let instance = state.read().await.instance.clone();
        {
            let mut value = state.write().await;
            value.status = "watching".into();
            value.history_clear_revision = 1;
            assert!(!observer.has_changed().unwrap());
        }
        state.write().await.status = "idle".into();
        observer.changed().await.unwrap();
        let value = state.read().await;
        assert_eq!(value.revision, 3);
        assert_eq!(value.history_clear_revision, 1);
        assert_eq!(value.status, "idle");
        assert_eq!(value.instance, instance);
    }
}
