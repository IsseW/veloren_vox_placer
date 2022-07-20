use std::ops::{Deref, DerefMut};

use dot_vox::{DotVoxData, Model};
use rand::Rng;
use serde::{Deserialize, Serialize};
use vek::*;
use veloren_common::{
    assets::{Asset, AssetExt, AssetHandle, DotVoxAsset, RonLoader},
    figure::Cell,
    vol::{VolSize, Vox, WriteVol, IntoFullVolIterator},
    volumes::{chunk::Chunk, vol_grid_3d::VolGrid3d}, terrain::{SpriteKind, Block, BlockKind},
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
    pub fn new_from(dot_vox_data: &DotVoxData, offset: Vec3<i32>) -> (Self, Vec<Aabb<i32>>) {
        let mut sparse_scene = SparseScene(match VolGrid3d::new() {
            Ok(ok) => ok,
            Err(_) => panic!(),
        });
        let mut aabbs = Vec::new();
        let palette = dot_vox_data
            .palette
            .iter()
            .map(|col| Rgba::from(col.to_ne_bytes()).into())
            .collect::<Vec<_>>();
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
            aabbs.push(Aabb {
                min: pos,
                max: pos + size.map(|e| e as i32),
            });
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
                                .insert(key, std::sync::Arc::new(Chunk::filled(Cell::empty(), ())));
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
            let scene = dot_vox_data.scene.get(scene as usize).unwrap();
            match scene {
                dot_vox::SceneNode::Transform { frames, child, .. } => {
                    if let Some(frame) = frames.get(0) {
                        let t = frame
                            .get("_t")
                            .and_then(|t| {
                                let mut elements = t.split(' ').map(|e| e.parse::<i32>().ok());
                                Some(Vec3::new(
                                    elements.next()??,
                                    elements.next()??,
                                    elements.next()??,
                                ))
                            })
                            .unwrap_or_default();

                        let r = frame
                            .get("_r")
                            .and_then(|r| {
                                let n = r.parse::<u8>().ok()?;
                                let signs = [
                                    if n >> 4 & 1 == 0 { 1 } else { -1 },
                                    if n >> 5 & 1 == 0 { 1 } else { -1 },
                                    if n >> 6 & 1 == 0 { 1 } else { -1 },
                                ];
                                let rows = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
                                if n & 3 != 3 && n >> 2 != 3 {
                                    let r1 = rows[(n & 3) as usize];
                                    let r2 = rows[(n >> 2 & 3) as usize];
                                    let r3 = rows[(!(n | (n >> 2)) & 3) as usize];

                                    Some(Mat3::new(
                                        r1[0] * signs[0],
                                        r1[1] * signs[0],
                                        r1[2] * signs[0],
                                        r2[0] * signs[1],
                                        r2[1] * signs[1],
                                        r2[2] * signs[1],
                                        r3[0] * signs[2],
                                        r3[1] * signs[2],
                                        r3[2] * signs[2],
                                    ))
                                } else {
                                    // Unknown format
                                    None
                                }
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

        // Zero is always the root node.
        insert_scene(
            dot_vox_data,
            &palette,
            0,
            Mat3::identity(),
            offset,
            &mut sparse_scene,
            &mut aabbs,
        );

        (sparse_scene, aabbs)
    }
}

#[derive(Serialize, Deserialize)]
struct VoxSpec(String, [i32; 3]);

#[derive(Serialize, Deserialize)]
struct PlaceSpec {
    pieces: Vec<VoxSpec>,
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
        let hack = "asset that doesn't exist";
        (
            match self.pieces.get(0) {
                Some(VoxSpec(specifier, offset)) => {
                    SparseScene::new_from(&graceful_load_vox(&specifier).read().0, Vec3::from(*offset))
                }
                None => SparseScene::new_from(&graceful_load_vox(&hack).read().0, Vec3::zero()),
            },
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

    let (vox, aabbs) = PlaceSpec::load_expect("place").read().build_place().0;
    for (key, chunk) in vox.iter() {
        println!("Filling chunk {}", key);
        for (pos, cell) in chunk.full_vol_iter() {
            let wpos = vox.key_pos(key) + pos;
            if !aabbs.iter().any(|aabb| aabb.contains_point(wpos)) {
                continue;
            }
            match cell.get_color() {
                Some(color) => {
                    let block = match color {
                        Rgb::<u8> {
                            r: 4,
                            g: 119,
                            b: 191,
                        } => Block::new(BlockKind::Water, Rgb::default()),
                        Rgb::<u8> {
                            r: 170,
                            g: 56,
                            b: 56,
                        } => Block::new(BlockKind::Lava, Rgb::new(255, 65, 0)),
                        Rgb::<u8> {
                            r: 243,
                            g: 255,
                            b: 113,
                        } => Block::air(SpriteKind::StreetLamp),
                        Rgb::<u8> {
                            r: 0,
                            g: 200,
                            b: 80,
                        } => Block::air(SpriteKind::Liana),
                        Rgb::<u8> {
                            r: 191,
                            g: 255,
                            b: 0,
                        } => Block::air(SpriteKind::CookingPot),
                        Rgb::<u8> {
                            r: 63,
                            g: 96,
                            b: 12,
                        } => {
                            if rand::thread_rng().gen_bool(0.5) {
                                Block::air(SpriteKind::JungleRedGrass)
                            } else {
                                Block::air(SpriteKind::JungleFern)
                            }
                        }
                        Rgb::<u8> {
                            r: 144,
                            g: 31,
                            b: 31,
                        } => Block::air(SpriteKind::DungeonChest4),
                        Rgb::<u8> {
                            r: 194,
                            g: 231,
                            b: 147,
                        } => Block::air(SpriteKind::FireBowlGround),
                        _ => {
                            if cell.is_hollow() {
                                Block::air(SpriteKind::Empty)
                            } else if cell.is_glowy() {
                                Block::new(BlockKind::GlowingRock, color)
                            } else if cell.is_shiny() {
                                Block::water(SpriteKind::Empty)
                            } else {
                                Block::new(BlockKind::Misc, color)
                            }
                        }
                    };
                    persistance.set_block(wpos, block);
                }
                None => {
                    persistance.set_block(wpos, Block::empty());
                }
            }
        }
    }

    persistance.unload_all();
}
