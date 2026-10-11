//! The operator's preferences: the orchestrator, harnesses, and sandboxes the host is set up with.
//!
//! The options offered here are fixed. Choosing them fetches nothing and changes nothing about the image; writing the
//! choices into the image is a later version. The interface renders these values and never decides them.

use std::fmt;

/// An option a preference menu offers. `ALL` is the order the menu lists them in.
pub trait Choice: Copy + PartialEq + 'static {
    const ALL: &'static [Self];

    fn name(self) -> &'static str;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orchestrator {
    Herdr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harness {
    Pi,
    OpenCode,
    ClaudeCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sandbox {
    Nono,
}

impl Choice for Orchestrator {
    const ALL: &'static [Self] = &[Self::Herdr];

    fn name(self) -> &'static str {
        match self {
            Self::Herdr => "herdr",
        }
    }
}

impl Choice for Harness {
    const ALL: &'static [Self] = &[Self::Pi, Self::OpenCode, Self::ClaudeCode];

    fn name(self) -> &'static str {
        match self {
            Self::Pi => "pi agent",
            Self::OpenCode => "opencode",
            Self::ClaudeCode => "Claude Code",
        }
    }
}

impl Choice for Sandbox {
    const ALL: &'static [Self] = &[Self::Nono];

    fn name(self) -> &'static str {
        match self {
            Self::Nono => "nono",
        }
    }
}

/// The choices made on the preferences screen. One orchestrator; one or more harnesses and sandboxes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preferences {
    pub orchestrator: Orchestrator,
    pub harnesses: Vec<Harness>,
    pub sandboxes: Vec<Sandbox>,
}

impl fmt::Display for Preferences {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Orchestrator: {}. Harnesses: {}. Sandboxes: {}.",
            self.orchestrator.name(),
            names(&self.harnesses),
            names(&self.sandboxes)
        )
    }
}

fn names<T: Choice>(choices: &[T]) -> String {
    choices
        .iter()
        .map(|choice| choice.name())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_names_every_choice_in_the_order_it_was_made() {
        let preferences = Preferences {
            orchestrator: Orchestrator::Herdr,
            harnesses: vec![Harness::ClaudeCode, Harness::Pi],
            sandboxes: vec![Sandbox::Nono],
        };

        assert_eq!(
            preferences.to_string(),
            "Orchestrator: herdr. Harnesses: Claude Code, pi agent. Sandboxes: nono."
        );
    }
}
