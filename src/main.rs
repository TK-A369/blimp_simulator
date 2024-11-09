use blimp_onboard_software;
use fixed;
use nalgebra;
use simba;
use tokio;
use typenum;

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
        Self {
            blimp: SimBlimp {
                coord_mat: nalgebra::Affine3::identity(),
                main_algo: blimp_onboard_software::obsw_algo::BlimpMainAlgo::new(),
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
    let mut sim = Simulation::new();

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

    {
        let mut shutdown_rx = shutdown_tx.subscribe();
        tokio::spawn(async move {
            loop {
                sim.step().await;

                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {},
                    _ = shutdown_rx.recv() => {
                        break;
                    },
                };
            }
        });
    }

    println!("Hello, world!");

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap_or(());
        shutdown_tx.send(()).unwrap();
    });

    shutdown_rx.recv().await.unwrap();
}
