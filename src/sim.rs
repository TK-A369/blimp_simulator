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

pub async fn sim_start(
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
) -> (
    tokio::sync::mpsc::Sender<blimp_onboard_software::obsw_algo::MessageG2B>,
    tokio::sync::broadcast::Receiver<(u8, i32)>,
) {
    // When simulated blimp wants to set motors, it will be sent to this channel
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

    let blimp_send_msg_tx = {
        // Channel for sending messages to blimp

        let mut shutdown_rx = shutdown_tx.subscribe();
        let sim = sim.clone();
        let (blimp_send_msg_tx, mut blimp_send_msg_rx) =
            tokio::sync::mpsc::channel::<blimp_onboard_software::obsw_algo::MessageG2B>(64);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = blimp_send_msg_rx.recv() => {
                        if let Some(msg) = msg {
                        sim.lock()
                            .await
                            .blimp
                            .main_algo
                            .handle_event(&blimp_onboard_software::obsw_algo::BlimpEvent::GetMsg(
                                postcard::to_stdvec::<blimp_onboard_software::obsw_algo::MessageG2B>(
                                    &msg,
                                )
                                .unwrap(),
                            ))
                            .await;
                        }
                        else {
                            break;
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        break;
                    }
                };
            }
        });
        blimp_send_msg_tx
    };

    (blimp_send_msg_tx, motors_rx)
}
