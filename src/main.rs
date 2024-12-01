mod sim;
mod websocket;

use fixed;
use futures_util::{SinkExt, StreamExt};
use nalgebra;
use postcard;
use simba;
use tokio;
use tokio_tungstenite;
use typenum;

use blimp_ground_ws_interface;
use blimp_onboard_software;
use blimp_onboard_software::obsw_interface::BlimpAlgorithm;

#[tokio::main]
async fn main() {
    let mut ws_conns: std::sync::Arc<
        tokio::sync::Mutex<std::collections::BTreeMap<u32, tokio::sync::mpsc::Sender<()>>>,
    > = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::BTreeMap::new()));

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

    let sim_channels = crate::sim::sim_start(shutdown_tx.clone()).await;

    {
        // Ping the blimp

        let mut shutdown_rx = shutdown_tx.subscribe();
        let blimp_send_msg_tx = sim_channels.msg_tx.clone();
        tokio::spawn(async move {
            let mut i: u32 = 0;
            loop {
                println!("Pinging the blimp with id {}", i);
                blimp_send_msg_tx
                    .send(blimp_onboard_software::obsw_algo::MessageG2B::Ping(i))
                    .await
                    .unwrap();
                i += 1;

                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(1000))=>{}
                    _ = shutdown_rx.recv()=>{
                        break;
                    }
                };
            }
        });
    }

    // WebSocket server for visualizations, etc.
    crate::websocket::ws_server_start(shutdown_tx.clone(), &sim_channels).await;

    println!("Hello, world!");

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap_or(());
        shutdown_tx.send(()).unwrap();
    });

    shutdown_rx.recv().await.unwrap();
}
