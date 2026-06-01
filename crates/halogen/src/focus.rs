use crate::UiNodeId;

/// Stable identity for a focus scope. Scopes can own tab order and modal traps.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FocusScopeId(String);

impl FocusScopeId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for FocusScopeId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for FocusScopeId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl std::fmt::Display for FocusScopeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Named keyboard action context for routing shortcuts by UI surface.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyContext(String);

impl KeyContext {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for KeyContext {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for KeyContext {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl std::fmt::Display for KeyContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TabStop {
    pub order: i32,
    pub enabled: bool,
}

impl TabStop {
    pub fn new(order: i32) -> Self {
        Self {
            order,
            enabled: true,
        }
    }

    pub fn disabled(order: i32) -> Self {
        Self {
            order,
            enabled: false,
        }
    }
}

impl From<i32> for TabStop {
    fn from(order: i32) -> Self {
        Self::new(order)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusNode {
    pub id: UiNodeId,
    pub scope: Option<FocusScopeId>,
    pub tab_stop: TabStop,
    pub key_context: Option<KeyContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusTree {
    nodes: Vec<FocusNode>,
    modal_trap: Option<FocusScopeId>,
}

impl FocusTree {
    pub fn register(&mut self, node: FocusNode) {
        self.nodes.push(node);
    }

    pub fn trap_modal_scope(&mut self, scope: impl Into<FocusScopeId>) {
        self.modal_trap = Some(scope.into());
    }

    pub fn clear_modal_trap(&mut self) {
        self.modal_trap = None;
    }

    pub fn tab_order(&self, scope: Option<&FocusScopeId>) -> Vec<&FocusNode> {
        let effective_scope = self.modal_trap.as_ref().or(scope);
        let mut nodes: Vec<&FocusNode> = self
            .nodes
            .iter()
            .filter(|node| node.tab_stop.enabled)
            .filter(|node| match effective_scope {
                Some(scope) => node.scope.as_ref() == Some(scope),
                None => true,
            })
            .collect();
        nodes.sort_by_key(|node| node.tab_stop.order);
        nodes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_order_is_scoped_sorted_and_trappable() {
        let modal = FocusScopeId::from("modal");
        let sidebar = FocusScopeId::from("sidebar");
        let mut tree = FocusTree::default();
        tree.register(FocusNode {
            id: "later".into(),
            scope: Some(modal.clone()),
            tab_stop: TabStop::new(20),
            key_context: Some(KeyContext::from("dialog")),
        });
        tree.register(FocusNode {
            id: "first".into(),
            scope: Some(modal.clone()),
            tab_stop: TabStop::new(10),
            key_context: None,
        });
        tree.register(FocusNode {
            id: "outside".into(),
            scope: Some(sidebar),
            tab_stop: TabStop::new(0),
            key_context: None,
        });

        let scoped: Vec<_> = tree
            .tab_order(Some(&modal))
            .into_iter()
            .map(|node| node.id.as_str().to_owned())
            .collect();
        assert_eq!(scoped, vec!["first", "later"]);

        tree.trap_modal_scope(modal);
        let trapped: Vec<_> = tree
            .tab_order(None)
            .into_iter()
            .map(|node| node.id.as_str().to_owned())
            .collect();
        assert_eq!(trapped, vec!["first", "later"]);
    }
}
