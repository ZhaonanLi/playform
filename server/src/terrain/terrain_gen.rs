use cgmath::{Point, Point3, Vector, Vector3};
use cgmath::Aabb3;
use std::cmp::{partial_min, partial_max};
use std::iter::range_inclusive;
use std::num::Int;
use std::sync::Mutex;

use common::block_position::BlockPosition;
use common::entity::EntityId;
use common::id_allocator::IdAllocator;
use common::lod::LODIndex;
use common::stopwatch::TimerSet;
use common::terrain_block::{TerrainBlock, BLOCK_WIDTH, LOD_QUALITY, tri};

use terrain::heightmap::HeightMap;
use terrain::terrain::{Frac8, Voxel, EdgeCrosses};
use voxel_tree;
use voxel_tree::{VoxelBounds, VoxelTree};

#[cfg(test)]
use test;

fn generate_voxel<FieldContains>(
  timers: &TimerSet,
  field_contains: &mut FieldContains,
  voxel: VoxelBounds,
) -> Option<Voxel>
  where FieldContains: FnMut(f32, f32, f32) -> bool,
{
  timers.time("generate_voxel", || {
    let d = 1 << voxel.lg_size;
    let x1 = voxel.x as f32;
    let x2 = (voxel.x + d) as f32;
    let y1 = voxel.y as f32;
    let y2 = (voxel.y + d) as f32;
    let z1 = voxel.z as f32;
    let z2 = (voxel.z + d) as f32;
    // corners[x][y][z]
    let corners = [
      [
        [ field_contains(x1, y1, z1), field_contains(x1, y1, z2) ],
        [ field_contains(x1, y2, z1), field_contains(x1, y2, z2) ],
      ],
      [
        [ field_contains(x2, y1, z1), field_contains(x2, y1, z2) ],
        [ field_contains(x2, y2, z1), field_contains(x2, y2, z2) ],
      ],
    ];

    let mut facing = [false; 3];

    macro_rules! edge(($f:expr, $x1:expr, $y1:expr, $z1:expr, $x2:expr, $y2:expr, $z2:expr) => {{
      let r = corners[$x1][$y1][$z1] != corners[$x2][$y2][$z2];
      if r && corners[$x1][$y1][$z1] {
        facing[$f] = true;
      }
      r
    }});

    let edges = EdgeCrosses {
      x_edges: [
        [ edge!(0, 0,0,0, 1,0,0), edge!(0, 0,0,1, 1,0,1) ],
        [ edge!(0, 0,1,0, 1,1,0), edge!(0, 0,1,1, 1,1,1) ],
      ],
      y_edges: [
        [ edge!(1, 0,0,0, 0,1,0), edge!(1, 1,0,0, 1,1,0) ],
        [ edge!(1, 0,0,1, 0,1,1), edge!(1, 1,0,1, 1,1,1) ],
      ],
      z_edges: [
        [ edge!(2, 0,0,0, 0,0,1), edge!(2, 1,0,0, 1,0,1) ],
        [ edge!(2, 0,1,0, 0,1,1), edge!(2, 1,1,0, 1,1,1) ],
      ],
    };

    let mut vertex = Vector3::new(0, 0, 0);
    let mut n = 0;
    let half = 0x80;

    for y in range_inclusive(0, 1) {
    for z in range_inclusive(0, 1) {
      if edges.x_edges[y][z] {
        vertex.add_self_v(&Vector3::new(half, y << 8, z << 8));
        n += 1;
      }
    }}

    for x in range_inclusive(0, 1) {
    for z in range_inclusive(0, 1) {
      if edges.y_edges[x][z] {
        vertex.add_self_v(&Vector3::new(x << 8, half, z << 8));
        n += 1;
      }
    }}

    for x in range_inclusive(0, 1) {
    for y in range_inclusive(0, 1) {
      if edges.z_edges[x][y] {
        vertex.add_self_v(&Vector3::new(x << 8, y << 8, half));
        n += 1;
      }
    }}

    if n == 0 {
      return None
    }

    let vertex = vertex.div_s(n);
    let vertex =
      Vector3::new(
        Frac8::of(vertex.x as u8),
        Frac8::of(vertex.y as u8),
        Frac8::of(vertex.z as u8),
      );

    Some(Voxel {
      vertex: vertex,
      edge_crosses: edges,
      facing: facing,
    })
  })
}

#[bench]
fn bench_generate_interesting_voxel(bencher: &mut test::Bencher) {
  let timers = TimerSet::new();
  bencher.iter(|| {
    let voxel = generate_voxel(&timers, VoxelBounds::new(0, 0, 0, 6));
    assert!(voxel.is_some());
  });
}

#[bench]
fn bench_generate_empty_voxel(bencher: &mut test::Bencher) {
  let timers = TimerSet::new();
  bencher.iter(|| {
    let voxel = generate_voxel(&timers, VoxelBounds::new(0, 0, 0, 4));
    assert!(voxel.is_none());
  });
}

pub fn generate_block(
  timers: &TimerSet,
  id_allocator: &Mutex<IdAllocator<EntityId>>,
  heightmap: &HeightMap,
  voxels: &mut VoxelTree<Voxel>,
  position: &BlockPosition,
  lod_index: LODIndex,
) -> TerrainBlock {
  timers.time("update.generate_block", || {
    let mut block = TerrainBlock::empty();

    let position = position.to_world_position();
    let iposition = Point3::new(position.x as i32, position.y as i32, position.z as i32);

    let lateral_samples = LOD_QUALITY[lod_index.0 as usize] as i32;
    assert!(lateral_samples <= BLOCK_WIDTH, "Sub-unit voxels not supported yet.");
    assert!(BLOCK_WIDTH % lateral_samples == 0, "Block width doesn't sample cleanly.");

    let voxel_size = BLOCK_WIDTH / lateral_samples;
    let lg_size = Int::trailing_zeros(voxel_size);
    assert!(1 << lg_size == voxel_size, "voxel_size should be an exponent of 2");
    assert!(lg_size < 31, "2^{} is too a huge voxel.", lg_size);
    let lg_size = lg_size as u8;

    {
      let mut add_poly = |v1: Point3<f32>, v2: Point3<f32>, center: Point3<f32>| {
        let id = id_allocator.lock().unwrap().allocate();

        let minx = partial_min(v1.x, v2.x);
        let minx = minx.and_then(|m| partial_min(m, center.x));
        let minx = minx.unwrap();

        let maxx = partial_max(v1.x, v2.x);
        let maxx = maxx.and_then(|m| partial_max(m, center.x));
        let maxx = maxx.unwrap();

        let miny = partial_min(v1.y, v2.y);
        let miny = miny.and_then(|m| partial_min(m, center.y));
        let miny = miny.unwrap();

        let maxy = partial_max(v1.y, v2.y);
        let maxy = maxy.and_then(|m| partial_max(m, center.y));
        let maxy = maxy.unwrap();

        let minz = partial_min(v1.z, v2.z);
        let minz = minz.and_then(|m| partial_min(m, center.z));
        let minz = minz.unwrap();

        let maxz = partial_max(v1.z, v2.z);
        let maxz = maxz.and_then(|m| partial_max(m, center.z));
        let maxz = maxz.unwrap();

        let norm = Vector3::new(0.0, 1.0, 0.0);

        block.vertex_coordinates.push(tri(v1, v2, center));
        // TODO: Real normals.
        block.normals.push(tri(norm, norm, norm));
        block.ids.push(id);

        block.bounds.push((
          id,
          Aabb3::new(
            // TODO: Remove this - 1.0. It's a temporary hack until voxel collisions work.
            Point3::new(minx, miny - 1.0, minz),
            Point3::new(maxx, maxy, maxz),
          ),
        ));
      };

      let mut field_contains = |x, y, z| {
        heightmap.height_at(x, z) >= y
      };

      macro_rules! get_voxel(($v:expr) => {{
        let bounds = VoxelBounds::new($v.x, $v.y, $v.z, lg_size);
        let branch = voxels.get_mut(bounds);
        match branch {
          &mut voxel_tree::TreeBody::Leaf(v) => Some(v),
          &mut voxel_tree::TreeBody::Empty => {
            // TODO: Add a "yes this is empty I checked" flag so we don't regen every time.
            generate_voxel(timers, &mut field_contains, bounds).map(|v| {
              *branch = voxel_tree::TreeBody::Leaf(v);
              v
            })
          },
          &mut voxel_tree::TreeBody::Branch(_) => {
            // Overwrite existing for now.
            // TODO: Don't do ^that.
            generate_voxel(timers, &mut field_contains, bounds).map(|v| {
              *branch = voxel_tree::TreeBody::Leaf(v);
              v
            })
          },
        }
      }});

      let to_world_vertex = |local: Vector3<Frac8>, voxel_position: Point3<i32>| {
        // Relative position of the vertex.
        let local =
          Vector3::new(
            ((local.x.numerator as i32) << lg_size) as f32 / 256.0,
            ((local.y.numerator as i32) << lg_size) as f32 / 256.0,
            ((local.z.numerator as i32) << lg_size) as f32 / 256.0,
          );
        let fv =
          Point3::new(
            voxel_position.x as f32,
            voxel_position.y as f32,
            voxel_position.z as f32,
          );
        fv.add_v(&local)
      };

      macro_rules! get_vertex(($w:expr) => (
        to_world_vertex(get_voxel!($w).unwrap().vertex, $w)
      ));

      macro_rules! extract((
        $edges:ident,
        $facing:expr,
        $d1:expr,
        $d2:expr,
      ) => (
          for x in 0..lateral_samples {
          for y in 0..lateral_samples {
          for z in 0..lateral_samples {
            let w = iposition.add_v(&Vector3::new(x << lg_size, y << lg_size, z << lg_size));
            let voxel;
            match get_voxel!(w) {
              None => continue,
              Some(v) => voxel = v,
            }

            let crosses_surface = voxel.edge_crosses.$edges[0][0];
            if !crosses_surface {
              continue
            }

            // Make a quad out of the vertices from the 4 voxels adjacent to this edge.
            // We know they have vertices in them because if the surface crosses an edge,
            // it must cross that edge's neighbors.

            let v1 = get_vertex!(w.add_v(&$d1).add_v(&$d2));
            let v2 = get_vertex!(w.add_v(&$d1));
            let v3 = to_world_vertex(voxel.vertex, w);
            let v4 = get_vertex!(w.add_v(&$d2));
            // Put a vertex at the average of the vertices.
            let center =
              v1.add_v(&v2.to_vec()).add_v(&v3.to_vec()).add_v(&v4.to_vec()).div_s(4.0);

            if voxel.facing[$facing] {
              // The polys are visible from positive infinity.
              add_poly(v1, v4, center);
              add_poly(v4, v3, center);
              add_poly(v3, v2, center);
              add_poly(v2, v1, center);
            } else {
              // The polys are visible from negative infinity.
              add_poly(v1, v2, center);
              add_poly(v2, v3, center);
              add_poly(v3, v4, center);
              add_poly(v4, v1, center);
            }
          }}}
        )
      );

      extract!(
        x_edges, 0,
        Vector3::new(0, -voxel_size, 0),
        Vector3::new(0, 0, -voxel_size),
      );

      extract!(
        y_edges, 1,
        Vector3::new(0, 0, -voxel_size),
        Vector3::new(-voxel_size, 0, 0),
      );

      extract!(
        z_edges, 2,
        Vector3::new(0, -voxel_size, 0),
        Vector3::new(-voxel_size, 0, 0),
      );
    }

    block
  })
}
