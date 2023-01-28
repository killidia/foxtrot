use crate::file_system_interaction::level_serialization::{CurrentLevel, WorldLoadRequest};
use crate::level_design::spawning::{DelayedSpawnEvent, GameObject, SpawnEvent};
use crate::player_control::player_embodiment::Player;
use crate::world_interaction::condition::ActiveConditions;
use crate::world_interaction::dialog::{CurrentDialog, DialogEvent};
use crate::GameState;
use bevy::prelude::*;
use chrono::prelude::Local;
use glob::glob;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

pub struct SavingPlugin;

impl Plugin for SavingPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<GameSaveRequest>()
            .add_event::<GameLoadRequest>()
            .register_type::<GameSaveRequest>()
            .register_type::<GameLoadRequest>()
            .add_system_set(
                SystemSet::on_in_stack_update(GameState::Playing)
                    .with_system(handle_load_requests.label("handle_game_load_requests"))
                    .with_system(handle_save_requests.after("handle_game_load_requests")),
            );
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Resource, Reflect, Serialize, Deserialize, Default)]
#[reflect(Resource, Serialize, Deserialize)]
pub struct GameSaveRequest {
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Resource, Reflect, Serialize, Deserialize, Default)]
#[reflect(Resource, Serialize, Deserialize)]
pub struct GameLoadRequest {
    pub filename: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Resource, Serialize, Deserialize, Default)]
struct SaveModel {
    scene: String,
    #[serde(default, skip_serializing_if = "ActiveConditions::is_empty")]
    conditions: ActiveConditions,
    player_transform: Transform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dialog_event: Option<DialogEvent>,
}

fn handle_load_requests(
    mut commands: Commands,
    mut load_events: EventReader<GameLoadRequest>,
    mut loader: EventWriter<WorldLoadRequest>,
    mut spawner: EventWriter<DelayedSpawnEvent>,
    mut dialog_event_writer: EventWriter<DialogEvent>,
) {
    for load in load_events.iter() {
        let path = match load
            .filename
            .as_ref()
            .map(|filename| Some(get_save_path(filename.clone())))
            .unwrap_or_else(|| {
                let mut saves: Vec<_> = glob("./saves/*.sav.ron")
                    .expect("Failed to read glob pattern")
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| entry.is_file())
                    .collect();
                saves.sort_by_cached_key(|f| f.metadata().unwrap().modified().unwrap());
                saves.last().map(|entry| entry.to_owned())
            }) {
            Some(path) => path,
            None => {
                error!("Failed to load save: No filename provided and no saves found on disk");
                continue;
            }
        };
        let serialized = match fs::read_to_string(&path) {
            Ok(serialized) => serialized,
            Err(e) => {
                error!(
                    "Failed to read save {:?} at {:?}: {}",
                    &load.filename, path, e
                );
                continue;
            }
        };
        let save_model: SaveModel = match ron::from_str(&serialized) {
            Ok(save_model) => save_model,
            Err(e) => {
                error!(
                    "Failed to deserialize save {:?} at {:?}: {}",
                    &load.filename, path, e
                );
                continue;
            }
        };
        loader.send(WorldLoadRequest {
            filename: save_model.scene,
        });
        if let Some(dialog_event) = save_model.dialog_event {
            dialog_event_writer.send(dialog_event);
        }
        commands.insert_resource(save_model.conditions);

        spawner.send(DelayedSpawnEvent {
            tick_delay: 2,
            event: SpawnEvent {
                object: GameObject::Player,
                transform: Transform {
                    scale: Vec3::splat(1.0),
                    ..save_model.player_transform
                },
                parent: None,
                name: Some("Player".into()),
            },
        });
    }
}

fn handle_save_requests(
    mut save_events: EventReader<GameSaveRequest>,
    conditions: Res<ActiveConditions>,
    dialog: Option<Res<CurrentDialog>>,
    player_query: Query<&GlobalTransform, With<Player>>,
    current_level: Option<Res<CurrentLevel>>,
) {
    let dialog = if let Some(ref dialog) = dialog {
        let dialog: CurrentDialog = dialog.as_ref().clone();
        Some(dialog)
    } else {
        None
    };
    let current_level = match current_level {
        Some(level) => level,
        None => return,
    };
    for save in save_events.iter() {
        for player in &player_query {
            let dialog_event = dialog.clone().map(|dialog| DialogEvent {
                dialog: dialog.id,
                page: Some(dialog.current_page),
            });
            let save_model = SaveModel {
                scene: current_level.scene.clone(),
                conditions: conditions.clone(),
                dialog_event,
                player_transform: player.compute_transform(),
            };
            let serialized = match ron::to_string(&save_model) {
                Ok(string) => string,
                Err(e) => {
                    error!("Failed to save world: {}", e);
                    continue;
                }
            };
            let filename = save
                .filename
                .clone()
                .unwrap_or_else(|| Local::now().to_rfc2822().replace(':', "-"));
            let path = get_save_path(filename.clone());
            fs::write(path, serialized)
                .unwrap_or_else(|e| error!("Failed to write save {filename}: {e}"));
        }
    }
}

fn get_save_path(filename: impl Into<Cow<'static, str>>) -> PathBuf {
    Path::new("saves").join(format!("{}.sav.ron", filename.into()))
}
