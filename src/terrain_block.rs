use gl::types::*;
use id_allocator::IdAllocator;
use nalgebra::{Pnt2, Pnt3, Vec3};
use ncollide::bounding_volume::{AABB, AABB3};
use state::EntityId;
use std::cmp::partial_max;
use std::num::Float;
use std::ops::Add;
use stopwatch::TimerSet;
use terrain::{TerrainType, LOD_QUALITY};
use terrain_heightmap::HeightMap;
use tree_placer::TreePlacer;

pub const BLOCK_WIDTH: i32 = 8;

#[derive(Debug, PartialEq, Eq, Hash, Copy, Clone)]
pub struct BlockPosition(Pnt3<i32>);

impl BlockPosition {
  #[inline(always)]
  pub fn new(x: i32, y: i32, z: i32) -> BlockPosition {
    BlockPosition(Pnt3::new(x, y, z))
  }

  #[inline(always)]
  pub fn as_pnt<'a>(&'a self) -> &'a Pnt3<i32> {
    let BlockPosition(ref pnt) = *self;
    pnt
  }

  #[inline(always)]
  pub fn as_mut_pnt3<'a>(&'a mut self) -> &'a mut Pnt3<i32> {
    let BlockPosition(ref mut pnt) = *self;
    pnt
  }

  pub fn from_world_position(world_position: &Pnt3<f32>) -> BlockPosition {
    macro_rules! convert_coordinate(
      ($x: expr) => ({
        let x = $x.floor() as i32;
        let x =
          if x < 0 {
            x - (BLOCK_WIDTH - 1)
          } else {
            x
          };
        x / BLOCK_WIDTH
      })
    );
    BlockPosition(
      Pnt3::new(
        convert_coordinate!(world_position.x),
        convert_coordinate!(world_position.y),
        convert_coordinate!(world_position.z),
      )
    )
  }

  pub fn to_world_position(&self) -> Pnt3<f32> {
    Pnt3::new(
      (self.as_pnt().x * BLOCK_WIDTH) as f32,
      (self.as_pnt().y * BLOCK_WIDTH) as f32,
      (self.as_pnt().z * BLOCK_WIDTH) as f32,
    )
  }
}

impl Add<Vec3<i32>> for BlockPosition {
  type Output = BlockPosition;

  fn add(mut self, rhs: Vec3<i32>) -> Self {
    self.as_mut_pnt3().x += rhs.x;
    self.as_mut_pnt3().y += rhs.y;
    self.as_mut_pnt3().z += rhs.z;
    self
  }
}

pub struct TerrainBlock {
  // These Vecs must all be ordered the same way; each entry is the next triangle.

  pub vertex_coordinates: Vec<[Pnt3<GLfloat>; 3]>,
  pub normals: Vec<[Vec3<GLfloat>; 3]>,
  // per-vertex 2D coordinates into terrain_vram_buffers::pixels.
  pub coords: Vec<[Pnt2<u32>; 3]>,

  // per-triangle entity IDs
  pub ids: Vec<EntityId>,
  // per-triangle bounding boxes
  // TODO: Change this back to a HashMap once initial capacity is zero for those.
  pub bounds: Vec<(EntityId, AABB3<GLfloat>)>,
}

impl TerrainBlock {
  pub fn empty() -> TerrainBlock {
    TerrainBlock {
      vertex_coordinates: Vec::new(),
      normals: Vec::new(),
      coords: Vec::new(),

      ids: Vec::new(),
      bounds: Vec::new(),
    }
  }

  pub fn generate(
    timers: &TimerSet,
    id_allocator: &mut IdAllocator<EntityId>,
    heightmap: &HeightMap,
    treemap: &TreePlacer,
    position: &BlockPosition,
    lod_index: u32,
  ) -> TerrainBlock {
    timers.time("update.generate_block", || {
      let mut block = TerrainBlock::empty();

      let x = (position.as_pnt().x * BLOCK_WIDTH) as f32;
      let y = (position.as_pnt().y * BLOCK_WIDTH) as f32;
      let z = (position.as_pnt().z * BLOCK_WIDTH) as f32;

      let lateral_samples = LOD_QUALITY[lod_index as usize];
      let sample_width = BLOCK_WIDTH as f32 / lateral_samples as f32;

      for dx in range(0, lateral_samples) {
        let x = x + dx as f32 * sample_width;
        for dz in range(0, lateral_samples) {
          let z = z + dz as f32 * sample_width;
          let position = Pnt3::new(x, y, z);
          TerrainBlock::add_tile(
            timers,
            heightmap,
            treemap,
            id_allocator,
            &mut block,
            sample_width,
            &position,
            lod_index,
          );
        }
      }

      block
    })
  }

  fn add_tile<'a>(
    timers: &TimerSet,
    hm: &HeightMap,
    treemap: &TreePlacer,
    id_allocator: &mut IdAllocator<EntityId>,
    block: &mut TerrainBlock,
    sample_width: f32,
    position: &Pnt3<f32>,
    lod_index: u32,
  ) {
    let half_width = sample_width / 2.0;
    let center = hm.point_at(position.x + half_width, position.z + half_width);

    if position.y >= center.y || center.y > position.y + BLOCK_WIDTH as f32 {
      return;
    }

    timers.time("update.generate_block.add_tile", || {
      let normal_delta = sample_width / 2.0;
      let center_normal =
        hm.normal_at(normal_delta, position.x + half_width, position.z + half_width);

      let x2 = position.x + sample_width;
      let z2 = position.z + sample_width;

      let ps: [Pnt3<f32>; 4] =
        [ hm.point_at(position.x, position.z)
        , hm.point_at(position.x, z2)
        , hm.point_at(x2, z2)
        , hm.point_at(x2, position.z)
        ];

      let ns: [Vec3<f32>; 4] =
        [ hm.normal_at(normal_delta, position.x, position.z)
        , hm.normal_at(normal_delta, position.x, z2)
        , hm.normal_at(normal_delta, x2, z2)
        , hm.normal_at(normal_delta, x2, position.z)
        ];

      let center_lower_than = ps.iter().filter(|v| center.y < v.y).count();

      let terrain_type =
        if center_lower_than == 4 {
          TerrainType::Stone
        } else if center_lower_than == 3 {
          TerrainType::Dirt
        } else {
          TerrainType::Grass
        };

      macro_rules! place_terrain(
        ($v1: expr,
         $v2: expr,
         $n1: expr,
         $n2: expr,
         $minx: expr,
         $minz: expr,
         $maxx: expr,
         $maxz: expr
        ) => ({
          let maxy = partial_max($v1.y, $v2.y);
          let maxy = maxy.and_then(|m| partial_max(m, center.y));
          let maxy = maxy.unwrap();

          let id = id_allocator.allocate();

          block.vertex_coordinates.push([$v1, $v2, center]);
          block.normals.push([$n1, $n2, center_normal]);
          let coord =
            Pnt2::new(0,
              match terrain_type {
                TerrainType::Grass => 0,
                TerrainType::Dirt => 1,
                TerrainType::Stone => 2,
                TerrainType::Wood => 3,
                TerrainType::Leaf => 4,
              }
            );
          block.coords.push([coord, coord, coord]);
          block.ids.push(id);
          block.bounds.push((
            id,
            AABB::new(
              Pnt3::new($minx, $v1.y, $minz),
              Pnt3::new($maxx, maxy, $maxz),
            ),
          ));
        });
      );

      let polys =
        (LOD_QUALITY[lod_index as usize] * LOD_QUALITY[lod_index as usize] * 4) as usize;
      block.vertex_coordinates.reserve(polys);
      block.normals.reserve(polys);
      block.coords.reserve(polys);
      block.ids.reserve(polys);
      block.bounds.reserve(polys);

      let centr = center; // makes alignment nice
      place_terrain!(ps[0], ps[1], ns[0], ns[1], ps[0].x, ps[0].z, centr.x, ps[1].z);
      place_terrain!(ps[1], ps[2], ns[1], ns[2], ps[1].x, centr.z, ps[2].x, ps[2].z);
      place_terrain!(ps[2], ps[3], ns[2], ns[3], centr.x, centr.z, ps[2].x, ps[2].z);
      place_terrain!(ps[3], ps[0], ns[3], ns[0], ps[0].x, ps[0].z, ps[3].x, centr.z);

      if treemap.should_place_tree(&centr) {
        treemap.place_tree(centr, id_allocator, block, lod_index);
      }
    })
  }
}
