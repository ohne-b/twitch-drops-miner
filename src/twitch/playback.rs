use std::{collections::VecDeque, time::Duration};

use http::{Method, StatusCode, header};
use serde_json::json;
use url::Url;

use super::{
    TwitchClient, TwitchError, TwitchHttp, USER_AGENT, channels::channel_login, diagnostics,
    operations::Operation, success,
};
use crate::domain::Channel;

pub(crate) const POLL_INTERVAL: Duration = Duration::from_secs(10);
const MAX_PLAYLIST: usize = 512 * 1024;
const MAX_SEGMENTS: usize = 256;

// Signed stream addresses stay in this network generation, never snapshots or storage.
#[derive(Clone)]
pub(crate) struct Playback {
    pub channel: u64,
    broadcast: String,
    login: String,
    playlist: Option<Url>,
    seen: VecDeque<Url>,
}

impl Playback {
    pub fn new(channel: &Channel) -> Option<Self> {
        Some(Self {
            channel: channel.identity.id,
            broadcast: channel.broadcast_id.clone()?,
            login: channel_login(&channel.identity.login)?,
            playlist: None,
            seen: VecDeque::new(),
        })
    }

    pub fn matches(&self, channel: &Channel) -> bool {
        self.channel == channel.identity.id
            && channel.broadcast_id.as_ref() == Some(&self.broadcast)
            && self.login == channel.identity.login
    }

    pub fn retain_checks(&mut self, other: &Self) {
        if self.channel == other.channel
            && self.broadcast == other.broadcast
            && self.login == other.login
        {
            for segment in &other.seen {
                self.remember(segment.clone());
            }
        }
    }

    fn remember(&mut self, segment: Url) {
        if self.seen.contains(&segment) {
            return;
        }
        if self.seen.len() == MAX_SEGMENTS {
            self.seen.pop_front();
        }
        self.seen.push_back(segment);
    }

    pub async fn poll(&mut self, client: &TwitchClient) -> Result<(), TwitchError> {
        tokio::time::timeout(POLL_INTERVAL, self.poll_inner(client))
            .await
            .unwrap_or(Err(TwitchError::Network))
    }

    async fn poll_inner(&mut self, client: &TwitchClient) -> Result<(), TwitchError> {
        if self.playlist.is_none() {
            let response = client
                .gql(Operation::PlaybackAccessToken.request(json!({"login":self.login})))
                .await?;
            let token = &response["data"]["streamPlaybackAccessToken"];
            let value = token["value"]
                .as_str()
                .filter(|v| !v.is_empty() && v.len() <= 16 * 1024)
                .ok_or(TwitchError::InvalidResponse)?;
            let signature = token["signature"]
                .as_str()
                .filter(|v| !v.is_empty() && v.len() <= 1024)
                .ok_or(TwitchError::InvalidResponse)?;
            let mut master = client
                .http
                .endpoints
                .usher
                .join(&format!("api/channel/hls/{}.m3u8", self.login))
                .map_err(|_| TwitchError::InvalidResponse)?;
            master
                .query_pairs_mut()
                .append_pair("sig", signature)
                .append_pair("token", value);
            let body = client
                .http
                .stream_request(Method::GET, master.clone())
                .await?;
            self.playlist = playlist_urls(&body, &master, true, &client.http)?
                .into_iter()
                .next();
        }
        let url = self
            .playlist
            .as_ref()
            .ok_or(TwitchError::InvalidResponse)?
            .clone();
        let body = match client.http.stream_request(Method::GET, url.clone()).await {
            Ok(body) => body,
            Err(error) => {
                if matches!(error, TwitchError::Status(401 | 403 | 404)) {
                    self.playlist = None;
                }
                return Err(error);
            }
        };
        let segments = match playlist_urls(&body, &url, false, &client.http) {
            Ok(segments) => segments,
            Err(error) => {
                self.playlist = None;
                return Err(error);
            }
        };
        let mut failure = None;
        for segment in segments {
            if self.seen.contains(&segment) {
                continue;
            }
            match client
                .http
                .stream_request(Method::HEAD, segment.clone())
                .await
            {
                Ok(_) => {
                    self.remember(segment);
                }
                Err(TwitchError::Cancelled) => return Err(TwitchError::Cancelled),
                Err(error) => {
                    if matches!(error, TwitchError::Status(401 | 403 | 404)) {
                        self.playlist = None;
                    }
                    failure = Some(error);
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

impl TwitchHttp {
    fn stream_url(&self, url: &Url) -> bool {
        if !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.as_str().len() > 32 * 1024
        {
            return false;
        }
        #[cfg(test)]
        if url.origin() == self.endpoints.usher.origin() {
            return true;
        }
        url.scheme() == "https"
            && url.port().is_none()
            && url.host_str().is_some_and(|host| {
                ["ttvnw.net", "twitch.tv", "jtvnw.net"]
                    .iter()
                    .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
            })
    }

    #[tracing::instrument(skip_all, fields(operation = "StreamPlayback", method = %method))]
    async fn stream_request(&self, method: Method, url: Url) -> Result<String, TwitchError> {
        if !self.stream_url(&url) || method == Method::GET && !url.path().ends_with(".m3u8") {
            return Err(TwitchError::InvalidResponse);
        }
        self.acquire().await?;
        let head = method == Method::HEAD;
        let request = self
            .catalog_client
            .request(method, url)
            .header(header::USER_AGENT, USER_AGENT)
            .timeout(Duration::from_secs(if head { 3 } else { 5 }));
        let mut response = tokio::select! {biased;
            _ = self.cancel.cancelled() => return Err(TwitchError::Cancelled),
            result = request.send() => result.map_err(|e| {
                // Signed URLs can occur in nested errors: retain typed causes only.
                diagnostics::network(&e, "stream_send", 1);
                TwitchError::Network
            })?,
        };
        tracing::debug!(target: "tdm_diagnostics",
            status = response.status().as_u16(),
            "Stream playback response"
        );
        success(response.status())?;
        if head {
            return Ok(String::new());
        }
        if response.status() != StatusCode::OK
            || response
                .content_length()
                .is_some_and(|n| n > MAX_PLAYLIST as u64)
            || !response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| {
                    matches!(
                        v.split(';')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_ascii_lowercase()
                            .as_str(),
                        "application/vnd.apple.mpegurl"
                            | "application/x-mpegurl"
                            | "audio/mpegurl"
                            | "audio/x-mpegurl"
                            | "text/plain"
                    )
                })
        {
            return Err(TwitchError::InvalidResponse);
        }
        let mut bytes = Vec::new();
        loop {
            let chunk = tokio::select! {biased;
                _ = self.cancel.cancelled() => return Err(TwitchError::Cancelled),
                result = response.chunk() => result.map_err(|e| {
                    diagnostics::network(&e, "stream_body", 1);
                    TwitchError::Network
                })?,
            };
            let Some(chunk) = chunk else { break };
            if bytes.len() + chunk.len() > MAX_PLAYLIST {
                return Err(TwitchError::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).map_err(|_| TwitchError::InvalidResponse)
    }
}

fn playlist_urls(
    body: &str,
    base: &Url,
    master: bool,
    http: &TwitchHttp,
) -> Result<Vec<Url>, TwitchError> {
    let mut lines = body.trim().lines();
    if body.len() > MAX_PLAYLIST || lines.next() != Some("#EXTM3U") {
        return Err(TwitchError::InvalidResponse);
    }
    let mut pending = None;
    let mut entries = Vec::new();
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let bandwidth = if let Some(attrs) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            if !master {
                return Err(TwitchError::InvalidResponse);
            }
            Some(attrs.split(',').find_map(|a| {
                a.strip_prefix("BANDWIDTH=")
                    .and_then(|v| v.parse::<u64>().ok())
            }))
        } else if line.starts_with("#EXTINF:") {
            if master {
                return Err(TwitchError::InvalidResponse);
            }
            Some(Some(0))
        } else {
            None
        };
        if let Some(bandwidth) = bandwidth {
            if pending.is_some() {
                return Err(TwitchError::InvalidResponse);
            }
            pending = Some(bandwidth.ok_or(TwitchError::InvalidResponse)?);
        } else if !line.starts_with('#') {
            let bandwidth = pending.take().ok_or(TwitchError::InvalidResponse)?;
            let url = base.join(line).map_err(|_| TwitchError::InvalidResponse)?;
            if !http.stream_url(&url)
                || master && !url.path().ends_with(".m3u8")
                || entries.len() == MAX_SEGMENTS
            {
                return Err(TwitchError::InvalidResponse);
            }
            entries.push((bandwidth, url));
        }
    }
    if pending.is_some() || entries.is_empty() {
        return Err(TwitchError::InvalidResponse);
    }
    if master {
        entries.sort_by_key(|(bandwidth, _)| *bandwidth);
    }
    Ok(entries.into_iter().map(|(_, url)| url).collect())
}

#[cfg(test)]
mod tests;
