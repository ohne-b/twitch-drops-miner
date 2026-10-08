use std::{
    collections::{BTreeSet, HashMap},
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use reqwest_websocket::{Message, Upgrade, WebSocket};
use serde_json::{Value, json};
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use super::{TwitchClient, TwitchError, channels::MAX_CHANNELS};
use crate::domain::number;

const SHARDS: usize = 8;
const TOPICS_PER_SOCKET: usize = 50;
const PING_INTERVAL: Duration = Duration::from_secs(180);
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::config::Settings;
    use crate::twitch::{Endpoints, TwitchHttp, tests::session};
    use axum::{
        Router,
        extract::ws::{Message as AxumMessage, WebSocketUpgrade},
        routing::get,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    async fn check(mode: &'static str) {
        let connections = Arc::new(AtomicUsize::new(0));
        let connected = connections.clone();
        let router=Router::new().route("/pubsub",get(move |upgrade:WebSocketUpgrade| {
            let connected=connected.clone();
            async move {upgrade.on_upgrade(move |mut socket|async move {
                let generation=connected.fetch_add(1,Ordering::SeqCst);
                while let Some(Ok(AxumMessage::Text(text)))=socket.recv().await {
                    let value:Value=serde_json::from_str(&text).unwrap();
                    if value["type"]=="PING" {
                        if mode!="no_pong" && socket.send(AxumMessage::Text(json!({"type":"PONG"}).to_string().into())).await.is_err(){break;}
                        continue;
                    }
                    if mode!="no_ack" {
                        let error=if mode=="bad_auth" {"ERR_BADAUTH"}else{""};
                        if socket.send(AxumMessage::Text(json!({"type":"RESPONSE","nonce":value["nonce"],"error":error}).to_string().into())).await.is_err(){break;}
                    }
                    if mode=="reconnect" && generation==0 {
                        let _=socket.send(AxumMessage::Text(json!({"type":"RECONNECT"}).to_string().into())).await;
                    }
                    if mode=="proxy" || mode=="full_queue" || mode=="reconnect" && generation>0 {
                        for minutes in 1..=3 {
                            let payload=json!({"type":"drop-progress","data":{"drop_id":"reward","current_progress_min":minutes}}).to_string();
                            let message=json!({"type":"MESSAGE","data":{"topic":"user-drop-events.42","message":payload}});
                            if socket.send(AxumMessage::Text(message.to_string().into())).await.is_err(){break;}
                        }
                    }
                }
            })}
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let shutdown = CancellationToken::new();
        let stopped = shutdown.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(stopped.cancelled_owned())
                .await
                .unwrap();
        });
        let mut settings = Settings::default();
        if mode == "proxy" {
            settings.proxy = base.clone();
        }
        let endpoints = if mode == "proxy" {
            Endpoints::mock("http://upstream.invalid")
        } else {
            Endpoints::mock(&base)
        };
        let http = TwitchHttp::build(
            &settings,
            Some("testdevice"),
            CancellationToken::new(),
            endpoints,
        )
        .unwrap();
        let client = TwitchClient::new(Arc::new(http), &session());
        let (events, mut received) = mpsc::channel(1);
        let mut pool = PubSub::start(client, events);
        match mode {
            "bad_auth" => {
                let event = tokio::time::timeout(Duration::from_secs(5), received.recv())
                    .await
                    .unwrap()
                    .unwrap();
                assert!(matches!(event, Event::Unauthorized));
            }
            "proxy" | "reconnect" => {
                let event = tokio::time::timeout(Duration::from_secs(5), received.recv())
                    .await
                    .unwrap()
                    .unwrap();
                assert!(matches!(event, Event::Progress { minutes: 1, .. }));
                assert!(connections.load(Ordering::SeqCst) >= if mode == "proxy" { 1 } else { 2 });
            }
            "no_pong" | "no_ack" => {
                tokio::time::timeout(Duration::from_secs(14), async {
                    while connections.load(Ordering::SeqCst) < 2 {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
                .unwrap();
            }
            "full_queue" => {
                tokio::time::timeout(Duration::from_secs(5), async {
                    while received.is_empty() {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            _ => unreachable!(),
        }
        tokio::time::timeout(Duration::from_secs(1), pool.close())
            .await
            .unwrap();
        assert!(pool.tasks.is_empty());
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn pubsub_uses_configured_http_proxy() {
        check("proxy").await;
    }
    #[tokio::test]
    async fn pubsub_reconnect_resubscribes() {
        check("reconnect").await;
    }
    #[tokio::test]
    async fn pubsub_bad_auth_reaches_owner() {
        check("bad_auth").await;
    }
    #[tokio::test]
    async fn pubsub_missing_ack_reconnects() {
        check("no_ack").await;
    }
    #[tokio::test]
    async fn pubsub_missing_pong_reconnects() {
        check("no_pong").await;
    }
    #[tokio::test]
    async fn pubsub_full_event_queue_does_not_prevent_close() {
        check("full_queue").await;
    }
}

pub enum Event {
    Progress { id: String, minutes: u32 },
    Claim { id: String, instance: String },
    Notification(String),
    Offline(u64),
    Changed(u64),
    Viewers { id: u64, count: u64 },
    Unauthorized,
}

pub struct PubSub {
    user_id: u64,
    topics: Vec<watch::Sender<BTreeSet<String>>>,
    cancel: CancellationToken,
    tasks: JoinSet<()>,
}

impl PubSub {
    pub fn start(client: TwitchClient, events: mpsc::Sender<Event>) -> Self {
        let cancel = client.http.cancel.child_token();
        let mut tasks = JoinSet::new();
        let topics = (0..SHARDS)
            .map(|_| {
                let (send, receive) = watch::channel(BTreeSet::new());
                tasks.spawn(worker(
                    client.clone(),
                    receive,
                    events.clone(),
                    cancel.clone(),
                ));
                send
            })
            .collect();
        let pool = Self {
            user_id: client.user_id,
            topics,
            cancel,
            tasks,
        };
        pool.set_channels(&[]);
        pool
    }
    pub fn set_channels(&self, channels: &[u64]) {
        let mut topics = vec![
            format!("user-drop-events.{}", self.user_id),
            format!("onsite-notifications.{}", self.user_id),
        ];
        for id in channels.iter().take(MAX_CHANNELS).collect::<BTreeSet<_>>() {
            topics.push(format!("video-playback-by-id.{id}"));
            topics.push(format!("broadcast-settings-update.{id}"));
        }
        for (index, sender) in self.topics.iter().enumerate() {
            let desired: BTreeSet<_> = topics
                .iter()
                .skip(index * TOPICS_PER_SOCKET)
                .take(TOPICS_PER_SOCKET)
                .cloned()
                .collect();
            sender.send_if_modified(|current| {
                if *current == desired {
                    false
                } else {
                    *current = desired;
                    true
                }
            });
        }
    }
    pub async fn close(&mut self) {
        self.cancel.cancel();
        while self.tasks.join_next().await.is_some() {}
    }
}
impl Drop for PubSub {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn worker(
    client: TwitchClient,
    mut desired: watch::Receiver<BTreeSet<String>>,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) {
    let mut failures = 0u32;
    loop {
        if desired.borrow().is_empty() {
            tokio::select! {biased; _=cancel.cancelled()=>return, changed=desired.changed()=>{if changed.is_err(){return;}}}
            continue;
        }
        let started = Instant::now();
        let result = tokio::select! {biased;
            _=cancel.cancelled()=>return,
            result=connection(&client,&mut desired,&events)=>result,
        };
        if result == Err(TwitchError::Unauthorized) {
            tokio::select! {biased; _=cancel.cancelled()=>{},_=events.send(Event::Unauthorized)=>{}}
            return;
        }
        if desired.borrow().is_empty() {
            continue;
        }
        if started.elapsed() > Duration::from_secs(60) {
            failures = 0;
        }
        if failures == 0 {
            tracing::warn!("Twitch event connection interrupted; reconnecting");
        }
        let delay = Duration::from_secs(1 << failures.min(6));
        failures = failures.saturating_add(1);
        tokio::select! {biased; _=cancel.cancelled()=>return,_=tokio::time::sleep(delay)=>{}}
    }
}

async fn send(socket: &mut WebSocket, value: Value) -> Result<(), TwitchError> {
    tokio::time::timeout(REPLY_TIMEOUT, socket.send(Message::Text(value.to_string())))
        .await
        .map_err(|_| TwitchError::Network)?
        .map_err(|_| TwitchError::Network)
}
async fn subscribe(
    socket: &mut WebSocket,
    verb: &str,
    topics: Vec<String>,
    token: &str,
    pending: &mut HashMap<String, Instant>,
) -> Result<(), TwitchError> {
    for topics in topics.chunks(10) {
        let nonce = crate::random_hex::<16>().map_err(|_| TwitchError::Configuration)?;
        send(
            socket,
            json!({"type":verb,"nonce":nonce,"data":{"topics":topics,"auth_token":token}}),
        )
        .await?;
        pending.insert(nonce, Instant::now());
    }
    Ok(())
}

async fn connection(
    client: &TwitchClient,
    desired: &mut watch::Receiver<BTreeSet<String>>,
    events: &mpsc::Sender<Event>,
) -> Result<(), TwitchError> {
    let config = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(512 * 1024));
    let response = client
        .http
        .client
        .get(client.http.endpoints.pubsub.clone())
        .upgrade()
        .web_socket_config(config)
        .send()
        .await
        .map_err(|_| TwitchError::Network)?;
    let mut socket = response
        .into_websocket()
        .await
        .map_err(|_| TwitchError::Network)?;
    let mut active = BTreeSet::new();
    let mut pending = HashMap::new();
    let mut ping_at = Instant::now();
    let mut pong_due = None;
    loop {
        let next = desired.borrow_and_update().clone();
        if active != next {
            subscribe(
                &mut socket,
                "UNLISTEN",
                active.difference(&next).cloned().collect(),
                client.token(),
                &mut pending,
            )
            .await?;
            subscribe(
                &mut socket,
                "LISTEN",
                next.difference(&active).cloned().collect(),
                client.token(),
                &mut pending,
            )
            .await?;
            active = next;
        }
        if active.is_empty() {
            return Ok(());
        }
        let deadline = pending
            .values()
            .map(|at| *at + REPLY_TIMEOUT)
            .chain(pong_due)
            .min()
            .unwrap_or(ping_at)
            .min(ping_at);
        tokio::select! {
            changed=desired.changed()=>{if changed.is_err(){return Ok(());}},
            _=tokio::time::sleep_until(deadline)=>{
                let now=Instant::now();
                if pending.values().any(|at|now.duration_since(*at)>=REPLY_TIMEOUT) || pong_due.is_some_and(|due|now>=due) {return Err(TwitchError::Network);}
                if now>=ping_at {
                    send(&mut socket,json!({"type":"PING"})).await?;
                    ping_at=now+PING_INTERVAL;
                    pong_due=Some(now+REPLY_TIMEOUT);
                }
            },
            message=socket.next()=>{
                let Some(Ok(message))=message else {return Err(TwitchError::Network)};
                match message {
                    Message::Ping(bytes)=> {tokio::time::timeout(REPLY_TIMEOUT,socket.send(Message::Pong(bytes))).await.map_err(|_|TwitchError::Network)?.map_err(|_|TwitchError::Network)?;},
                    Message::Text(text)=>{
                        let Ok(value)=serde_json::from_str::<Value>(&text) else {continue};
                        match value["type"].as_str() {
                            Some("PONG")=>pong_due=None,
                            Some("RECONNECT")=>return Ok(()),
                            Some("RESPONSE")=>{
                                if let Some(nonce)=value["nonce"].as_str() && pending.remove(nonce).is_some() {
                                    match value["error"].as_str() {
                                        Some("")=>{},
                                        Some("ERR_BADAUTH")=>return Err(TwitchError::Unauthorized),
                                        _=>return Err(TwitchError::Network),
                                    }
                                }
                            },
                            Some("MESSAGE")=>{
                                if let Some(topic)=value["data"]["topic"].as_str().filter(|t|active.contains(*t))
                                    && let Some(raw)=value["data"]["message"].as_str()
                                    && let Ok(payload)=serde_json::from_str::<Value>(raw)
                                    && let Some(event)=parse_event(topic,&payload) {
                                        events.send(event).await.map_err(|_|TwitchError::Cancelled)?;
                                    }
                            },
                            _=>{},
                        }
                    },
                    Message::Close{..}=>return Ok(()),
                    _=>{},
                }
            },
        }
    }
}

fn parse_event(topic: &str, value: &Value) -> Option<Event> {
    let (topic, id) = topic.rsplit_once('.')?;
    let id: u64 = id.parse().ok()?;
    let kind = value["type"].as_str()?;
    let data = &value["data"];
    match (topic, kind) {
        ("user-drop-events", "drop-progress") => Some(Event::Progress {
            id: data["drop_id"].as_str()?.to_owned(),
            minutes: number(&data["current_progress_min"]).and_then(|v| u32::try_from(v).ok())?,
        }),
        ("user-drop-events", "drop-claim") => Some(Event::Claim {
            id: data["drop_id"].as_str()?.to_owned(),
            instance: data["drop_instance_id"]
                .as_str()
                .filter(|v| !v.is_empty())?
                .to_owned(),
        }),
        ("onsite-notifications", "create-notification")
            if data["notification"]["type"] == "user_drop_reward_reminder_notification" =>
        {
            Some(Event::Notification(
                data["notification"]["id"].as_str()?.to_owned(),
            ))
        }
        ("video-playback-by-id", "stream-down") => Some(Event::Offline(id)),
        ("video-playback-by-id", "stream-up")
        | ("broadcast-settings-update", "broadcast_settings_update") => Some(Event::Changed(id)),
        ("video-playback-by-id", "viewcount") => Some(Event::Viewers {
            id,
            count: number(&value["viewers"])?,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Settings,
        twitch::{Endpoints, TwitchHttp, tests::session},
    };
    use axum::{
        Router,
        extract::ws::{Message as AxumMessage, WebSocketUpgrade},
        routing::get,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn messages_use_topic_identity_and_malformed_neighbors_are_ignored() {
        assert!(matches!(
            parse_event(
                "user-drop-events.42",
                &json!({"type":"drop-progress","data":{"drop_id":"a","current_progress_min":12}})
            ),
            Some(Event::Progress { minutes: 12, .. })
        ));
        assert!(
            parse_event(
                "video-playback-by-id.10",
                &json!({"type":"drop-progress","data":{"drop_id":"a","current_progress_min":12}})
            )
            .is_none()
        );
        assert!(
            parse_event(
                "user-drop-events.42",
                &json!({"type":"drop-progress","data":{"drop_id":"a","current_progress_min":-1}})
            )
            .is_none()
        );
        assert!(matches!(
            parse_event(
                "video-playback-by-id.10",
                &json!({"type":"stream-down","channel_id":999})
            ),
            Some(Event::Offline(10))
        ));
        assert!(matches!(
            parse_event(
                "video-playback-by-id.10",
                &json!({"type":"viewcount","viewers":0})
            ),
            Some(Event::Viewers { id: 10, count: 0 })
        ));
        assert!(parse_event("onsite-notifications.42",&json!({"type":"create-notification","data":{"notification":{"type":"unrelated","id":"x"}}})).is_none());
    }

    #[tokio::test]
    async fn real_socket_subscribes_updates_and_drains_all_shards() {
        let (received, mut requests) = mpsc::unbounded_channel();
        let connections = Arc::new(AtomicUsize::new(0));
        let open = connections.clone();
        let router=Router::new().route("/pubsub",get(move|upgrade:WebSocketUpgrade| {
            let received=received.clone();let open=open.clone();
            async move {upgrade.on_upgrade(move|mut socket|async move {
                open.fetch_add(1,Ordering::SeqCst);
                while let Some(Ok(AxumMessage::Text(text)))=socket.recv().await {
                    let value:Value=serde_json::from_str(&text).unwrap();
                    received.send(value.clone()).unwrap();
                    let response=if value["type"]=="PING" {json!({"type":"PONG"})}else {json!({"type":"RESPONSE","nonce":value["nonce"],"error":""})};
                    if socket.send(AxumMessage::Text(response.to_string().into())).await.is_err(){break;}
                    if value["type"]=="LISTEN" && value["data"]["topics"].as_array().unwrap().iter().any(|v|v=="user-drop-events.42") {
                        for payload in ["malformed".to_owned(),json!({"type":"drop-progress","data":{"drop_id":"reward","current_progress_min":17}}).to_string()] {
                            let event=json!({"type":"MESSAGE","data":{"topic":"user-drop-events.42","message":payload}});
                            if socket.send(AxumMessage::Text(event.to_string().into())).await.is_err(){break;}
                        }
                    }
                }
                open.fetch_sub(1,Ordering::SeqCst);
            })}
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let shutdown = CancellationToken::new();
        let stopped = shutdown.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(stopped.cancelled_owned())
                .await
                .unwrap();
        });
        let http = TwitchHttp::build(
            &Settings::default(),
            Some("testdevice"),
            CancellationToken::new(),
            Endpoints::mock(&base),
        )
        .unwrap();
        let client = TwitchClient::new(Arc::new(http), &session());
        let (events, mut receiver) = mpsc::channel(32);
        let mut pool = PubSub::start(client, events);
        let event = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event, Event::Progress { minutes: 17, .. }));
        pool.set_channels(&(1..=199).collect::<Vec<_>>());
        let mut subscribed = BTreeSet::new();
        tokio::time::timeout(Duration::from_secs(5), async {
            while subscribed.len() < 400 {
                let request = requests.recv().await.unwrap();
                if request["type"] == "LISTEN" {
                    assert_eq!(request["data"]["auth_token"], "testtoken");
                    let topics = request["data"]["topics"].as_array().unwrap();
                    assert!(topics.len() <= 10);
                    subscribed.extend(topics.iter().map(|t| t.as_str().unwrap().to_owned()));
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(connections.load(Ordering::SeqCst), 8);
        pool.set_channels(&[]);
        let unlisten = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let value = requests.recv().await.unwrap();
                if value["type"] == "UNLISTEN" {
                    break value;
                }
            }
        })
        .await
        .unwrap();
        assert!(!unlisten["data"]["topics"].as_array().unwrap().is_empty());
        pool.close().await;
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(connections.load(Ordering::SeqCst), 0);
        assert!(pool.tasks.is_empty());
    }
}
