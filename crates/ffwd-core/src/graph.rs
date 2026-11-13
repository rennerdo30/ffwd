//! Commit graph lanes, computed incrementally as history streams in.
//!
//! Each row records the segments drawn in its top half (from the previous
//! row's lanes into this row's node) and bottom half (from the node out to
//! the next row's lanes). Lanes keep their column while active, so
//! appending rows never changes rows already laid out.

use crate::log::{Commit, Oid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seg {
    pub from: u16,
    pub to: u16,
    pub color: u16,
    /// `true` for the top half (previous row to this row's mid line).
    pub top: bool,
}

#[derive(Clone, Debug, Default)]
pub struct GraphRow {
    pub node: u16,
    pub color: u16,
    pub segs: Vec<Seg>,
    /// Lanes in use across this row (for the graph column width).
    pub width: u16,
    pub is_merge: bool,
}

#[derive(Default)]
pub struct Graph {
    lanes: Vec<Option<(Oid, u16)>>,
    next_color: u16,
    pub rows: Vec<GraphRow>,
    pub max_width: u16,
}

impl Graph {
    pub fn new() -> Graph {
        Graph::default()
    }

    fn free_slot(&mut self, from: usize) -> usize {
        if let Some(i) = (from..self.lanes.len()).find(|&i| self.lanes[i].is_none()) {
            return i;
        }
        self.lanes.push(None);
        self.lanes.len() - 1
    }

    fn color(&mut self) -> u16 {
        let c = self.next_color;
        self.next_color = self.next_color.wrapping_add(1);
        c
    }

    pub fn push(&mut self, c: &Commit) {
        let mut segs = Vec::new();
        let hits: Vec<usize> = self
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| l.is_some_and(|(id, _)| id == c.id))
            .map(|(i, _)| i)
            .collect();
        let (node, color) = match hits.first() {
            Some(&i) => (i, self.lanes[i].map_or(0, |l| l.1)),
            None => {
                let i = self.free_slot(0);
                let col = self.color();
                (i, col)
            }
        };
        for (i, l) in self.lanes.iter().enumerate() {
            if let Some((id, col)) = l {
                let to = if *id == c.id { node } else { i };
                segs.push(Seg { from: i as u16, to: to as u16, color: *col, top: true });
            }
        }
        for &h in &hits {
            self.lanes[h] = None;
        }

        let mut fresh = Vec::new();
        for (pi, p) in c.parents.iter().enumerate() {
            if let Some(j) = self.lanes.iter().position(|l| l.is_some_and(|(id, _)| id == *p)) {
                // Another child already waits for this parent: join its lane.
                let col = self.lanes[j].map_or(color, |l| l.1);
                segs.push(Seg {
                    from: node as u16,
                    to: j as u16,
                    color: if pi == 0 { color } else { col },
                    top: false,
                });
                continue;
            }
            let (slot, col) = if pi == 0 && self.lanes[node].is_none() {
                (node, color)
            } else {
                let s = self.free_slot(node + 1);
                let col = self.color();
                (s, col)
            };
            self.lanes[slot] = Some((*p, col));
            fresh.push(slot);
            segs.push(Seg { from: node as u16, to: slot as u16, color: col, top: false });
        }
        for (i, l) in self.lanes.iter().enumerate() {
            if let Some((_, col)) = l
                && !fresh.contains(&i)
            {
                segs.push(Seg { from: i as u16, to: i as u16, color: *col, top: false });
            }
        }
        while self.lanes.last().is_some_and(|l| l.is_none()) {
            self.lanes.pop();
        }
        let width = segs.iter().map(|s| s.from.max(s.to) + 1).max().unwrap_or(1).max(node as u16 + 1);
        self.max_width = self.max_width.max(width);
        self.rows.push(GraphRow { node: node as u16, color, segs, width, is_merge: c.parents.len() > 1 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(n: u8) -> Oid {
        let hex = format!("{:040x}", n);
        Oid::from_hex(hex.as_bytes()).unwrap()
    }

    fn commit(id: u8, parents: &[u8]) -> Commit {
        Commit {
            id: oid(id),
            parents: parents.iter().map(|&p| oid(p)).collect(),
            author: "a".into(),
            email: "e".into(),
            time: 0,
            subject: "s".into(),
            has_body: false,
        }
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let mut g = Graph::new();
        for (c, p) in [(3, vec![2]), (2, vec![1]), (1, vec![])] {
            g.push(&commit(c, &p));
        }
        assert!(g.rows.iter().all(|r| r.node == 0));
        assert_eq!(g.max_width, 1);
    }

    #[test]
    fn branch_and_merge_use_two_lanes_and_converge() {
        // 5 = merge(4, 3); 4 -> 2; 3 -> 2; 2 -> 1
        let mut g = Graph::new();
        for (c, p) in [(5, vec![4, 3]), (4, vec![2]), (3, vec![2]), (2, vec![1]), (1, vec![])] {
            g.push(&commit(c, &p));
        }
        let nodes: Vec<u16> = g.rows.iter().map(|r| r.node).collect();
        assert_eq!(nodes, vec![0, 0, 1, 0, 0]);
        assert!(g.rows[0].is_merge);
        // Commit 3 (lane 1) leads into the lane already waiting for 2.
        assert!(g.rows[2].segs.iter().any(|s| !s.top && s.from == 1 && s.to == 0));
        assert_eq!(g.max_width, 2);
        // After the branch converges, lane 1 is free again.
        assert_eq!(g.rows[4].width, 1);
    }
}
