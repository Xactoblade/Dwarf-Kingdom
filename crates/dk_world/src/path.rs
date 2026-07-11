//! Pathfinding: A* over walkable tiles plus a region-connectivity cache.
//!
//! The region cache is the load-bearing piece (BLUEPRINT.md §4.4): before any
//! A*, callers check `Regions::same_region(a, b)` for an O(1) rejection of
//! impossible jobs. Phase 1 rebuilds the whole cache when terrain changes
//! (~100k tiles, sub-millisecond); incremental updates come with chunking.

use crate::{Map, TileShape};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Pos {
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        Pos { x, y, z }
    }

    pub fn manhattan(self, other: Pos) -> u32 {
        self.x.abs_diff(other.x) + self.y.abs_diff(other.y) + self.z.abs_diff(other.z)
    }
}

const HORIZONTAL: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

/// All positions a walker can step to from `p` in one move.
pub fn neighbors(map: &Map, p: Pos, out: &mut Vec<Pos>) {
    out.clear();
    let here = map.tile_at(p).map(|t| t.shape);
    for (dx, dy) in HORIZONTAL {
        let q = Pos::new(p.x + dx, p.y + dy, p.z);
        if map.walkable(q) {
            out.push(q);
        }
        // Up a ramp: standing on a ramp, step onto an adjacent tile one up.
        if here == Some(TileShape::Ramp) {
            let up = Pos::new(q.x, q.y, p.z + 1);
            if map.walkable(up) {
                out.push(up);
            }
        }
        // Down onto a ramp one level below (the reverse edge).
        let down = Pos::new(q.x, q.y, p.z - 1);
        if map.tile_at(down).is_some_and(|t| t.shape == TileShape::Ramp) {
            out.push(down);
        }
    }
    // Vertical movement only through aligned stair tiles.
    if here == Some(TileShape::Stairs) {
        for dz in [-1, 1] {
            let q = Pos::new(p.x, p.y, p.z + dz);
            if map.tile_at(q).is_some_and(|t| t.shape == TileShape::Stairs) {
                out.push(q);
            }
        }
    }
}

/// Positions from which a dwarf can work on `target` (a designated tile):
/// orthogonally adjacent walkable tiles, the tile itself if walkable
/// (carving stairs into an existing floor), or stairs directly above
/// (digging the staircase downward).
pub fn work_positions(map: &Map, target: Pos, out: &mut Vec<Pos>) {
    out.clear();
    if map.walkable(target) {
        out.push(target);
    }
    for (dx, dy) in HORIZONTAL {
        let q = Pos::new(target.x + dx, target.y + dy, target.z);
        if map.walkable(q) {
            out.push(q);
        }
    }
    let above = Pos::new(target.x, target.y, target.z + 1);
    if map.tile_at(above).is_some_and(|t| t.shape == TileShape::Stairs) {
        out.push(above);
    }
}

/// Connected-component labels over all walkable tiles. 0 = not walkable.
#[derive(Debug, Default)]
pub struct Regions {
    ids: Vec<u32>,
    width: usize,
    height: usize,
    depth: usize,
    pub dirty: bool,
}

impl Regions {
    pub fn new(map: &Map) -> Self {
        let mut r = Regions {
            ids: Vec::new(),
            width: 0,
            height: 0,
            depth: 0,
            dirty: false,
        };
        r.rebuild(map);
        r
    }

    pub fn rebuild(&mut self, map: &Map) {
        self.width = map.width;
        self.height = map.height;
        self.depth = map.depth;
        self.ids = vec![0; map.width * map.height * map.depth];
        self.dirty = false;

        let mut next_id = 1u32;
        let mut queue = VecDeque::new();
        let mut scratch = Vec::with_capacity(6);
        for z in 0..map.depth as i32 {
            for y in 0..map.height as i32 {
                for x in 0..map.width as i32 {
                    let p = Pos::new(x, y, z);
                    if !map.walkable(p) || self.id(p) != 0 {
                        continue;
                    }
                    // BFS-label this component.
                    let label = next_id;
                    next_id += 1;
                    self.set_id(p, label);
                    queue.push_back(p);
                    while let Some(cur) = queue.pop_front() {
                        neighbors(map, cur, &mut scratch);
                        for &n in &scratch {
                            if self.id(n) == 0 {
                                self.set_id(n, label);
                                queue.push_back(n);
                            }
                        }
                    }
                }
            }
        }
    }

    #[inline]
    fn index(&self, p: Pos) -> Option<usize> {
        if p.x < 0 || p.y < 0 || p.z < 0 {
            return None;
        }
        let (x, y, z) = (p.x as usize, p.y as usize, p.z as usize);
        if x >= self.width || y >= self.height || z >= self.depth {
            return None;
        }
        Some((z * self.height + y) * self.width + x)
    }

    pub fn id(&self, p: Pos) -> u32 {
        self.index(p).map_or(0, |i| self.ids[i])
    }

    fn set_id(&mut self, p: Pos, id: u32) {
        let i = self.index(p).expect("set_id out of bounds");
        self.ids[i] = id;
    }

    /// O(1) "is a path even possible" check. Both must be walkable.
    pub fn same_region(&self, a: Pos, b: Pos) -> bool {
        let ia = self.id(a);
        ia != 0 && ia == self.id(b)
    }
}

/// Admissible A* heuristic: a ramp step changes x/y and z together at cost 1,
/// so plain manhattan (which sums them) can overestimate. Each move reduces
/// horizontal distance by at most 1 AND vertical distance by at most 1.
fn heuristic(a: Pos, b: Pos) -> u32 {
    (a.x.abs_diff(b.x) + a.y.abs_diff(b.y)).max(a.z.abs_diff(b.z))
}

/// A* shortest path. Returns the tile sequence from `start` (exclusive) to
/// `goal` (inclusive), or None. Callers should gate on `Regions::same_region`
/// first; `max_nodes` is a safety valve, not the primary guard.
pub fn astar(map: &Map, start: Pos, goal: Pos, max_nodes: usize) -> Option<Vec<Pos>> {
    if start == goal {
        return Some(Vec::new());
    }
    let mut open: BinaryHeap<Reverse<(u32, u32, Pos)>> = BinaryHeap::new();
    let mut best: HashMap<Pos, (u32, Pos)> = HashMap::new(); // pos -> (g, parent)
    open.push(Reverse((heuristic(start, goal), 0, start)));
    best.insert(start, (0, start));

    let mut scratch = Vec::with_capacity(6);
    let mut expanded = 0usize;
    while let Some(Reverse((_, g, cur))) = open.pop() {
        if cur == goal {
            // Reconstruct.
            let mut path = vec![cur];
            let mut node = cur;
            while let Some(&(_, parent)) = best.get(&node) {
                if parent == node {
                    break;
                }
                path.push(parent);
                node = parent;
            }
            path.pop(); // drop `start`
            path.reverse();
            return Some(path);
        }
        // Stale queue entry?
        if best.get(&cur).is_some_and(|&(bg, _)| bg < g) {
            continue;
        }
        expanded += 1;
        if expanded > max_nodes {
            return None;
        }
        neighbors(map, cur, &mut scratch);
        for &n in &scratch {
            let ng = g + 1;
            if best.get(&n).is_none_or(|&(bg, _)| ng < bg) {
                best.insert(n, (ng, cur));
                open.push(Reverse((ng + heuristic(n, goal), ng, n)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tile;

    /// A 8x8x4 map: solid at z=0, floor at z=1 everywhere.
    fn flat_map() -> Map {
        let mut m = Map::new_air(8, 8, 4, 0);
        for y in 0..8 {
            for x in 0..8 {
                m.set(x, y, 0, Tile::solid(0));
                m.set(x, y, 1, Tile::floor(0));
            }
        }
        m
    }

    #[test]
    fn astar_walks_around_walls() {
        let mut m = flat_map();
        // Wall across x=4 except a gap at y=7.
        for y in 0..7 {
            m.set(4, y, 1, Tile::solid(0));
        }
        let path = astar(&m, Pos::new(0, 0, 1), Pos::new(7, 0, 1), 10_000).unwrap();
        assert_eq!(*path.last().unwrap(), Pos::new(7, 0, 1));
        assert!(path.iter().any(|p| p.y == 7), "must detour through the gap");
    }

    #[test]
    fn stairs_connect_z_levels() {
        let mut m = flat_map();
        // Carve a stair shaft at (2,2): z1 and a floor + stair at z2.
        m.set(2, 2, 1, Tile { material: 0, shape: TileShape::Stairs, water: 0, magma: 0 });
        m.set(2, 2, 2, Tile { material: 0, shape: TileShape::Stairs, water: 0, magma: 0 });
        m.set(3, 2, 2, Tile::floor(0));
        let path = astar(&m, Pos::new(0, 0, 1), Pos::new(3, 2, 2), 10_000).unwrap();
        assert_eq!(*path.last().unwrap(), Pos::new(3, 2, 2));

        let regions = Regions::new(&m);
        assert!(regions.same_region(Pos::new(0, 0, 1), Pos::new(3, 2, 2)));
    }

    #[test]
    fn regions_separate_disconnected_areas() {
        let mut m = flat_map();
        // Full wall across x=4, no gap.
        for y in 0..8 {
            m.set(4, y, 1, Tile::solid(0));
        }
        let regions = Regions::new(&m);
        assert!(!regions.same_region(Pos::new(0, 0, 1), Pos::new(7, 0, 1)));
        assert!(regions.same_region(Pos::new(0, 0, 1), Pos::new(3, 7, 1)));
        assert!(astar(&m, Pos::new(0, 0, 1), Pos::new(7, 0, 1), 10_000).is_none());
    }

    #[test]
    fn ramps_connect_plateaus() {
        // Two plateaus: floor at z1 (x<4), floor at z2 (x>=4), ramp at (3,*,z1).
        let mut m = Map::new_air(8, 8, 4, 0);
        for y in 0..8 {
            for x in 0..8 {
                m.set(x, y, 0, Tile::solid(0));
                if x < 4 {
                    m.set(x, y, 1, Tile::floor(0));
                } else {
                    m.set(x, y, 1, Tile::solid(0));
                    m.set(x, y, 2, Tile::floor(0));
                }
            }
            m.set(3, y, 1, Tile { material: 0, shape: TileShape::Ramp, water: 0, magma: 0 });
        }
        let low = Pos::new(0, 0, 1);
        let high = Pos::new(7, 7, 2);
        let regions = Regions::new(&m);
        assert!(regions.same_region(low, high), "ramp must join the plateaus");
        let path = astar(&m, low, high, 10_000).unwrap();
        assert_eq!(*path.last().unwrap(), high);
    }

    #[test]
    fn work_positions_include_stairs_above() {
        let mut m = flat_map();
        m.set(2, 2, 1, Tile { material: 0, shape: TileShape::Stairs, water: 0, magma: 0 });
        let mut out = Vec::new();
        // Target: solid tile below the stair (digging downward).
        work_positions(&m, Pos::new(2, 2, 0), &mut out);
        assert!(out.contains(&Pos::new(2, 2, 1)));
    }
}
