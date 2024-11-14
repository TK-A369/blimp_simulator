use fixed;
use futures_util::StreamExt;
use nalgebra;
use postcard;
use simba;
use tokio;
use tokio_tungstenite;
use typenum;

use blimp_ground_ws_interface;
use blimp_onboard_software;
use blimp_onboard_software::obsw_interface::BlimpAlgorithm;

struct SimBlimp {
    coord_mat: nalgebra::Affine3<f64>,
    main_algo: blimp_onboard_software::obsw_algo::BlimpMainAlgo,
}

struct Simulation {
    blimp: SimBlimp,
    earth_radius: f64,
}

impl Simulation {
    fn new() -> Self {
        let mut blimp_main_algo = blimp_onboard_software::obsw_algo::BlimpMainAlgo::new();
        //blimp_main_algo.set_action_callback();
        //blimp_main_algo.set_action_callback(action_callback);
        Self {
            blimp: SimBlimp {
                coord_mat: nalgebra::Affine3::identity(),
                main_algo: blimp_main_algo,
            },
            earth_radius: 6371000.0,
        }
    }

    async fn step(&mut self) {
        self.blimp.main_algo.step().await;
    }
}

async fn handle_ground_ws_connection(
    stream: tokio::net::TcpStream,
    motors_rx: tokio::sync::broadcast::Receiver<(u8, i32)>,
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
        let mut curr_interest = blimp_ground_ws_interface::VisInterest::new();

        let mut handle_message_v2g = move |msg: blimp_ground_ws_interface::MessageV2G| {
            println!("Got V2G message:\n{:#?}", &msg);
            match msg {
                blimp_ground_ws_interface::MessageV2G::DeclareInterest(interest) => {
                    curr_interest = interest;
                }
                blimp_ground_ws_interface::MessageV2G::Controls(ctrls) => {}
            }
        };

        while let Some(ws_msg) = ws_stream.next().await {
            if let Ok(ws_msg) = ws_msg {
                match ws_msg {
                    tokio_tungstenite::tungstenite::Message::Text(msg_str) => {
                        if let None = use_postcard {
                            use_postcard = Some(false);
                        }
                        let msg =
                            serde_json::from_str::<blimp_ground_ws_interface::MessageV2G>(&msg_str)
                                .unwrap();
                        handle_message_v2g(msg);
                    }
                    tokio_tungstenite::tungstenite::Message::Binary(msg_bin) => {
                        if let None = use_postcard {
                            use_postcard = Some(true);
                        }
                        let msg =
                            postcard::from_bytes::<blimp_ground_ws_interface::MessageV2G>(&msg_bin)
                                .unwrap();
                        handle_message_v2g(msg);
                    }
                    _ => {}
                }
            }
        }
    } else {
        eprintln!("Error occurred while accepting WebSocket connection!");
    }
}

#[tokio::main]
async fn main() {
    let mut ws_conns: std::sync::Arc<
        tokio::sync::Mutex<std::collections::BTreeMap<u32, tokio::sync::mpsc::Sender<()>>>,
    > = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::BTreeMap::new()));

    let (motors_tx, mut motors_rx) = tokio::sync::broadcast::channel::<(u8, i32)>(64);

    let mut sim: std::sync::Arc<tokio::sync::Mutex<Simulation>> =
        std::sync::Arc::new(tokio::sync::Mutex::new(Simulation::new()));

    let blimp_action_callback = {
        let sim = sim.clone();
        let motors_tx = motors_tx.clone();
        let blimp_action_callback = Box::new(move |action| {
            // println!("Action {:#?}", action);
            match action {
                blimp_onboard_software::obsw_algo::BlimpAction::SendMsg(msg) => {
                    if let Ok(msg_des) =
                        postcard::from_bytes::<blimp_onboard_software::obsw_algo::MessageB2G>(&msg)
                    {
                        println!("Got message:\n{:#?}", msg_des);

                        match msg_des {
                            blimp_onboard_software::obsw_algo::MessageB2G::Ping(ping_id) => {
                                let sim = sim.clone();
                                tokio::spawn(async move {
                                    sim.blocking_lock().blimp.main_algo.handle_event(
                                        &blimp_onboard_software::obsw_algo::BlimpEvent::GetMsg(
                                            postcard::to_stdvec::<
                                                blimp_onboard_software::obsw_algo::MessageG2B,
                                            >(
                                                &blimp_onboard_software::obsw_algo::MessageG2B::Pong(
                                                    ping_id,
                                                ),
                                            )
                                            .unwrap(),
                                        ),
                                    ).await;
                                });
                            }
                            blimp_onboard_software::obsw_algo::MessageB2G::Pong(ping_id) => {}
                            blimp_onboard_software::obsw_algo::MessageB2G::ForwardAction(
                                fwd_action,
                            ) => match fwd_action {
                                blimp_onboard_software::obsw_algo::BlimpAction::SetMotor {
                                    motor,
                                    speed,
                                } => {
                                    motors_tx.send((motor, speed)).unwrap();
                                }
                                _ => {}
                            },
                        }
                    }
                }
                blimp_onboard_software::obsw_algo::BlimpAction::SetMotor { motor, speed } => {}
                _ => {}
            }
        });
        blimp_action_callback
    };
    sim.lock()
        .await
        .blimp
        .main_algo
        .set_action_callback(blimp_action_callback);

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

    {
        // Execute blimp's algorithm steps

        let mut shutdown_rx = shutdown_tx.subscribe();
        let sim = sim.clone();
        tokio::spawn(async move {
            loop {
                sim.lock().await.step().await;

                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {},
                    _ = shutdown_rx.recv() => {
                        break;
                    },
                };
            }
        });
    }

    {
        // Ping the blimp

        let mut shutdown_rx = shutdown_tx.subscribe();
        tokio::spawn(async move {
            let mut i: u32 = 0;
            loop {
                println!("Pinging the blimp with id {}", i);
                sim.lock()
                    .await
                    .blimp
                    .main_algo
                    .handle_event(&blimp_onboard_software::obsw_algo::BlimpEvent::GetMsg(
                        postcard::to_stdvec::<blimp_onboard_software::obsw_algo::MessageG2B>(
                            &blimp_onboard_software::obsw_algo::MessageG2B::Ping(i),
                        )
                        .unwrap(),
                    ))
                    .await;
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

    {
        // WebSocket server for visualizations, etc.

        let mut shutdown_rx = shutdown_tx.subscribe();
        let ws_listener = tokio::net::TcpListener::bind("127.0.0.1:8765")
            .await
            .expect("Couldn't open WebSocket listener");
        let mut ws_conn_next_id: u32 = 0;
        loop {
            tokio::select! {
                res = ws_listener.accept() => {
                    if let Ok((stream, _)) = res {
                        tokio::spawn(handle_ground_ws_connection(stream, motors_tx.subscribe()));
                    }
                }
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        }
    }

    println!("Hello, world!");

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap_or(());
        shutdown_tx.send(()).unwrap();
    });

    shutdown_rx.recv().await.unwrap();
}
