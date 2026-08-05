use axum::http::HeaderValue;
use axum::http::header::SEC_WEBSOCKET_PROTOCOL;
use futures_util::StreamExt;
use remarque_page_log::{ServerMessage, decode_server_message};
use std::collections::BTreeSet;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let websocket_url = std::env::var("REMARQUE_INSPECT_WEBSOCKET_URL")?;
    let session_token = std::env::var("REMARQUE_INSPECT_SESSION_TOKEN")?;
    let mut request = websocket_url.into_client_request()?;
    request.headers_mut().insert(
        SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_str(&format!("remarque.session.{session_token}"))?,
    );
    let (mut websocket, _) = connect_async(request).await?;
    let bytes = websocket
        .next()
        .await
        .ok_or("relay closed before the welcome message")??
        .into_data();
    let ServerMessage::Welcome { snapshot, .. } = decode_server_message(&bytes)? else {
        return Err("first relay message was not a welcome".into());
    };
    let committed_authors = snapshot
        .strokes
        .iter()
        .map(|stroke| stroke.author)
        .collect::<BTreeSet<_>>();
    let active_authors = snapshot
        .active_strokes
        .iter()
        .map(|active| active.stroke.author)
        .collect::<BTreeSet<_>>();
    let single_point_strokes = snapshot
        .strokes
        .iter()
        .filter(|stroke| stroke.points.len() == 1)
        .count();
    let stationary_strokes = snapshot
        .strokes
        .iter()
        .filter(|stroke| {
            stroke
                .points
                .iter()
                .skip(1)
                .all(|point| point.x == stroke.points[0].x && point.y == stroke.points[0].y)
        })
        .count();
    println!("revision={}", snapshot.revision);
    println!("committed_strokes={}", snapshot.strokes.len());
    println!("committed_authors={}", committed_authors.len());
    println!("single_point_strokes={single_point_strokes}");
    println!("stationary_strokes={stationary_strokes}");
    println!("active_strokes={}", snapshot.active_strokes.len());
    println!("active_authors={}", active_authors.len());
    for (index, active) in snapshot.active_strokes.iter().enumerate() {
        println!("active[{index}].points={}", active.stroke.points.len());
    }
    Ok(())
}
