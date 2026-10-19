use std::cell::RefCell;
use std::fmt::Debug;

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

const FALLBACK_TARGET: &str = "diffy::difftastic";
pub const PHASE_PREFIX: &str = "difftastic timing ";
pub const RECORD_PREFIX: &str = "semantic-eval-record ";

thread_local! {
    static FALLBACK: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn install() {
    tracing_subscriber::registry().with(FallbackLayer).init();
}

pub fn take_fallback() -> Option<String> {
    FALLBACK.with(|slot| slot.borrow_mut().take())
}

pub fn parse_phase(line: &str) -> Option<(String, f64)> {
    let rest = line.strip_prefix(PHASE_PREFIX)?;
    let mut phase = None;
    let mut elapsed = None;
    for token in rest.split_whitespace() {
        if let Some(value) = token.strip_prefix("phase=") {
            phase = Some(value.to_owned());
        } else if let Some(value) = token.strip_prefix("elapsed_ms=") {
            elapsed = value.parse().ok();
        }
    }
    Some((phase?, elapsed?))
}

struct FallbackLayer;

impl<S: Subscriber> Layer<S> for FallbackLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if event.metadata().target() != FALLBACK_TARGET {
            return;
        }
        let mut reason = Reason(None);
        event.record(&mut reason);
        if let Some(reason) = reason.0 {
            FALLBACK.with(|slot| *slot.borrow_mut() = Some(reason));
        }
    }
}

struct Reason(Option<String>);

impl Visit for Reason {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "reason" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "reason" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}
