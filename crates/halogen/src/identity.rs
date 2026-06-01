use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(
    /// Stable identity for a retained semantic UI node.
    UiNodeId
);

string_id!(
    /// Stable sibling identity used when children are reordered.
    UiKey
);

string_id!(
    /// Stable identifier intended for harnesses, tests, and debug tooling.
    TestId
);

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn ids_are_stable_hashable_strings() {
        let a = UiNodeId::from("button.primary");
        let b = UiNodeId::new("button.primary");
        let mut set = HashSet::new();
        set.insert(a.clone());

        assert_eq!(a, b);
        assert!(set.contains(&b));
        assert_eq!(a.as_str(), "button.primary");
        assert_eq!(a.to_string(), "button.primary");
    }
}
