use futures_util::{SinkExt, StreamExt};
use tokio;
use tokio_tungstenite;

use blimp_ground_ws_interface;

pub async fn handle_ground_ws_connection(
    stream: tokio::net::TcpStream,
    mut motors_rx: tokio::sync::broadcast::Receiver<(u8, i32)>,
    mut servos_rx: tokio::sync::broadcast::Receiver<(u8, i16)>,
    blimp_send_msg_tx: tokio::sync::mpsc::Sender<blimp_onboard_software::obsw_algo::MessageG2B>,
) {
    println!("Accepting new WebSocket connection...");
    if let Ok(mut ws_stream) = tokio_tungstenite::accept_async(stream).await {
        println!(
            "New WebSocket connection with {}",
            ws_stream
                .get_ref()
                .peer_addr()
                .and_then(|x| Ok(format!("{}", x)))
                .unwrap_or("unknown".to_string())
        );

        let mut use_postcard: Option<bool> = None;
        let curr_interest = std::sync::Arc::new(tokio::sync::Mutex::new(
            blimp_ground_ws_interface::VizInterest::new(),
        ));

        async fn handle_message_v2g(
            msg: blimp_ground_ws_interface::MessageV2G,
            curr_interest: std::sync::Arc<
                tokio::sync::Mutex<blimp_ground_ws_interface::VizInterest>,
            >,
            blimp_send_msg_tx: tokio::sync::mpsc::Sender<
                blimp_onboard_software::obsw_algo::MessageG2B,
            >,
        ) {
            println!("Got V2G message:\n{:#?}", &msg);
            match msg {
                blimp_ground_ws_interface::MessageV2G::DeclareInterest(interest) => {
                    *(curr_interest.lock().await) = interest;
                }
                blimp_ground_ws_interface::MessageV2G::Controls(
                    blimp_ground_ws_interface::Controls {
                        throttle,
                        elevation,
                        yaw,
                    },
                ) => {
                    blimp_send_msg_tx
                        .send(blimp_onboard_software::obsw_algo::MessageG2B::Control(
                            blimp_onboard_software::obsw_algo::Controls {
                                throttle,
                                elevation,
                                yaw,
                            },
                        ))
                        .await
                        .unwrap();
                }
            }
        }

        async fn send_ws_msg(
            ws_stream: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
            use_postcard: bool,
            msg: blimp_ground_ws_interface::MessageG2V,
        ) {
            let msg_ser = if use_postcard {
                tokio_tungstenite::tungstenite::Message::Binary(postcard::to_stdvec(&msg).unwrap())
            } else {
                tokio_tungstenite::tungstenite::Message::Text(serde_json::to_string(&msg).unwrap())
            };
            ws_stream.send(msg_ser).await.unwrap();
        }

        loop {
            tokio::select! {
                ws_msg = ws_stream.next() => {
                    if let Some(ws_msg)=ws_msg {
                        if let Ok(ws_msg) = ws_msg {
                            match ws_msg {
                                tokio_tungstenite::tungstenite::Message::Text(msg_str) => {
                                    if let None = use_postcard {
                                        use_postcard = Some(false);
                                    }
                                    if let Ok(msg) =
                                        serde_json::from_str::<blimp_ground_ws_interface::MessageV2G>(
                                            &msg_str,
                                        ) {
                                            handle_message_v2g(msg, curr_interest.clone(), blimp_send_msg_tx.clone()).await;
                                    }
                                    else {
                                        eprintln!("Couldn't deserialize JSON message from WebSocket!");
                                    }
                                }
                                tokio_tungstenite::tungstenite::Message::Binary(msg_bin) => {
                                    if let None = use_postcard {
                                        use_postcard = Some(true);
                                    }
                                    if let Ok(msg) =
                                        postcard::from_bytes::<blimp_ground_ws_interface::MessageV2G>(
                                            &msg_bin,
                                        ) {
                                        handle_message_v2g(msg, curr_interest.clone(), blimp_send_msg_tx.clone()).await;
                                    }
                                    else {
                                        eprintln!("Couldn't deserialize Postcard message from WebSocket!");
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    else {
                        break;
                    }
                }
                motors_update = motors_rx.recv() => {
                    if curr_interest.lock().await.motors.clone() {
                        let motors_update = motors_update.unwrap();
                        send_ws_msg(
                            &mut ws_stream,
                            use_postcard.unwrap_or(true),
                            blimp_ground_ws_interface::MessageG2V::MotorSpeed{id: motors_update.0, speed: motors_update.1}
                        ).await;
                    }
                }
                servos_update = servos_rx.recv() => {
                    if curr_interest.lock().await.servos {
                        let servos_update = servos_update.unwrap();
                        send_ws_msg(&mut ws_stream, use_postcard.unwrap_or(true), blimp_ground_ws_interface::MessageG2V::ServoPosition{id: servos_update.0, angle:servos_update.1}).await;
                    }
                }
            }
        }
    } else {
        eprintln!("Error occurred while accepting WebSocket connection!");
    }
}

pub async fn ws_server_start(
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
    sim_channels: &crate::sim::SimChannels,
) {
    let mut shutdown_rx = shutdown_tx.subscribe();
    let ws_listener = tokio::net::TcpListener::bind("127.0.0.1:8765")
        .await
        .expect("Couldn't open WebSocket listener");
    let mut ws_conn_next_id: u32 = 0;
    loop {
        tokio::select! {
            res = ws_listener.accept() => {
                if let Ok((stream, _)) = res {
                    tokio::spawn(handle_ground_ws_connection(stream, sim_channels. motors_rx.resubscribe(), sim_channels. servos_rx.resubscribe(), sim_channels. msg_tx.clone()));
                }
            }
            _ = shutdown_rx.recv() => {
                break;
            }
        }
    }
}
