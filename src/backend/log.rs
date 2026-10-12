//! The step log that every step of a run writes to.
//!
//! Each step records what it did as it does it, so the log holds everything a run established, including the steps
//! before a failure. The interface renders this log; nothing here depends on how it is shown.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Resolve,
    Download,
    Checksum,
    Signature,
    Decompress,
    Configure,
}

impl Step {
    pub fn label(self) -> &'static str {
        match self {
            Step::Resolve => "resolve",
            Step::Download => "download",
            Step::Checksum => "checksum",
            Step::Signature => "signature",
            Step::Decompress => "decompress",
            Step::Configure => "configure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub step: Step,
    pub message: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct StepLog {
    entries: Vec<Entry>,
}

impl StepLog {
    pub fn record(&mut self, step: Step, message: impl Into<String>) {
        self.entries.push(Entry {
            step,
            message: message.into(),
        });
    }

    #[cfg(test)]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
}

impl fmt::Display for StepLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for entry in &self.entries {
            writeln!(f, "[{}] {}", entry.step.label(), entry.message)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_keep_the_order_they_were_recorded_in() {
        let mut log = StepLog::default();

        log.record(Step::Resolve, "chose a device");
        log.record(Step::Download, "fetched an image");

        assert_eq!(
            log.entries(),
            [
                Entry {
                    step: Step::Resolve,
                    message: "chose a device".to_owned(),
                },
                Entry {
                    step: Step::Download,
                    message: "fetched an image".to_owned(),
                },
            ]
        );
        assert_eq!(
            log.to_string(),
            "[resolve] chose a device\n[download] fetched an image\n"
        );
    }
}
