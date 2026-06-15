//! HTTP-level integration test for the web layer: the human `/api` surface and the
//! dashboard route, exercised against a real axum server on an ephemeral port.

use agora::bus::Bus;
use agora::store;
use agora::web;

async fn spawn_server() -> String {
    let conn = store::open_in_memory().unwrap();
    let bus = Bus::new(conn, "main");
    let app = web::router_single(bus);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn human_api_round_trip() {
    let url = spawn_server().await;
    let client = reqwest::Client::new();

    // Dashboard serves HTML.
    let html = client.get(&url).send().await.unwrap().text().await.unwrap();
    assert!(html.contains("agora"), "dashboard should render");

    // Human posts a directed message.
    let posted: serde_json::Value = client
        .post(format!("{url}/api/post"))
        .json(&serde_json::json!({
            "from": "human", "to": ["architect"], "body": "design the schema"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(posted["from"], "human");
    assert_eq!(posted["kind"], "msg");
    assert_eq!(posted["seq"], 1);

    // The roster lists connected agents only; the human operator (posting via the
    // dashboard API) is not registered as an agent.
    let roster: Vec<serde_json::Value> = client
        .get(format!("{url}/api/roster"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!roster.iter().any(|a| a["name"] == "human"), "human must not appear in the roster");

    // The directed task is unanswered -> system is active (a worker owes a reply,
    // but here only the human exists, so it shows active rather than done).
    let q: serde_json::Value = client
        .get(format!("{url}/api/quiescence"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(q["state"].is_string());

    // History returns the posted message.
    let history: Vec<serde_json::Value> = client
        .get(format!("{url}/api/history?limit=10"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["body"], "design the schema");
}
