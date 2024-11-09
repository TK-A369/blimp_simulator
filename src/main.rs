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
            },
            earth_radius: 6371000.0,
        }
    }

    async fn step(&mut self) {}
}

#[tokio::main]
async fn main() {
    let mut sim = Simulation::new();

    tokio::spawn(async move {
        loop {
            sim.step().await;

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    });

    println!("Hello, world!");
}
