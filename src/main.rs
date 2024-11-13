use fixed;
use nalgebra;
use postcard;
use simba;
use tokio;
use tokio_tungstenite;
use typenum;

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
        blimp_main_algo.set_action_callback(Box::new(|action| {
            // println!("Action {:#?}", action);
            match action {
                blimp_onboard_software::obsw_algo::BlimpAction::SendMsg(msg) => {
                    if let Ok(msg_des) =
                        postcard::from_bytes::<blimp_onboard_software::obsw_algo::MessageB2G>(&msg)
                    {
                        println!("Got message:\n{:#?}", msg_des);
                    }
                }
                _ => {}
            }
        }));
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

#[tokio::main]
async fn main() {
    let mut sim = std::sync::Arc::new(tokio::sync::Mutex::new(Simulation::new()));

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

        let ws_listener = tokio::net::TcpListener::bind("127.0.0.1:8765")
            .await
            .expect("Couldn't open WebSocket listener");
        loop {
            tokio::select! {
                res = ws_listener.accept() => {
                    if let Ok((stream, _)) = res {
                        tokio::spawn(async {
                            if let Ok(mut ws_stream) = tokio_tungstenite::accept_async(stream).await{
                                println!("New WebSocket connection with {}", ws_stream.get_ref().peer_addr().and_then(|x| Ok(format!("{}", x))).unwrap_or("unknown".to_string()));
                            }
                        });
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
