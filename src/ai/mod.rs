pub mod cli;
pub mod diff_compress;
pub mod locate;
pub mod prompt;
pub mod stream;

pub use cli::{CliAuth, CliCommand, CliInfo, CliScan, CliTool, RunOutcome};
pub use prompt::{DEFAULT_STEERING_PROMPT, MAX_DIFF_BYTES, build_user_message};
pub use stream::{GenerateRequest, Provider, StreamMessage, run_streaming};
