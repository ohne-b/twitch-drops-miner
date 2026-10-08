use serde_json::{Value, json};

#[derive(Clone, Copy)]
pub enum Operation {
    Inventory,
    GameDirectory,
    StreamInfo,
    CurrentDrop,
    PlaybackAccessToken,
    ClaimDrop,
    AvailableDrops,
    DeleteNotification,
}

impl Operation {
    pub fn request(self, mut variables: Value) -> Value {
        match self {
            Self::Inventory => variables["fetchRewardCampaigns"] = false.into(),
            Self::CurrentDrop => variables["channelLogin"] = "".into(),
            Self::PlaybackAccessToken => {
                variables["isLive"] = true.into();
                variables["isVod"] = false.into();
                variables["vodID"] = "".into();
                variables["platform"] = "web".into();
                variables["playerType"] = "site".into();
            }
            _ => {}
        }
        let (name, hash) = match self {
            Self::Inventory => (
                "Inventory",
                "d86775d0ef16a63a33ad52e80eaff963b2d5b72fada7c991504a57496e1d8e4b",
            ),
            Self::GameDirectory => (
                "DirectoryPage_Game",
                "cb5dc816e139dcb8a118f14b4b677d59abc224a4b016c4bc2bb00a47fe0ddec4",
            ),
            Self::StreamInfo => (
                "VideoPlayerStreamInfoOverlayChannel",
                "198492e0857f6aedead9665c81c5a06d67b25b58034649687124083ff288597d",
            ),
            Self::CurrentDrop => (
                "DropCurrentSessionContext",
                "4d06b702d25d652afb9ef835d2a550031f1cf762b193523a92166f40ea3d142b",
            ),
            Self::PlaybackAccessToken => (
                "PlaybackAccessToken",
                "ed230aa1e33e07eebb8928504583da78a5173989fadfb1ac94be06a04f3cdbe9",
            ),
            Self::ClaimDrop => (
                "DropsPage_ClaimDropRewards",
                "a455deea71bdc9015b78eb49f4acfbce8baa7ccbedd28e549bb025bd0f751930",
            ),
            Self::AvailableDrops => (
                "DropsHighlightService_AvailableDrops",
                "9a62a09bce5b53e26e64a671e530bc599cb6aab1e5ba3cbd5d85966d3940716f",
            ),
            Self::DeleteNotification => (
                "OnsiteNotifications_DeleteNotification",
                "13d463c831f28ffe17dccf55b3148ed8b3edbbd0ebadd56352f1ff0160616816",
            ),
        };
        json!({"operationName":name,"variables":variables,"extensions":{"persistedQuery":{"version":1,"sha256Hash":hash}}})
    }
}

pub fn directory(slug: &str, limit: usize) -> Value {
    Operation::GameDirectory.request(json!({"limit":limit,"slug":slug,"imageWidth":50,"includeCostreaming":false,
        "options":{"broadcasterLanguages":[],"freeformTags":null,"includeRestricted":["SUB_ONLY_LIVE"],"recommendationsContext":{"platform":"web"},
            "sort":"RELEVANCE","systemFilters":["DROPS_ENABLED"],"tags":[],"requestID":"JIRA-VXP-2397"},"sortTypeIsRecency":false}))
}

pub(super) fn can_replay_response(request: &Value) -> bool {
    if let Some(batch) = request.as_array() {
        return !batch.is_empty() && batch.iter().all(can_replay_response);
    }
    request.get("query").is_none()
        && [
            Operation::Inventory,
            Operation::GameDirectory,
            Operation::StreamInfo,
            Operation::CurrentDrop,
            Operation::PlaybackAccessToken,
            Operation::AvailableDrops,
        ]
        .into_iter()
        .any(|operation| {
            let known = operation.request(json!({}));
            request["operationName"] == known["operationName"]
                && request["extensions"]["persistedQuery"] == known["extensions"]["persistedQuery"]
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_replay_requires_known_read_operations_and_safe_batches() {
        for (operation, replay) in [
            (Operation::Inventory, true),
            (Operation::GameDirectory, true),
            (Operation::StreamInfo, true),
            (Operation::CurrentDrop, true),
            (Operation::PlaybackAccessToken, true),
            (Operation::AvailableDrops, true),
            (Operation::ClaimDrop, false),
            (Operation::DeleteNotification, false),
        ] {
            assert_eq!(can_replay_response(&operation.request(json!({}))), replay);
        }
        let read = Operation::Inventory.request(json!({}));
        assert!(can_replay_response(&json!([read.clone(), read.clone()])));
        assert!(!can_replay_response(&json!([
            read.clone(),
            Operation::ClaimDrop.request(json!({}))
        ])));
        assert!(!can_replay_response(&json!([])));
        let mut forged = read;
        forged["extensions"]["persistedQuery"]["sha256Hash"] = json!("unknown");
        assert!(!can_replay_response(&forged));
    }

    #[test]
    fn persisted_operation_contracts_include_all_required_variables() {
        for (operation, input, name, expected) in [
            (
                Operation::Inventory,
                json!({}),
                "Inventory",
                json!({"fetchRewardCampaigns":false}),
            ),
            (
                Operation::CurrentDrop,
                json!({"channelID":"10"}),
                "DropCurrentSessionContext",
                json!({"channelID":"10","channelLogin":""}),
            ),
            (
                Operation::StreamInfo,
                json!({"channel":"streamer"}),
                "VideoPlayerStreamInfoOverlayChannel",
                json!({"channel":"streamer"}),
            ),
            (
                Operation::ClaimDrop,
                json!({"input":{"dropInstanceID":"earned"}}),
                "DropsPage_ClaimDropRewards",
                json!({"input":{"dropInstanceID":"earned"}}),
            ),
            (
                Operation::AvailableDrops,
                json!({"channelID":"10"}),
                "DropsHighlightService_AvailableDrops",
                json!({"channelID":"10"}),
            ),
            (
                Operation::DeleteNotification,
                json!({"input":{"id":"notification"}}),
                "OnsiteNotifications_DeleteNotification",
                json!({"input":{"id":"notification"}}),
            ),
        ] {
            let request = operation.request(input);
            assert_eq!(request["operationName"], name);
            assert_eq!(request["variables"], expected);
            assert_eq!(request["extensions"]["persistedQuery"]["version"], 1);
            assert_eq!(
                request["extensions"]["persistedQuery"]["sha256Hash"]
                    .as_str()
                    .unwrap()
                    .len(),
                64
            );
        }
        let request = directory("rust", 20);
        assert_eq!(request["operationName"], "DirectoryPage_Game");
        assert_eq!(
            request["variables"],
            json!({"limit":20,"slug":"rust","imageWidth":50,"includeCostreaming":false,
            "options":{"broadcasterLanguages":[],"freeformTags":null,"includeRestricted":["SUB_ONLY_LIVE"],"recommendationsContext":{"platform":"web"},"sort":"RELEVANCE","systemFilters":["DROPS_ENABLED"],"tags":[],"requestID":"JIRA-VXP-2397"},"sortTypeIsRecency":false})
        );
    }
}
