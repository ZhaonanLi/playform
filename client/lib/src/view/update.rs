//! Define the updates passed from the client to the view.

use cgmath::Point3;
use stopwatch;

use common::entity_id;

use chunk_position;
use lod;
use terrain_mesh;
use vertex::ColoredVertex;
use view;
use view::light;
use view::mob_buffers::VERTICES_PER_MOB;
use view::player_buffers::VERTICES_PER_PLAYER;

/// Messages from the client to the view.
pub enum T {
  /// Set the camera location.
  MoveCamera(Point3<f32>),

  /// Update a player mesh.
  UpdatePlayer(entity_id::T, [ColoredVertex; VERTICES_PER_PLAYER]),
  /// Update a mob mesh.
  UpdateMob(entity_id::T, [ColoredVertex; VERTICES_PER_MOB]),

  /// Update the sun.
  SetSun(light::Sun),

  /// Add a terrain chunk to the view.
  AddChunk(chunk_position::T, terrain_mesh::T, lod::T),
  /// Remove a terrain entity.
  RemoveTerrain(entity_id::T),
  /// Remove a grass billboard.
  RemoveGrass(entity_id::T),
  /// Treat a series of updates as an atomic operation.
  Atomic(Vec<T>),
}

unsafe impl Send for T {}

pub use self::T::*;

#[allow(missing_docs)]
pub fn apply_client_to_view(view: &mut view::T, up: T) {
  match up {
    T::MoveCamera(position) => {
      view.camera.translate_to(position);
    },
    T::UpdateMob(id, triangles) => {
      view.mob_buffers.insert(&mut view.gl, id, &triangles);
    },
    T::UpdatePlayer(id, triangles) => {
      view.player_buffers.insert(&mut view.gl, id, &triangles);
    },
    T::SetSun(sun) => {
      match view.input_mode {
        view::InputMode::Sun => {},
        _ => {
          view.sun = sun;
        },
      }
    },
    T::AddChunk(_, chunk, _) => {
      stopwatch::time("add_chunk", || {
        view.terrain_buffers.push(
          &mut view.gl,
          chunk.vertex_coordinates.as_ref(),
          chunk.normals.as_ref(),
          chunk.ids.as_ref(),
          chunk.materials.as_ref(),
        );
        view.grass_buffers.push(
          &mut view.gl,
          chunk.grass.as_ref(),
          chunk.grass_ids.as_ref(),
        );
      })
    },
    T::RemoveTerrain(id) => {
      view.terrain_buffers.swap_remove(&mut view.gl, id);
    },
    T::RemoveGrass(id) => {
      view.grass_buffers.swap_remove(&mut view.gl, id);
    },
    T::Atomic(updates) => {
      for up in updates.into_iter() {
        apply_client_to_view(view, up);
      }
    },
  };
}