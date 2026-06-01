use crate::UiNodeId;

/// Event categories the native platform can route without exposing DOM events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiEventKind {
    Click,
    PointerDown,
    PointerUp,
    PointerMove,
    PointerEnter,
    PointerLeave,
    Wheel,
    KeyDown,
    TextInput,
    Focus,
    Blur,
}

/// Capture/target/bubble phases borrowed from the web, expressed as native data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiEventPhase {
    Capture,
    Target,
    Bubble,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiEventPropagation {
    Continue,
    Stop,
}

/// Handler result: explicit propagation/default intent, no implicit exceptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UiEventResult {
    pub propagation: UiEventPropagation,
    pub default_prevented: bool,
}

impl UiEventResult {
    pub const fn continue_propagation() -> Self {
        Self {
            propagation: UiEventPropagation::Continue,
            default_prevented: false,
        }
    }

    pub const fn stop_propagation() -> Self {
        Self {
            propagation: UiEventPropagation::Stop,
            default_prevented: false,
        }
    }

    pub const fn prevent_default() -> Self {
        Self {
            propagation: UiEventPropagation::Continue,
            default_prevented: true,
        }
    }

    pub const fn stop_and_prevent_default() -> Self {
        Self {
            propagation: UiEventPropagation::Stop,
            default_prevented: true,
        }
    }

    pub const fn should_continue(self) -> bool {
        matches!(self.propagation, UiEventPropagation::Continue)
    }
}

impl Default for UiEventResult {
    fn default() -> Self {
        Self::continue_propagation()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiEventBinding {
    pub kind: UiEventKind,
    pub phase: UiEventPhase,
    pub default_result: UiEventResult,
}

impl UiEventBinding {
    pub fn new(kind: UiEventKind, phase: UiEventPhase) -> Self {
        Self {
            kind,
            phase,
            default_result: UiEventResult::continue_propagation(),
        }
    }

    pub fn with_result(mut self, result: UiEventResult) -> Self {
        self.default_result = result;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutedEventStep {
    pub node: UiNodeId,
    pub phase: UiEventPhase,
}

/// A target plus ancestor chain, enough to test and inspect native propagation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiEventRoute {
    ancestors: Vec<UiNodeId>,
    target: UiNodeId,
}

impl UiEventRoute {
    pub fn new(target: impl Into<UiNodeId>, ancestors: impl IntoIterator<Item = UiNodeId>) -> Self {
        Self {
            ancestors: ancestors.into_iter().collect(),
            target: target.into(),
        }
    }

    pub fn ordered_steps(&self) -> Vec<RoutedEventStep> {
        let mut steps = Vec::with_capacity(self.ancestors.len() * 2 + 1);
        for node in &self.ancestors {
            steps.push(RoutedEventStep {
                node: node.clone(),
                phase: UiEventPhase::Capture,
            });
        }
        steps.push(RoutedEventStep {
            node: self.target.clone(),
            phase: UiEventPhase::Target,
        });
        for node in self.ancestors.iter().rev() {
            steps.push(RoutedEventStep {
                node: node.clone(),
                phase: UiEventPhase::Bubble,
            });
        }
        steps
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PointerCapture {
    pub owner: UiNodeId,
    pub pointer_id: u64,
}

impl PointerCapture {
    pub fn new(owner: impl Into<UiNodeId>, pointer_id: u64) -> Self {
        Self {
            owner: owner.into(),
            pointer_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DragSession {
    pub owner: UiNodeId,
    pub pointer_id: u64,
    pub start: (f32, f32),
    pub current: (f32, f32),
}

impl DragSession {
    pub fn new(owner: impl Into<UiNodeId>, pointer_id: u64, start: (f32, f32)) -> Self {
        Self {
            owner: owner.into(),
            pointer_id,
            start,
            current: start,
        }
    }

    pub fn update(&mut self, current: (f32, f32)) {
        self.current = current;
    }

    pub fn delta(&self) -> (f32, f32) {
        (self.current.0 - self.start.0, self.current.1 - self.start.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_orders_capture_target_bubble() {
        let route = UiEventRoute::new("button", [UiNodeId::from("root"), UiNodeId::from("dialog")]);
        let got: Vec<_> = route
            .ordered_steps()
            .into_iter()
            .map(|step| (step.node.to_string(), step.phase))
            .collect();

        assert_eq!(
            got,
            vec![
                ("root".to_owned(), UiEventPhase::Capture),
                ("dialog".to_owned(), UiEventPhase::Capture),
                ("button".to_owned(), UiEventPhase::Target),
                ("dialog".to_owned(), UiEventPhase::Bubble),
                ("root".to_owned(), UiEventPhase::Bubble),
            ]
        );
    }

    #[test]
    fn event_result_is_explicit_about_stop_and_default() {
        let result = UiEventResult::stop_and_prevent_default();

        assert!(!result.should_continue());
        assert!(result.default_prevented);
    }

    #[test]
    fn drag_session_tracks_delta_under_pointer_capture() {
        let capture = PointerCapture::new("splitter", 1);
        let mut drag = DragSession::new(capture.owner, capture.pointer_id, (10.0, 20.0));
        drag.update((14.5, 18.0));

        assert_eq!(drag.delta(), (4.5, -2.0));
    }
}
