// Configure clippy for Bevy usage
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::enum_glob_use)]

use bevy::{
    app::{ScheduleRunnerPlugin, ScheduleRunnerSettings},
    core::CorePlugin,
    prelude::*,
    utils::Duration,
};

use game::GamePlugin;

mod game;
mod map;
mod network;

const TIMESTEP_5_PER_SECOND: f64 = 30.0 / 60.0;

pub fn setup() {
    App::new()
        .insert_resource(ScheduleRunnerSettings::run_loop(Duration::from_secs_f64(
            TIMESTEP_5_PER_SECOND,
        )))
        .add_plugin(CorePlugin::default())
        .add_plugin(ScheduleRunnerPlugin::default())
        .add_plugin(GamePlugin)
        .run();
}
