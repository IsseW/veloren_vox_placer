use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
};

use dot_vox::{DotVoxData, Model};
use rand::{thread_rng, Rng};
use serde::Deserialize;
use vek::*;
use veloren_common::{
    assets::{Asset, AssetExt, AssetHandle, DotVoxAsset, RonLoader},
    figure::Cell,
    lottery::Lottery,
    terrain::{Block, BlockKind, SpriteKind},
    vol::{IntoFullVolIterator, VolSize, WriteVol},
    volumes::{chunk::Chunk, vol_grid_3d::VolGrid3d},
};
use veloren_server::terrain_persistence::TerrainPersistence;

#[derive(Clone, Debug)]
pub struct SscSize;
impl VolSize for SscSize {
    const SIZE: Vec3<u32> = Vec3 {
        x: 32,
        y: 32,
        z: 32,
    };
}

struct SparseScene(VolGrid3d<Chunk<Cell, SscSize, ()>>);
impl Deref for SparseScene {
    type Target = VolGrid3d<Chunk<Cell, SscSize, ()>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for SparseScene {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl SparseScene {
    pub fn new_from<'a>(
        dot_vox_data: impl Iterator<Item = (assets_manager::AssetGuard<'a, DotVoxAsset>, Vec3<i32>)>,
    ) -> (Self, Vec<Aabb<i32>>) {
        fn render_model(
            palette: &Vec<Rgb<u8>>,
            model: &Model,
            sparse_scene: &mut SparseScene,
            aabbs: &mut Vec<Aabb<i32>>,
            rot: Mat3<i32>,
            trans: Vec3<i32>,
        ) {
            // Get the rotated size of the model
            let size =
                rot.map(|e| e.abs() as u32) * Vec3::new(model.size.x, model.size.y, model.size.z);
            // Position of min corner
            let pos = trans
                .map2(size, |m, s| (s, m))
                .map2(rot * Vec3::<i32>::one(), |(s, m), f| {
                    m - (s as i32 + f.min(0) * -1) / 2
                });
            let model_bounds = Aabb {
                min: pos,
                // vek Aabbs are inclusive
                max: pos + size.map(|e| e as i32) - 1,
            };
            if !aabbs.iter_mut().any(|aabb| {
                (if model_bounds.contains_aabb(*aabb) {
                    *aabb = model_bounds;
                    true
                } else {
                    false
                }) || aabb.contains_aabb(model_bounds)
            }) {
                aabbs.push(model_bounds);
            }
            // dbg!(pos);
            // Insert required chunks
            let min_key = sparse_scene.pos_key(pos);
            let max_key = sparse_scene.pos_key(pos + size.map(|e| e as i32 - 1));
            for x in min_key.x..=max_key.x {
                for y in min_key.y..=max_key.y {
                    for z in min_key.z..=max_key.z {
                        let key = Vec3::new(x, y, z);
                        if sparse_scene.get_key_arc(key).is_none() {
                            sparse_scene
                                .insert(key, std::sync::Arc::new(Chunk::filled(Cell::Empty, ())));
                        }
                    }
                }
            }
            let offset = (rot
                * Vec3::new(model.size.x, model.size.y, model.size.z).map(|e| e as i32))
            .map(|e| if e > 0 { 0 } else { -e - 1 });
            for voxel in &model.voxels {
                if let Some(&color) = palette.get(voxel.i as usize) {
                    sparse_scene
                        .set(
                            (rot * Vec3::new(voxel.x, voxel.y, voxel.z).map(|e| i32::from(e)))
                                + offset
                                + pos,
                            Cell::new(color, false, false, voxel.i == 16),
                        )
                        .unwrap();
                }
            }
        }

        fn insert_scene(
            dot_vox_data: &DotVoxData,
            palette: &Vec<Rgb<u8>>,
            scene: u32,
            mut rot: Mat3<i32>,
            mut trans: Vec3<i32>,
            sparse_scene: &mut SparseScene,
            aabbs: &mut Vec<Aabb<i32>>,
        ) {
            let scene = dot_vox_data.scenes.get(scene as usize).unwrap();
            match scene {
                dot_vox::SceneNode::Transform { frames, child, .. } => {
                    if let Some(frame) = frames.get(0) {
                        let t = frame
                            .position()
                            .and_then(|t| Some(Vec3::new(t.x, t.y, t.z)))
                            .unwrap_or_default();

                        let r = frame
                            .orientation()
                            .map(|r| {
                                let arr = r.to_cols_array_2d();
                                Mat3::from_col_arrays(arr).map(|f| f as i32)
                            })
                            .unwrap_or(Mat3::identity());

                        trans += rot * t;
                        rot *= r;
                    }

                    insert_scene(
                        dot_vox_data,
                        palette,
                        *child,
                        rot,
                        trans,
                        sparse_scene,
                        aabbs,
                    );
                }
                dot_vox::SceneNode::Group { children, .. } => {
                    for child in children {
                        insert_scene(
                            dot_vox_data,
                            palette,
                            *child,
                            rot,
                            trans,
                            sparse_scene,
                            aabbs,
                        );
                    }
                }
                dot_vox::SceneNode::Shape { models, .. } => {
                    for model in models {
                        if let Some(model) = dot_vox_data.models.get(model.model_id as usize) {
                            render_model(palette, model, sparse_scene, aabbs, rot, trans);
                        }
                    }
                }
            }
        }

        let mut sparse_scene = SparseScene(match VolGrid3d::new() {
            Ok(ok) => ok,
            Err(_) => panic!(),
        });
        let mut aabbs = Vec::new();
        for (dot_vox_data, offset) in dot_vox_data {
            let palette = dot_vox_data
                .0
                .palette
                .iter()
                .map(|col| Rgb::new(col.r, col.g, col.b))
                .collect::<Vec<_>>();
            // Zero is always the root node.
            insert_scene(
                &dot_vox_data.0,
                &palette,
                0,
                Mat3::identity(),
                offset,
                &mut sparse_scene,
                &mut aabbs,
            );
        }

        (sparse_scene, aabbs)
    }
}

#[derive(Deserialize, Default, Clone, Copy)]
enum Medium {
    #[default]
    Air,
    Water,
}

#[derive(Deserialize, Clone)]
enum BlockSpec {
    Sprite {
        kind: SpriteKind,
        #[serde(default)]
        medium: Medium,
    },
    Block {
        kind: BlockKind,
        #[serde(default)]
        color: [u8; 3],
    },
    Random(Lottery<BlockSpec>),
}

impl BlockSpec {
    fn get_block(&self, rng: &mut impl Rng) -> Block {
        match self {
            BlockSpec::Sprite { kind, medium } => match medium {
                Medium::Air => Block::air(*kind),
                Medium::Water => Block::water(*kind),
            },
            BlockSpec::Block { kind, color } => Block::new(*kind, Rgb::from(*color)),
            BlockSpec::Random(lottery) => lottery.choose_seeded(rng.gen()).get_block(rng),
        }
    }
}

#[derive(Deserialize)]
struct VoxSpec(String, [i32; 3]);

#[derive(Deserialize)]
struct PlaceSpec {
    pieces: Vec<VoxSpec>,
    #[serde(default)]
    replace: Vec<([u8; 3], BlockSpec)>,
    #[serde(default)]
    fill_empty: bool,
}

impl PlaceSpec {
    // pub fn load_watched() -> std::sync::Arc<Self> {
    //     PlaceSpec::load("place")
    // }

    pub fn build_place(&self) -> ((SparseScene, Vec<Aabb<i32>>), Vec3<i32>) {
        // TODO add sparse scene combination
        //use common::figure::{DynaUnionizer, Segment};
        fn graceful_load_vox(name: &str) -> AssetHandle<DotVoxAsset> {
            match DotVoxAsset::load(name) {
                Ok(dot_vox) => dot_vox,
                Err(_) => {
                    println!("Could not load vox file for placement: {}", name);
                    DotVoxAsset::load_expect("voxygen.voxel.not_found")
                }
            }
        }
        //let mut unionizer = DynaUnionizer::new();
        //for VoxSpec(specifier, offset) in &self.pieces {
        //    let seg = Segment::from(graceful_load_vox(&specifier,
        // indicator).as_ref());    unionizer = unionizer.add(seg,
        // (*offset).into());
        //}

        //unionizer.unify()
        (
            SparseScene::new_from(self.pieces.iter().map(|spec| {
                let vox = graceful_load_vox(&spec.0).read();
                let offset = Vec3::<i32>::from(spec.1);
                (vox, offset)
            })),
            Vec3::zero(),
        )
    }
}

impl Asset for PlaceSpec {
    type Loader = RonLoader;

    const EXTENSION: &'static str = "ron";
}

fn main() {
    let mut persistance = TerrainPersistence::new("./terrain/".into());
    let mut rng = thread_rng();
    let place_spec = PlaceSpec::load_expect("place").read();
    let ((vox, aabbs), _) = place_spec.build_place();
    let replace_map = place_spec
        .replace
        .iter()
        .map(|(color, block)| (Rgb::from(*color), block.clone()))
        .collect::<HashMap<_, _>>();
    for (key, chunk) in vox.iter() {
        println!("Filling chunk {}", key);
        for (pos, cell) in chunk.full_vol_iter() {
            let wpos = vox.key_pos(key) + pos;
            if place_spec.fill_empty {
                if !aabbs.iter().any(|aabb| aabb.contains_point(pos)) {
                    continue;
                }
            } else if matches!(cell, Cell::Empty) {
                continue;
            }
            let block = match cell.get_color() {
                Some(color) => replace_map
                    .get(&color)
                    .map(|spec| spec.get_block(&mut rng))
                    .unwrap_or_else(|| {
                        if cell.is_hollow() {
                            Block::air(SpriteKind::Empty)
                        } else if cell.is_glowy() {
                            Block::new(BlockKind::GlowingRock, color)
                        } else if cell.is_shiny() {
                            Block::water(SpriteKind::Empty)
                        } else {
                            Block::new(BlockKind::Misc, color)
                        }
                    }),
                None => Block::empty(),
            };
            persistance.set_block(wpos, block);
        }
    }

    persistance.unload_all();
}
