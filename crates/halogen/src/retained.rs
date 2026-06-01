use std::any::Any;
use std::collections::{HashMap, HashSet};

use crate::{UiKey, UiNodeId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedNode {
    pub id: UiNodeId,
    pub key: Option<UiKey>,
    pub parent: Option<UiNodeId>,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisposedNode {
    pub id: UiNodeId,
    pub key: Option<UiKey>,
}

/// Small retained-state store keyed by stable native UI identity.
#[derive(Default)]
pub struct RetainedTree {
    generation: u64,
    nodes: HashMap<UiNodeId, RetainedNode>,
    state: HashMap<UiNodeId, Box<dyn Any>>,
    seen: HashSet<UiNodeId>,
}

impl RetainedTree {
    pub fn begin_frame(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.seen.clear();
    }

    pub fn retain_node(
        &mut self,
        id: impl Into<UiNodeId>,
        key: Option<UiKey>,
        parent: Option<UiNodeId>,
    ) -> &RetainedNode {
        let id = id.into();
        self.seen.insert(id.clone());
        let generation = self.generation;
        let node = self
            .nodes
            .entry(id.clone())
            .or_insert_with(|| RetainedNode {
                id,
                key: key.clone(),
                parent: parent.clone(),
                generation,
            });
        node.key = key;
        node.parent = parent;
        node.generation = generation;
        node
    }

    pub fn local_state_or_insert_with<T: 'static>(
        &mut self,
        id: &UiNodeId,
        init: impl FnOnce() -> T,
    ) -> &mut T {
        self.state
            .entry(id.clone())
            .or_insert_with(|| Box::new(init()))
            .downcast_mut::<T>()
            .expect("retained local state type changed for node id")
    }

    pub fn end_frame(&mut self) -> Vec<DisposedNode> {
        let mut disposed = Vec::new();
        let stale: Vec<UiNodeId> = self
            .nodes
            .keys()
            .filter(|id| !self.seen.contains(*id))
            .cloned()
            .collect();
        for id in stale {
            if let Some(node) = self.nodes.remove(&id) {
                self.state.remove(&id);
                disposed.push(DisposedNode {
                    id: node.id,
                    key: node.key,
                });
            }
        }
        disposed
    }

    pub fn contains(&self, id: &UiNodeId) -> bool {
        self.nodes.contains_key(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_local_state_and_disposes_unseen_nodes() {
        let mut tree = RetainedTree::default();
        let id = UiNodeId::from("button.save");

        tree.begin_frame();
        tree.retain_node(id.clone(), Some(UiKey::from("save")), None);
        *tree.local_state_or_insert_with(&id, || 0usize) += 1;
        assert!(tree.end_frame().is_empty());

        tree.begin_frame();
        tree.retain_node(id.clone(), Some(UiKey::from("save")), None);
        *tree.local_state_or_insert_with(&id, || 0usize) += 1;
        assert_eq!(*tree.local_state_or_insert_with(&id, || 0usize), 2);
        assert!(tree.end_frame().is_empty());

        tree.begin_frame();
        let disposed = tree.end_frame();
        assert_eq!(disposed.len(), 1);
        assert_eq!(disposed[0].id, id);
        assert_eq!(disposed[0].key.as_ref().map(UiKey::as_str), Some("save"));
        assert!(!tree.contains(&disposed[0].id));
    }
}
