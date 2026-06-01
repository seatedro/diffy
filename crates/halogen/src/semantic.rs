use crate::{
    FocusNode, FocusScopeId, FocusTree, KeyContext, Rect, StyleState, TabStop, TestId,
    UiEventBinding, UiEventRoute, UiKey, UiNodeId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemanticRole {
    Button,
    Dialog,
    CheckBox,
    Switch,
    RadioButton,
    Tab,
    TreeItem,
    ListItem,
    ListBoxOption,
    MenuItem,
    ComboBox,
    TextInput,
    ScrollArea,
    Group,
    Label,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SemanticActions {
    pub click: bool,
    pub focus: bool,
    pub text_value: bool,
    pub scroll: bool,
    pub drag: bool,
    pub tooltip: bool,
    pub hit_test: bool,
}

impl SemanticActions {
    pub fn clickable(mut self) -> Self {
        self.click = true;
        self.hit_test = true;
        self
    }

    pub fn focusable(mut self) -> Self {
        self.focus = true;
        self
    }

    pub fn text_value(mut self) -> Self {
        self.text_value = true;
        self.focus = true;
        self
    }

    pub fn scrollable(mut self) -> Self {
        self.scroll = true;
        self.hit_test = true;
        self
    }

    pub fn draggable(mut self) -> Self {
        self.drag = true;
        self.hit_test = true;
        self
    }

    pub fn tooltip(mut self) -> Self {
        self.tooltip = true;
        self
    }

    pub fn hit_test(mut self) -> Self {
        self.hit_test = true;
        self
    }

    pub fn is_empty(self) -> bool {
        !self.click
            && !self.focus
            && !self.text_value
            && !self.scroll
            && !self.drag
            && !self.tooltip
            && !self.hit_test
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SemanticNodeState {
    pub disabled: bool,
    pub selected: Option<bool>,
    pub toggled: Option<bool>,
    pub expanded: Option<bool>,
    pub style_state: StyleState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticNode {
    pub id: Option<UiNodeId>,
    pub key: Option<UiKey>,
    pub test_id: Option<TestId>,
    pub parent: Option<usize>,
    pub role: Option<SemanticRole>,
    pub label: Option<String>,
    pub value: Option<String>,
    pub description: Option<String>,
    pub tooltip: Option<String>,
    pub bounds: Rect,
    pub actions: SemanticActions,
    pub state: SemanticNodeState,
    pub focus_scope: Option<FocusScopeId>,
    pub tab_stop: Option<TabStop>,
    pub key_context: Option<KeyContext>,
    pub event_bindings: Vec<UiEventBinding>,
}

impl SemanticNode {
    pub fn new(bounds: Rect) -> Self {
        Self {
            id: None,
            key: None,
            test_id: None,
            parent: None,
            role: None,
            label: None,
            value: None,
            description: None,
            tooltip: None,
            bounds,
            actions: SemanticActions::default(),
            state: SemanticNodeState::default(),
            focus_scope: None,
            tab_stop: None,
            key_context: None,
            event_bindings: Vec::new(),
        }
    }

    pub fn id(mut self, id: impl Into<UiNodeId>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn key(mut self, key: impl Into<UiKey>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn test_id(mut self, test_id: impl Into<TestId>) -> Self {
        self.test_id = Some(test_id.into());
        self
    }

    pub fn role(mut self, role: SemanticRole) -> Self {
        self.role = Some(role);
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        let label = label.into();
        if !label.is_empty() {
            self.label = Some(label);
        }
        self
    }

    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        let description = description.into();
        if !description.is_empty() {
            self.description = Some(description);
        }
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        let tooltip = tooltip.into();
        if !tooltip.is_empty() {
            self.tooltip = Some(tooltip);
            self.actions = self.actions.tooltip();
        }
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SemanticFrame {
    root_bounds: Rect,
    nodes: Vec<SemanticNode>,
}

impl SemanticFrame {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            root_bounds: Rect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
            nodes: Vec::new(),
        }
    }

    pub fn root_bounds(&self) -> Rect {
        self.root_bounds
    }

    pub fn push(&mut self, node: SemanticNode) -> usize {
        let index = self.nodes.len();
        self.nodes.push(node);
        index
    }

    pub fn nodes(&self) -> &[SemanticNode] {
        &self.nodes
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
    }

    pub fn node_path(&self, target: usize) -> Option<Vec<usize>> {
        if target >= self.nodes.len() {
            return None;
        }
        let mut path = Vec::new();
        let mut current = Some(target);
        while let Some(index) = current {
            let node = self.nodes.get(index)?;
            path.push(index);
            current = node.parent;
        }
        path.reverse();
        Some(path)
    }

    pub fn event_route(&self, target: usize) -> Option<UiEventRoute> {
        let path = self.node_path(target)?;
        let target_id = self.stable_node_id(*path.last()?)?;
        let ancestors = path
            .iter()
            .take(path.len().saturating_sub(1))
            .filter_map(|index| self.stable_node_id(*index))
            .collect::<Vec<_>>();
        Some(UiEventRoute::new(target_id, ancestors))
    }

    pub fn focus_tree(&self) -> FocusTree {
        let mut tree = FocusTree::default();
        for (index, node) in self.nodes.iter().enumerate() {
            if !(node.actions.focus || node.actions.text_value || node.tab_stop.is_some()) {
                continue;
            }
            let Some(id) = self.stable_node_id(index) else {
                continue;
            };
            tree.register(FocusNode {
                id,
                scope: node.focus_scope.clone(),
                tab_stop: node.tab_stop.unwrap_or_else(|| TabStop::new(index as i32)),
                key_context: node.key_context.clone(),
            });
        }
        tree
    }

    fn stable_node_id(&self, index: usize) -> Option<UiNodeId> {
        let node = self.nodes.get(index)?;
        node.id
            .clone()
            .or_else(|| node.test_id.as_ref().map(|id| UiNodeId::from(id.as_str())))
    }
}

pub fn dump_semantic(frame: &SemanticFrame) -> String {
    let mut out = String::new();
    for (i, node) in frame.nodes.iter().enumerate() {
        let id = node
            .id
            .as_ref()
            .map(UiNodeId::as_str)
            .or_else(|| node.test_id.as_ref().map(TestId::as_str))
            .unwrap_or("-");
        let role = node
            .role
            .map(|role| format!("{role:?}"))
            .unwrap_or_else(|| "-".to_owned());
        let label = node.label.as_deref().unwrap_or("");
        let test_id = node.test_id.as_ref().map(TestId::as_str).unwrap_or("-");
        out.push_str(&format!(
            "{i} | parent={:?} | {id} | test={test_id} | {role} | {label} | {:.0},{:.0},{:.0},{:.0} | actions={}{}{}{}{}{}{} | state={}\n",
            node.parent,
            node.bounds.x,
            node.bounds.y,
            node.bounds.width,
            node.bounds.height,
            if node.actions.click { " click" } else { "" },
            if node.actions.focus { " focus" } else { "" },
            if node.actions.text_value { " text" } else { "" },
            if node.actions.scroll { " scroll" } else { "" },
            if node.actions.drag { " drag" } else { "" },
            if node.actions.tooltip { " tooltip" } else { "" },
            if node.actions.hit_test { " hit" } else { "" },
            node.state.style_state.bits(),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::{UiEventKind, UiEventPhase};

    use super::*;

    #[test]
    fn semantic_frame_retains_tree_metadata() {
        let mut frame = SemanticFrame::new(400.0, 300.0);
        let parent = frame.push(
            SemanticNode::new(Rect {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 300.0,
            })
            .id("dialog")
            .role(SemanticRole::Dialog)
            .label("Settings"),
        );
        let mut child = SemanticNode::new(Rect {
            x: 20.0,
            y: 20.0,
            width: 100.0,
            height: 32.0,
        })
        .id("save")
        .key("primary")
        .test_id("save-button")
        .role(SemanticRole::Button)
        .label("Save");
        child.parent = Some(parent);
        child.actions = child.actions.clickable().focusable();
        child.event_bindings.push(UiEventBinding::new(
            UiEventKind::Click,
            UiEventPhase::Target,
        ));
        frame.push(child);

        assert_eq!(frame.nodes().len(), 2);
        assert_eq!(frame.nodes()[1].parent, Some(0));
        assert!(frame.nodes()[1].actions.click);
        assert!(dump_semantic(&frame).contains("save-button"));
    }

    #[test]
    fn semantic_frame_derives_event_route_and_focus_tree() {
        let mut frame = SemanticFrame::new(400.0, 300.0);
        let dialog = frame.push(
            SemanticNode::new(Rect {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 300.0,
            })
            .id("dialog")
            .role(SemanticRole::Dialog),
        );
        let mut button = SemanticNode::new(Rect {
            x: 20.0,
            y: 20.0,
            width: 100.0,
            height: 32.0,
        })
        .id("save")
        .role(SemanticRole::Button);
        button.parent = Some(dialog);
        button.actions = SemanticActions::default().clickable().focusable();
        button.focus_scope = Some(FocusScopeId::from("dialog"));
        button.tab_stop = Some(TabStop::new(0));
        let button_index = frame.push(button);

        let route = frame.event_route(button_index).expect("event route");
        let steps: Vec<_> = route
            .ordered_steps()
            .into_iter()
            .map(|step| (step.node.to_string(), step.phase))
            .collect();
        assert_eq!(steps[0].0, "dialog");
        assert_eq!(steps[1].0, "save");

        let scope = FocusScopeId::from("dialog");
        let tree = frame.focus_tree();
        let order: Vec<_> = tree
            .tab_order(Some(&scope))
            .into_iter()
            .map(|node| node.id.as_str().to_owned())
            .collect();
        assert_eq!(order, vec!["save"]);
    }
}
