/// Default steering prompt shown in Settings → Clankers. Users can override
/// it; `start_generate_commit_message` falls back to this when the override
/// is empty.
pub const DEFAULT_STEERING_PROMPT: &str = "You are an expert at writing Git commits. Your job is to write a concise commit message following the Conventional Commits standard.

Express the change with just a single line subject. Don't include a body.

Only return the commit message. Do not return anything else.";

/// Upper bound on the diff payload sent to the model.
pub const MAX_DIFF_BYTES: usize = 20_000;

/// Assemble the single user-role message sent to the model: steering prompt,
/// optional subject hint the user has already typed, then the diff.
pub fn build_user_message(
    prompt: &str,
    subject_override: Option<&str>,
    diff_text: &str,
) -> String {
    let subject_section = match subject_override.map(str::trim).filter(|s| !s.is_empty()) {
        Some(subject) => format!("\nHere is the user's subject line:\n{subject}"),
        None => String::new(),
    };

    format!("{prompt}{subject_section}\nHere are the changes in this commit:\n{diff_text}")
}
