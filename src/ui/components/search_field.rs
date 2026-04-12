use halogen::view;

use crate::actions::Action;
use crate::ui::components::{Button, ButtonSize};
use crate::ui::design::{Ico, Sp};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

pub fn search_field(
    input: impl IntoAnyElement,
    has_value: bool,
    on_clear: Option<Action>,
    shortcut_hint: Option<&str>,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let m = &theme.metrics;

    let trailing = if has_value {
        on_clear.map(|action| {
            view! {
                <Button action={action}
                        tooltip={"Clear"}
                        size={ButtonSize::Compact}>
                    <Icon>{lucide::X}</Icon>
                </Button>
            }
        })
    } else {
        shortcut_hint.map(|hint| super::kbd(hint, theme).into_any())
    };

    view! {
        <div class="w-full flex-row items-center"
             gap={m.spacing_sm}
             px={m.spacing_sm + Sp::XXS}
             py={m.spacing_xs}
             rounded={m.control_radius}
             border={tc.border_variant}>
            <icon svg={lucide::SEARCH} size={Ico::XS} color={tc.text_muted} />
            <div class="flex-1" min_w={0.0}>
                {input}
            </div>
            {?trailing}
        </div>
    }
}

pub fn filter_bar(theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let m = &theme.metrics;

    view! {
        <div class="flex-row items-center"
             gap={m.spacing_sm}
             px={m.spacing_sm}
             py={m.spacing_xs}
             border_b={tc.border_variant} />
    }
}
