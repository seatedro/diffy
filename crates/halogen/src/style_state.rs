use crate::UiNodeId;

/// Typed interaction state that can invalidate style without a CSS selector engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct StyleState(u16);

impl StyleState {
    pub const HOVER: Self = Self(1 << 0);
    pub const ACTIVE: Self = Self(1 << 1);
    pub const FOCUS_VISIBLE: Self = Self(1 << 2);
    pub const DISABLED: Self = Self(1 << 3);
    pub const SELECTED: Self = Self(1 << 4);
    pub const CHECKED: Self = Self(1 << 5);
    pub const EXPANDED: Self = Self(1 << 6);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub fn contains(self, state: Self) -> bool {
        self.0 & state.0 == state.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn insert(&mut self, state: Self) {
        self.0 |= state.0;
    }

    pub fn with(mut self, state: Self) -> Self {
        self.insert(state);
        self
    }

    pub fn bits(self) -> u16 {
        self.0
    }
}

impl std::ops::BitOr for StyleState {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for StyleState {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Why style-dependent output became dirty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleInvalidationReason {
    State(StyleState),
    Animation,
    Transition,
    Inherited,
    Theme,
    Visibility,
    ViewStyle,
    TargetedSubElement(String),
}

/// A typed invalidation record for retained element/style caches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleInvalidation {
    pub target: Option<UiNodeId>,
    pub reason: StyleInvalidationReason,
}

impl StyleInvalidation {
    pub fn subtree(reason: StyleInvalidationReason) -> Self {
        Self {
            target: None,
            reason,
        }
    }

    pub fn target(target: impl Into<UiNodeId>, reason: StyleInvalidationReason) -> Self {
        Self {
            target: Some(target.into()),
            reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_state_tracks_multiple_typed_flags() {
        let state = StyleState::HOVER | StyleState::FOCUS_VISIBLE;

        assert!(state.contains(StyleState::HOVER));
        assert!(state.contains(StyleState::FOCUS_VISIBLE));
        assert!(!state.contains(StyleState::DISABLED));
        assert!(!state.is_empty());
    }

    #[test]
    fn invalidation_can_target_a_retained_node() {
        let invalidation = StyleInvalidation::target(
            "row:42",
            StyleInvalidationReason::TargetedSubElement("label".to_owned()),
        );

        assert_eq!(
            invalidation.target.as_ref().map(UiNodeId::as_str),
            Some("row:42")
        );
    }
}
