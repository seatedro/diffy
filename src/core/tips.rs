use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::keymap::ShortcutCommand;

const DAY_SECS: u64 = 24 * 60 * 60;
pub const STALE_SECS: u64 = 14 * DAY_SECS;
pub const COOLDOWN_SECS: u64 = 7 * DAY_SECS;
pub const USE_RECORD_SECS: u64 = 60 * 60;
pub const HINT_BAR_SESSIONS: u32 = 3;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TipsMemory {
    pub used: BTreeMap<String, u64>,
    pub shown: BTreeMap<String, u64>,
    pub sessions: u32,
    pub hint_bar_hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Hunks,
    Threads,
    Go,
    Jump,
    Palette,
    Areas,
}

impl Family {
    pub fn key(self) -> &'static str {
        match self {
            Self::Hunks => "hunks",
            Self::Threads => "threads",
            Self::Go => "go",
            Self::Jump => "jump",
            Self::Palette => "palette",
            Self::Areas => "areas",
        }
    }
}

pub fn family_of(command: ShortcutCommand) -> Option<Family> {
    use ShortcutCommand as C;
    Some(match command {
        C::NextHunk | C::PreviousHunk | C::NextFile | C::PreviousFile | C::ReviewMarkViewed => {
            Family::Hunks
        }
        C::NextThread
        | C::PreviousThread
        | C::ReviewReply
        | C::ReviewResolve
        | C::ReviewToggleThread => Family::Threads,
        C::GoPrefix => Family::Go,
        C::JumpToElement => Family::Jump,
        C::OpenCommandPalette => Family::Palette,
        C::FocusAreaLeft | C::FocusAreaRight | C::FocusAreaUp | C::FocusAreaDown => Family::Areas,
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Reviewing,
    Workspace,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Tip {
    pub id: &'static str,
    pub family: Family,
    pub context: Option<Context>,
    pub title: &'static str,
    pub keys: &'static [(ShortcutCommand, &'static str)],
    pub leader: bool,
}

pub const TIPS: &[Tip] = &[
    Tip {
        id: "threads",
        family: Family::Threads,
        context: Some(Context::Reviewing),
        title: "Review from the keyboard",
        keys: &[
            (ShortcutCommand::NextThread, "next thread"),
            (ShortcutCommand::ReviewComment, "reply"),
            (ShortcutCommand::ReviewResolve, "resolve"),
            (ShortcutCommand::ReviewMarkViewed, "viewed, next file"),
        ],
        leader: false,
    },
    Tip {
        id: "hunks",
        family: Family::Hunks,
        context: Some(Context::Workspace),
        title: "Move through changes",
        keys: &[
            (ShortcutCommand::NextHunk, "next hunk"),
            (ShortcutCommand::PreviousHunk, "previous hunk"),
            (ShortcutCommand::NextFile, "next file"),
            (ShortcutCommand::ReviewMarkViewed, "viewed, next file"),
        ],
        leader: false,
    },
    Tip {
        id: "jump",
        family: Family::Jump,
        context: None,
        title: "Click anything without the mouse",
        keys: &[(
            ShortcutCommand::JumpToElement,
            "label every control, then type one",
        )],
        leader: false,
    },
    Tip {
        id: "go",
        family: Family::Go,
        context: None,
        title: "Go anywhere",
        keys: &[],
        leader: true,
    },
    Tip {
        id: "areas",
        family: Family::Areas,
        context: None,
        title: "Move between areas",
        keys: &[
            (ShortcutCommand::FocusAreaLeft, "left"),
            (ShortcutCommand::FocusAreaDown, "down"),
            (ShortcutCommand::FocusAreaUp, "up"),
            (ShortcutCommand::FocusAreaRight, "right"),
        ],
        leader: false,
    },
    Tip {
        id: "palette",
        family: Family::Palette,
        context: None,
        title: "Everything is one search away",
        keys: &[(
            ShortcutCommand::OpenCommandPalette,
            "commands, files, repos, PR links",
        )],
        leader: false,
    },
];

fn older_than(at: Option<&u64>, now: u64, window: u64) -> bool {
    at.is_none_or(|at| now.saturating_sub(*at) >= window)
}

pub fn eligible(memory: &TipsMemory, tip: &Tip, now: u64) -> bool {
    older_than(memory.used.get(tip.family.key()), now, STALE_SECS)
        && older_than(memory.shown.get(tip.id), now, COOLDOWN_SECS)
}

pub fn next_tip(memory: &TipsMemory, context: Context, now: u64) -> Option<&'static Tip> {
    TIPS.iter()
        .filter(|tip| tip.context.is_none_or(|wanted| wanted == context))
        .find(|tip| eligible(memory, tip, now))
}

pub fn should_record_use(memory: &TipsMemory, family: Family, now: u64) -> bool {
    older_than(memory.used.get(family.key()), now, USE_RECORD_SECS)
}

pub fn show_hint_bar(memory: &TipsMemory) -> bool {
    !memory.hint_bar_hidden && memory.sessions <= HINT_BAR_SESSIONS
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 100 * DAY_SECS;

    fn memory(used: &[(&str, u64)], shown: &[(&str, u64)]) -> TipsMemory {
        TipsMemory {
            used: used.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
            shown: shown.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
            ..TipsMemory::default()
        }
    }

    #[test]
    fn recent_use_of_any_key_in_a_family_hides_its_tip() {
        let fresh = memory(&[], &[]);
        assert_eq!(
            next_tip(&fresh, Context::Workspace, NOW).map(|t| t.id),
            Some("hunks")
        );

        let used_go = memory(&[("go", NOW - DAY_SECS), ("hunks", NOW)], &[]);
        let ids: Vec<_> = TIPS
            .iter()
            .filter(|tip| eligible(&used_go, tip, NOW))
            .map(|tip| tip.id)
            .collect();
        assert!(!ids.contains(&"go"));
        assert!(!ids.contains(&"hunks"));
        assert_eq!(family_of(ShortcutCommand::GoPrefix), Some(Family::Go));
    }

    #[test]
    fn tips_return_after_the_family_goes_unused_and_the_cooldown_passes() {
        let stale = memory(&[("go", NOW - STALE_SECS)], &[("go", NOW - COOLDOWN_SECS)]);
        assert!(eligible(&stale, &TIPS[3], NOW));
        let just_shown = memory(&[("go", NOW - STALE_SECS)], &[("go", NOW - DAY_SECS)]);
        assert!(!eligible(&just_shown, &TIPS[3], NOW));
    }

    #[test]
    fn reviewing_prefers_review_tips_and_workspace_skips_them() {
        let fresh = memory(&[], &[]);
        assert_eq!(
            next_tip(&fresh, Context::Reviewing, NOW).map(|t| t.id),
            Some("threads")
        );
        let all_used = memory(&[("hunks", NOW)], &[]);
        assert_ne!(
            next_tip(&all_used, Context::Workspace, NOW).map(|t| t.id),
            Some("threads")
        );
    }

    #[test]
    fn uses_are_recorded_at_most_hourly() {
        let recent = memory(&[("jump", NOW - 60)], &[]);
        assert!(!should_record_use(&recent, Family::Jump, NOW));
        assert!(should_record_use(&recent, Family::Go, NOW));
    }
}
