use std::{
    collections::HashMap,
    hash::{BuildHasher, Hash, RandomState},
};

use crate::hash::BuildIdentityHasher;

const NONE: usize = usize::MAX;

pub struct InternTable<T> {
    arena: Vec<T>,
    boundaries: Vec<usize>,
    next: Vec<usize>,
    table: HashMap<u64, usize, BuildIdentityHasher>,
    hasher: RandomState,
}

impl<T> Default for InternTable<T> {
    fn default() -> Self {
        Self {
            arena: Vec::new(),
            boundaries: Vec::new(),
            next: Vec::new(),
            table: HashMap::default(),
            hasher: RandomState::new(),
        }
    }
}

impl<T> InternTable<T>
where
    T: Clone + Hash + Eq,
{
    pub fn intern(&mut self, item: &[T]) -> usize {
        if let Some(item) = self.get(item) {
            return item;
        }

        let hash = self.hasher.hash_one(item);
        let id = self.boundaries.len();

        self.arena.extend_from_slice(item);
        let end = self.arena.len();

        self.boundaries.push(end);
        self.next.push(self.table.insert(hash, id).unwrap_or(NONE));

        id
    }

    pub fn get(&self, item: &[T]) -> Option<usize> {
        let hash = self.hasher.hash_one(item);

        let mut id = self.table.get(&hash).copied().unwrap_or(NONE);
        while id != NONE {
            let (start, end) = indices(&self.boundaries, id);
            if &self.arena[start..end] == item {
                return Some(id);
            }
            id = self.next[id];
        }

        None
    }

    pub fn resolve(&self, id: usize) -> &[T] {
        let (start, end) = indices(&self.boundaries, id);
        &self.arena[start..end]
    }
}

fn indices(boundaries: &[usize], id: usize) -> (usize, usize) {
    let end = boundaries[id];
    let start = if id > 0 { boundaries[id - 1] } else { 0 };
    (start, end)
}
