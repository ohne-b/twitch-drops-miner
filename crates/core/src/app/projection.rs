use serde::Serialize;

use crate::dto::*;

/// Each patch is computed from the last state delivered to this subscriber.
/// A coalesced publication therefore never creates a false revision gap.
#[derive(Serialize)]
pub struct StatePatch {
    pub protocol: u32,
    pub instance: String,
    pub base_revision: u64,
    pub revision: u64,
    pub changes: Changes,
}

macro_rules! changes {
    ($($field:ident: $ty:ty),+ $(,)?) => {
        #[derive(Serialize, Default)]
        pub struct Changes {
            $(#[serde(skip_serializing_if = "Option::is_none")] pub $field: Option<$ty>,)+
        }
        impl StatePatch {
            pub fn between(previous: &Snapshot, next: &Snapshot) -> Self {
                Self {
                    protocol: super::state::PROTOCOL,
                    instance: next.instance.clone(), base_revision: previous.revision,
                    revision: next.revision,
                    changes: Changes { $($field: (previous.$field != next.$field).then(|| next.$field.clone()),)+ },
                }
            }
        }
    };
}

changes! {
    mining: MiningStatus,
    status: String,
    campaigns: Vec<CampaignView>,
    channels: Vec<ChannelView>,
    console: Vec<String>,
    activity: Vec<super::activity::ActivityEvent>,
    settings: SettingsView,
    login: Login,
    manual_mode: ManualMode,
    current_drop: Option<Progress>,
    wanted_items: Vec<WantedGame>,
    inventory_status: InventoryStatus,
    inventory_refresh: InventoryRefresh,
    history_revision: u64,
    history_clear_revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewer_patch_does_not_rebuild_campaigns_and_null_clears_progress() {
        let mut previous = Snapshot::default();
        previous.channels.push(ChannelView::default());
        previous.current_drop = Some(Progress {
            drop_id: "drop".into(),
            drop_name: "Reward".into(),
            campaign_id: "campaign".into(),
            campaign_name: "Campaign".into(),
            game_name: "Game".into(),
            current_minutes: 1,
            confirmed_minutes: 1,
            confirmed_at: None,
            required_minutes: 20,
            progress: 5.0,
            remaining_seconds: 1140,
        });
        let mut next = previous.clone();
        next.channels[0].viewers = Some(42);
        next.current_drop = None;
        next.revision = 4;
        let patch = serde_json::to_value(StatePatch::between(&previous, &next)).unwrap();
        assert!(patch["changes"].get("campaigns").is_none());
        assert_eq!(patch["changes"]["channels"][0]["viewers"], 42);
        assert_eq!(
            patch["changes"].get("current_drop"),
            Some(&serde_json::Value::Null)
        );
    }
}
