use std::time::Duration;

use http::{HeaderValue, Method, StatusCode, header};
use serde::Deserialize;

use super::{RetryPolicy, TwitchClient, TwitchError, success};
use crate::{app::commands::GameQuery, config::GameMetadata};

impl TwitchClient {
    pub async fn games(&self, query: &GameQuery) -> Result<Vec<GameMetadata>, TwitchError> {
        if !query.valid() {
            return Err(TwitchError::InvalidResponse);
        }
        tokio::time::timeout(Duration::from_secs(10), async {
            let _permit = tokio::select! { biased;
                _ = self.http.cancel.cancelled() => return Err(TwitchError::Cancelled),
                permit = self.http.concurrent.acquire() => permit.map_err(|_| TwitchError::Cancelled)?,
            };
            self.http.acquire().await?;
            let mut url = self.http.endpoints.helix.join(match query {
                GameQuery::Search(_) => "search/categories",
                GameQuery::Names(_) => "games",
            }).map_err(|_| TwitchError::Configuration)?;
            match query {
                GameQuery::Search(query) => { url.query_pairs_mut().append_pair("query", query.trim()).append_pair("first", "20"); }
                GameQuery::Names(names) => { for name in names { url.query_pairs_mut().append_pair("name", name); } }
            }
            let mut authorization = HeaderValue::from_str(&format!("Bearer {}", self.token()))
                .map_err(|_| TwitchError::Configuration)?;
            authorization.set_sensitive(true);
            let response = self.http.execute(
                self.http.request(Method::GET, url).header(header::AUTHORIZATION, authorization),
                RetryPolicy::Replay,
            ).await?;
            if matches!(response.status(), StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                return Err(TwitchError::Unauthorized);
            }
            success(response.status())?;
            #[derive(Deserialize)]
            struct Games { data: Vec<GameMetadata> }
            let games: Games = serde_json::from_slice(response.body()).map_err(|_| TwitchError::InvalidResponse)?;
            if games.data.len() > 100 || games.data.iter().any(|game| !game.valid()) {
                return Err(TwitchError::InvalidResponse);
            }
            let mut seen = std::collections::HashSet::new();
            Ok(games.data.into_iter().filter(|game| seen.insert(game.id.clone())).collect())
        }).await.map_err(|_| TwitchError::Network)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twitch::tests::{http, session};
    use serde_json::json;
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path, query_param},
    };

    #[tokio::test]
    async fn game_directory_uses_the_existing_token_and_returns_official_metadata() {
        let server = MockServer::start().await;
        let session = session();
        let client = TwitchClient::new(Arc::new(http(&server)), &session);
        let game = json!({"id":"33214", "name":"Fortnite", "box_art_url":"https://static-cdn.jtvnw.net/ttv-boxart/33214-52x72.jpg"});
        for (endpoint, field, value) in [
            ("/helix/search/categories", "query", "fort"),
            ("/helix/games", "name", "Fortnite"),
        ] {
            Mock::given(method("GET"))
                .and(path(endpoint))
                .and(query_param(field, value))
                .and(header("client-id", super::super::CLIENT_ID))
                .and(header(
                    "authorization",
                    format!("Bearer {}", session.access_token),
                ))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"data":[game.clone()]})),
                )
                .expect(1)
                .mount(&server)
                .await;
        }
        assert_eq!(
            client
                .games(&GameQuery::Search("fort".into()))
                .await
                .unwrap()[0]
                .name,
            "Fortnite"
        );
        assert_eq!(
            client
                .games(&GameQuery::Names(vec!["Fortnite".into()]))
                .await
                .unwrap()[0]
                .id,
            "33214"
        );
        assert!(client.games(&GameQuery::Search(" ".into())).await.is_err());
        assert!(!GameQuery::Names(vec!["name".into(); 101]).valid());
        client.http.cancel.cancel();
        assert_eq!(
            client
                .games(&GameQuery::Search("fort".into()))
                .await
                .unwrap_err(),
            TwitchError::Cancelled
        );
    }

    #[tokio::test]
    async fn game_directory_rejects_bad_metadata_and_preserves_auth_errors() {
        let server = MockServer::start().await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        for body in [
            json!({}),
            json!({"data":null}),
            json!({"data":[{"id":"1", "name":"Game", "box_art_url":"javascript:alert(1)"}]}),
        ] {
            server.reset().await;
            Mock::given(path("/helix/search/categories"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            assert_eq!(
                client
                    .games(&GameQuery::Search("game".into()))
                    .await
                    .unwrap_err(),
                TwitchError::InvalidResponse
            );
        }
        for status in [401, 403] {
            server.reset().await;
            Mock::given(path("/helix/search/categories"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            assert_eq!(
                client
                    .games(&GameQuery::Search("game".into()))
                    .await
                    .unwrap_err(),
                TwitchError::Unauthorized
            );
        }
    }
}
