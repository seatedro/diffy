pub mod interaction;
pub mod pipeline;

pub use interaction::InteractionServices;
pub use pipeline::PipelineServices;

#[derive(Debug, Clone, Default)]
pub struct ViewportServices {
    pub pipeline: PipelineServices,
    pub interaction: InteractionServices,
}
