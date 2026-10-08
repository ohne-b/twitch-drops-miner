//! Version-one durable evidence. Presentation additions never enter these files.
use crate::dto::{BenefitView, CampaignView, DropView};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct StoredBenefit {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub image_url: String,
}

impl From<BenefitView> for StoredBenefit {
    fn from(value: BenefitView) -> Self {
        Self {
            name: value.name,
            kind: value.kind,
            image_url: value.image_url,
        }
    }
}

impl From<StoredBenefit> for BenefitView {
    fn from(value: StoredBenefit) -> Self {
        Self {
            name: value.name,
            kind: value.kind,
            image_url: value.image_url,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StoredDrop {
    pub id: String,
    pub name: String,
    pub current_minutes: u32,
    #[serde(default)]
    pub confirmed_minutes: u32,
    #[serde(default)]
    pub confirmed_at: Option<DateTime<Utc>>,
    pub required_minutes: u32,
    pub progress: f64,
    pub is_claimed: bool,
    pub can_claim: bool,
    pub is_ignored: bool,
    pub is_mineable: bool,
    pub is_skipped: bool,
    pub ignored_reason: Option<String>,
    pub ignored_keyword: Option<String>,
    pub ignored_precondition: Option<String>,
    pub benefits: Vec<StoredBenefit>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

impl From<DropView> for StoredDrop {
    fn from(value: DropView) -> Self {
        Self {
            id: value.id,
            name: value.name,
            current_minutes: value.current_minutes,
            confirmed_minutes: value.confirmed_minutes,
            confirmed_at: value.confirmed_at,
            required_minutes: value.required_minutes,
            progress: value.progress,
            is_claimed: value.is_claimed,
            can_claim: value.can_claim,
            is_ignored: value.is_ignored,
            is_mineable: value.is_mineable,
            is_skipped: value.is_skipped,
            ignored_reason: value.ignored_reason,
            ignored_keyword: value.ignored_keyword,
            ignored_precondition: value.ignored_precondition,
            benefits: value.benefits.into_iter().map(Into::into).collect(),
            starts_at: value.starts_at,
            ends_at: value.ends_at,
        }
    }
}

impl From<StoredDrop> for DropView {
    fn from(value: StoredDrop) -> Self {
        Self {
            id: value.id,
            name: value.name,
            current_minutes: value.current_minutes,
            confirmed_minutes: value.confirmed_minutes,
            confirmed_at: value.confirmed_at,
            required_minutes: value.required_minutes,
            progress: value.progress,
            is_claimed: value.is_claimed,
            can_claim: value.can_claim,
            is_ignored: value.is_ignored,
            is_mineable: value.is_mineable,
            is_skipped: value.is_skipped,
            ignored_reason: value.ignored_reason,
            ignored_keyword: value.ignored_keyword,
            ignored_precondition: value.ignored_precondition,
            benefits: value.benefits.into_iter().map(Into::into).collect(),
            starts_at: value.starts_at,
            ends_at: value.ends_at,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StoredCampaign {
    pub id: String,
    pub name: String,
    pub game_name: String,
    pub game_box_art_url: String,
    pub campaign_url: String,
    pub link_url: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub linked: Option<bool>,
    pub active: bool,
    pub upcoming: bool,
    pub expired: bool,
    pub finished: bool,
    #[serde(default)]
    pub mining_finished: bool,
    pub claimed_drops: usize,
    pub total_drops: usize,
    pub ignored_drops: usize,
    pub skipped_drops: usize,
    pub drops: Vec<StoredDrop>,
}

impl From<CampaignView> for StoredCampaign {
    fn from(value: CampaignView) -> Self {
        Self {
            id: value.id,
            name: value.name,
            game_name: value.game_name,
            game_box_art_url: value.game_box_art_url,
            campaign_url: value.campaign_url,
            link_url: value.link_url,
            starts_at: value.starts_at,
            ends_at: value.ends_at,
            linked: value.linked,
            active: value.active,
            upcoming: value.upcoming,
            expired: value.expired,
            finished: value.finished,
            mining_finished: value.mining_finished,
            claimed_drops: value.claimed_drops,
            total_drops: value.total_drops,
            ignored_drops: value.ignored_drops,
            skipped_drops: value.skipped_drops,
            drops: value.drops.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<StoredCampaign> for CampaignView {
    fn from(value: StoredCampaign) -> Self {
        Self {
            id: value.id,
            name: value.name,
            game_name: value.game_name,
            game_box_art_url: value.game_box_art_url,
            campaign_url: value.campaign_url,
            link_url: value.link_url,
            starts_at: value.starts_at,
            ends_at: value.ends_at,
            linked: value.linked,
            active: value.active,
            upcoming: value.upcoming,
            expired: value.expired,
            finished: value.finished,
            mining_finished: value.mining_finished,
            claimed_drops: value.claimed_drops,
            total_drops: value.total_drops,
            ignored_drops: value.ignored_drops,
            skipped_drops: value.skipped_drops,
            drops: value.drops.into_iter().map(Into::into).collect(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_one_records_exclude_runtime_projection_and_round_trip() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../../frontend/tests/fixture.json")).unwrap();
        let mut campaign: CampaignView =
            serde_json::from_value(fixture["campaigns"][0].clone()).unwrap();
        campaign.game_key = "rust".into();
        campaign.selected = true;
        campaign.drops[0].prerequisites = vec!["runtime-only".into()];
        let frozen = StoredCampaign::from(campaign);
        let json = serde_json::to_value(&frozen).unwrap();
        for key in [
            "game_key",
            "selected",
            "saved_rank",
            "priority",
            "allowed_channels",
        ] {
            assert!(json.get(key).is_none(), "{key}");
        }
        for key in [
            "eligibility",
            "prerequisites",
            "effective_starts_at",
            "effective_ends_at",
            "priority_deadline",
        ] {
            assert!(json["drops"][0].get(key).is_none(), "{key}");
        }
        let restored: StoredCampaign = serde_json::from_value(json).unwrap();
        let view: CampaignView = restored.into();
        assert!(view.drops[0].prerequisites.is_empty());
        assert_eq!(StoredCampaign::from(view), frozen);
    }
}
