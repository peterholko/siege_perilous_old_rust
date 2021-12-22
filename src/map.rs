
use bevy::prelude::*;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::convert::TryInto;

use serde::{Deserialize, Serialize};

use tiled::{parse, LayerData};


pub const WIDTH: u32 = 60;
pub const HEIGHT: u32 = 50;

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub enum TileType {
    Grasslands,
    Snow,
    River,
    Ocean,
    Plains,
    HillsPlains,
    Desert,
    Oasis, 
    HillsDesert,
    HillsGrasslands,
    Swamp,
    HillsSnow,
    DeciduousForest,
    Rainforest,
    Jungle,
    Savanna,
    FrozenForest,
    PineForest,
    PalmForest,
    Mountain,
    Volcano,
    Unknown
}

#[derive(Debug, Clone)]
pub struct TileInfo {
    pub tile_type: TileType,
    pub layers: Vec<u32>
}

#[derive(Debug)]
pub struct Map {
    pub width: u32,
    pub height: u32,
    pub base: Vec<TileInfo>
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct MapTile {
    pub x: i32,
    pub y: i32,
    pub t: Vec<u32>
}


impl FromWorld for Map {
    fn from_world(_world: &mut World) -> Self {
        let map: Map = load_map();

        println!("{:?}", map);
        //Return map
        map
    }
}

fn load_map() -> Map {
    let mut map = Map{
        width: WIDTH,
        height: HEIGHT,
        base: Vec::with_capacity(3000)
    };

    let file = File::open(&Path::new("test3.tmx")).unwrap();
    println!("Opened file");
    let reader = BufReader::new(file);
    let raw_map = parse(reader).unwrap();

    for layer in raw_map.layers.iter() {
        println!("layer name: {}", layer.name);

        if layer.name == "base1" {

            if let LayerData::Finite(layer_tiles) = &layer.tiles {
                for row in layer_tiles.iter() {
                    for col in row.iter() {                    

                        let tile = TileInfo {
                            tile_type: gid_to_tiletype(col.gid),
                            layers: vec![col.gid]
                        };

                        map.base.push(tile);
                    }
                }
            }
        } else if layer.name == "base2" {
            let mut index = 0;

            if let LayerData::Finite(layer_tiles) = &layer.tiles {
                for row in layer_tiles.iter() {
                    for col in row.iter() { 
                        
                        //Do not store if tile is 0
                        if col.gid != 0 {                            
                            map.base[index].tile_type = gid_to_tiletype(col.gid);    
                            map.base[index].layers.push(col.gid);
                        }

                        index += 1;
                    }
                }
            }
        }
    }
    
    map
}

pub fn get_neighbour_tiles(center_x: i32, center_y: i32, r: u32, map: Map) -> Vec<MapTile> {

    let mut tiles = Vec::new();
    let neighbours = range((center_x, center_y), r);

    for neighbour in neighbours {
        let (x, y) = neighbour;

        //Reminder tile_index = y * width + x
        let tile_index: usize = (y as usize) * (WIDTH as usize) + (x as usize);
        let layers = map.base[tile_index].layers.clone();

        let tile = MapTile{
            x: x,
            y: y,
            t: layers
        };

        tiles.push(tile);
    }

    tiles
}


fn gid_to_tiletype(gid: u32) -> TileType {
    match gid {
        1 => TileType::Grasslands,
        2 => TileType::Snow,
        3 => TileType::River,
        4 => TileType::River,
        5 => TileType::Ocean,
        6 => TileType::Plains,
        7 => TileType::HillsPlains,
        8 => TileType::HillsPlains,
        9 => TileType::Plains,
        10 => TileType::Desert,
        11 => TileType::Oasis,
        12 => TileType::HillsDesert,
        13 => TileType::HillsGrasslands,
        14 => TileType::Swamp,
        15 => TileType::Swamp,
        16 => TileType::HillsSnow,
        17 => TileType::Ocean,
        18 => TileType::Swamp,
        19 => TileType::DeciduousForest,
        20 => TileType::Rainforest,
        21 => TileType::Jungle,
        22 => TileType::Savanna,
        23 => TileType::DeciduousForest,
        24 => TileType::DeciduousForest,
        25 => TileType::FrozenForest,
        26 => TileType::FrozenForest,
        27 => TileType::PineForest,
        28 => TileType::FrozenForest,
        29 => TileType::Savanna,
        30 => TileType::PalmForest,
        31 => TileType::Jungle,
        32 => TileType::Mountain,
        33 => TileType::Mountain,
        34 => TileType::Mountain,
        35 => TileType::Mountain,
        36 => TileType::Mountain,
        37 => TileType::Mountain,
        38 => TileType::Mountain,
        39 => TileType::Volcano,
        _ => TileType::Unknown
    }
}

fn odd_q_to_cube((q, r): (i32, i32)) -> (i32, i32, i32) {
    let x = q;
    let z = r - (q - (q & 1)) / 2;
    let y = -x-z;
    (x, y, z)
}

fn cube_to_odd_q((x, y, z): (i32, i32, i32)) -> (i32, i32) {
    let q = x;
    let r = z + (x - (x & 1)) / 2;
    (q, r)
}

fn distance(src_pos: (i32, i32), dst_pos: (i32, i32)) -> i32 {
    let (sx, sy, sz) = odd_q_to_cube(src_pos);
    let (dx, dy, dz) = odd_q_to_cube(dst_pos);
    
    ((sx - dx).abs() + (sy - dy).abs() + (sz - dz).abs()) / 2
}

pub fn range((q, r): (i32, i32), num: u32) -> Vec<(i32, i32)> {
    let n = num as i32;

    let mut result: Vec<(i32, i32)> = Vec::new();

    let (cx, cy, cz) = odd_q_to_cube((q, r));

    //TODO could be optimized as per Amit's hex guide
    for sx in -n..=n {
        for sy in -n..=n { 
            for sz in -n..=n {
                if (cx + sx) + (cy + sy) + (cz + sz) == 0 {   
                    let pos = cube_to_odd_q(((cx + sx), (cy + sy), (cz + sz)));
                    
                    if is_valid_pos(pos) {
                        result.push(pos);
                    }
                }
            }
        }
    }

    result
}

fn neighbours((q, r): (i32, i32)) -> Vec<(i32, i32)> {
    let neighbours_table: Vec<(i32, i32, i32)> = vec![(1,-1,0), (1,0,-1), (0,1,-1), (-1,1,0), (-1,0,1), (0,-1,1)];

    let mut result: Vec<(i32, i32)> = Vec::new();

    let (x, y, z) = odd_q_to_cube((q, r));

    for (nx, ny, nz) in neighbours_table {
        let neighbour_cube = (x + nx, y + ny, z + nz);
        let neighbour_odd = cube_to_odd_q(neighbour_cube);

        if is_valid_pos(neighbour_odd) {
            result.push(neighbour_odd);
        }
    }

    result
}

fn is_valid_pos((q, r): (i32, i32)) -> bool {
    q >= 0 && r >= 0 && q < (WIDTH as i32) && r < (HEIGHT as i32)
}

#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;    

    #[test]
    fn test_load_map() {
        let map: Map = load_map();

        let tile_index : usize = (35 * WIDTH + 17).try_into().unwrap();
                
        assert_eq!(map.base[tile_index].layers, vec![13]);
        assert_eq!(map.base[tile_index].tile_type, TileType::HillsGrasslands);

    }

     #[test]
    fn test_get_neighbour_tiles() {
        let map: Map = load_map();

        let mut tiles = get_neighbour_tiles(16, 36, 2, map);

        let test_tiles = r#"[{"t":[13],"x":17,"y":34},{"t":[1],"x":16,"y":37},{"t":[1],"x":16,"y":35},{"t":[13],"x":18,"y":36},{"t":[5],"x":15,"y":36},{"t":[1],"x":17,"y":37},{"t":[1],"x":15,"y":34},{"t":[5],"x":14,"y":37},{"t":[13],"x":17,"y":35},{"t":[1],"x":16,"y":38},{"t":[1],"x":14,"y":35},{"t":[1],"x":16,"y":36},{"t":[1],"x":18,"y":37},{"t":[13],"x":16,"y":34},{"t":[5],"x":15,"y":37},{"t":[1],"x":18,"y":35},{"t":[13],"x":15,"y":35},{"t":[1],"x":17,"y":36},{"t":[13],"x":14,"y":36}]"#;

        let deserialized_test_tiles: Vec<MapTile>  = serde_json::from_str(&test_tiles).unwrap();

        println!("{:?}", deserialized_test_tiles);

        for tile in tiles {
            assert_eq!(deserialized_test_tiles.contains(&tile), true);
        }
    }
    

    #[test]
    fn test_odd_q_to_cube() {
        let odd_q = (2, 1);

        assert_eq!(odd_q_to_cube(odd_q), (2, -2, 0));

        let odd_q = (-1, 2);
        assert_eq!(odd_q_to_cube(odd_q), (-1, -2, 3));
    }

    #[test]
    fn test_cube_to_odd_q() {
        let cube = (2, -2, 0);

        assert_eq!(cube_to_odd_q(cube), (2, 1));

        let cube = (-1, -2, 3);
        assert_eq!(cube_to_odd_q(cube), (-1, 2));
    }    

    #[test]
    fn test_distance() {
        let src_pos = (2, 4);
        let dst_pos = (5, 3);

        assert_eq!(distance(src_pos, dst_pos), 3);

        let src_pos = (2, 4);
        let dst_pos = (5, 5);

        assert_eq!(distance(src_pos, dst_pos), 3);

        let src_pos = (2, 4);
        let dst_pos = (5, 6);

        assert_eq!(distance(src_pos, dst_pos), 4);

        let src_pos = (-2, 4);
        let dst_pos = (5, 6); 
        
        assert_eq!(distance(src_pos, dst_pos), 7);
    }   
    

    #[test]
    fn test_range() {
        //[{2,3},{2,2},{3,3},{3,2},{3,1},{4,3},{4,2}]
        let result = vec![(2,3), (2,2), (3,3), (3,2), (3,1), (4,3), (4,2)];

        assert_eq!(range((3, 2), 1), result);

        //[{0,2},{0,1},{0,0},{1,1},{1,0}]
        let result = vec![(0,2),(0,1),(0,0),(1,1),(1,0)];

        assert_eq!(range((0, 1), 1), result);
    }      

    #[test]
    fn test_neighbours() {
        //[{1,2},{0,2},{0,1},{1,0},{2,1},{2,2}]
        let result = vec![(2, 2), (2, 1), (1, 0), (0, 1), (0, 2), (1, 2)];

        assert_eq!(neighbours((1, 1)), result);

        //[{0,1},{1,0}]
        let result = vec![(1,0),(0,1)];

        assert_eq!(neighbours((0,0)), result);
    }         
}