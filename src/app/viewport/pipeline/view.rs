use quark::view;
use quark_app::ViewContext;
use quark_ui::element::*;
use quark_ui::style::Styled;

use super::PipelineController;
use super::state::PipelineStatus;
use crate::app::shell::Slot;
use crate::app::tokens::TextStyle;

pub fn view(pipeline: &mut PipelineController, slot: &Slot, cx: &mut ViewContext) -> AnyElement {
    let theme = cx.theme;
    let label = status_label(pipeline);
    view! {
        <div h={slot.height} class="flex-row items-center" test_id="status.compare">
            <text size={TextStyle::BodySmall.font_size(theme)} color={theme.colors.text_muted}>
                {label}
            </text>
        </div>
    }
}

fn status_label(pipeline: &PipelineController) -> String {
    match pipeline.status() {
        PipelineStatus::Idle => "No comparison".to_owned(),
        PipelineStatus::Comparing(Some(phase)) => phase.label(),
        PipelineStatus::Comparing(None) => "Comparing\u{2026}".to_owned(),
        PipelineStatus::Ready => {
            let files = pipeline
                .manifest()
                .map_or(0, |manifest| manifest.files.len());
            match files {
                0 => "No changes".to_owned(),
                1 => "1 file changed".to_owned(),
                n => format!("{n} files changed"),
            }
        }
        PipelineStatus::Failed(message) => message.clone(),
    }
}
