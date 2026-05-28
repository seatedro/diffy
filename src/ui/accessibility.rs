use std::collections::HashMap;

use accesskit::{
    Action as AxAction, Node, NodeId, Rect as AxRect, Role, Toggled, Tree, TreeId, TreeUpdate,
};

use crate::actions::Action;
use crate::render::Rect;
use crate::ui::element::ScrollActionBuilder;
use crate::ui::state::FocusTarget;

pub const ROOT_ID: NodeId = NodeId(1);

#[derive(Debug, Clone)]
pub enum AccessibilityAction {
    Click(Action),
    Focus(FocusTarget),
    TextValue(FocusTarget),
    Scroll(ScrollActionBuilder),
}

#[derive(Debug, Clone)]
pub struct AccessibilityNode {
    id: NodeId,
    role: Role,
    bounds: Rect,
    label: Option<String>,
    value: Option<String>,
    description: Option<String>,
    disabled: bool,
    selected: Option<bool>,
    toggled: Option<bool>,
    expanded: Option<bool>,
    action: Option<AccessibilityAction>,
    author_id: String,
}

impl AccessibilityNode {
    pub fn new(key: impl AsRef<str>, role: Role, bounds: Rect) -> Self {
        let key = key.as_ref();
        Self {
            id: stable_node_id(key),
            role,
            bounds,
            label: None,
            value: None,
            description: None,
            disabled: false,
            selected: None,
            toggled: None,
            expanded: None,
            action: None,
            author_id: key.to_owned(),
        }
    }

    pub fn button(key: impl AsRef<str>, label: impl Into<String>, bounds: Rect) -> Self {
        Self::new(key, Role::Button, bounds).label(label)
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        let label = label.into();
        if !label.is_empty() {
            self.label = Some(label);
        }
        self
    }

    pub fn value(mut self, value: impl Into<String>) -> Self {
        let value = value.into();
        self.value = Some(value);
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        let description = description.into();
        if !description.is_empty() {
            self.description = Some(description);
        }
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    pub fn toggled(mut self, toggled: bool) -> Self {
        self.toggled = Some(toggled);
        self
    }

    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    pub fn action(mut self, action: AccessibilityAction) -> Self {
        self.action = Some(action);
        self
    }

    fn to_accesskit_node(&self) -> Node {
        let mut node = Node::new(self.role);
        node.set_bounds(ax_rect(self.bounds));
        node.set_author_id(self.author_id.clone());
        if let Some(label) = &self.label {
            node.set_label(label.clone());
        }
        if let Some(value) = &self.value {
            node.set_value(value.clone());
        }
        if let Some(description) = &self.description {
            node.set_description(description.clone());
        }
        if self.disabled {
            node.set_disabled();
        }
        if let Some(selected) = self.selected {
            node.set_selected(selected);
        }
        if let Some(toggled) = self.toggled {
            node.set_toggled(Toggled::from(toggled));
        }
        if let Some(expanded) = self.expanded {
            node.set_expanded(expanded);
        }
        match &self.action {
            Some(AccessibilityAction::Click(_)) => node.add_action(AxAction::Click),
            Some(AccessibilityAction::Focus(_)) => node.add_action(AxAction::Focus),
            Some(AccessibilityAction::TextValue(_)) => {
                node.add_action(AxAction::Focus);
                node.add_action(AxAction::SetValue);
                node.add_action(AxAction::ReplaceSelectedText);
            }
            Some(AccessibilityAction::Scroll(_)) => {
                node.add_action(AxAction::ScrollUp);
                node.add_action(AxAction::ScrollDown);
            }
            None => {}
        }
        node
    }
}

#[derive(Debug, Clone, Default)]
pub struct AccessibilityFrame {
    nodes: Vec<AccessibilityNode>,
    actions: HashMap<NodeId, AccessibilityAction>,
    focused: Option<NodeId>,
    root_bounds: Rect,
}

impl AccessibilityFrame {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            root_bounds: Rect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
            ..Self::default()
        }
    }

    pub fn push(&mut self, node: AccessibilityNode) {
        if let Some(action) = node.action.clone() {
            self.actions.insert(node.id, action);
        }
        if matches!(
            node.action,
            Some(AccessibilityAction::Focus(_) | AccessibilityAction::TextValue(_))
        ) {
            self.focused.get_or_insert(node.id);
        }
        self.nodes.push(node);
    }

    pub fn action_for(&self, id: NodeId) -> Option<&AccessibilityAction> {
        self.actions.get(&id)
    }

    pub fn tree_update(&self, focus: Option<FocusTarget>) -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_bounds(ax_rect(self.root_bounds));
        root.set_label(crate::platform::startup::app_display_name());
        root.set_children(self.nodes.iter().map(|node| node.id).collect::<Vec<_>>());

        let mut nodes = Vec::with_capacity(self.nodes.len() + 1);
        nodes.push((ROOT_ID, root));

        let mut focused = ROOT_ID;
        for node in &self.nodes {
            if let Some(target) = focus {
                let node_focus = match node.action {
                    Some(AccessibilityAction::Focus(t) | AccessibilityAction::TextValue(t)) => {
                        t == target
                    }
                    _ => false,
                };
                if node_focus {
                    focused = node.id;
                }
            }
            nodes.push((node.id, node.to_accesskit_node()));
        }

        TreeUpdate {
            nodes,
            tree: Some(Tree {
                root: ROOT_ID,
                toolkit_name: Some("Diffy Halogen".to_owned()),
                toolkit_version: Some(crate::APP_VERSION.to_owned()),
            }),
            tree_id: TreeId::ROOT,
            focus: focused,
        }
    }
}

pub fn empty_tree_update() -> TreeUpdate {
    AccessibilityFrame::new(1.0, 1.0).tree_update(None)
}

fn ax_rect(rect: Rect) -> AxRect {
    AxRect::new(
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.x + rect.width),
        f64::from(rect.y + rect.height),
    )
}

fn stable_node_id(key: &str) -> NodeId {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    NodeId(hash.max(2))
}
