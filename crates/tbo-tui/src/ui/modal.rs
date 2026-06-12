//! Blocking modal overlays: yes/no confirmations and single-line text prompts.
//!
//! While a [`Modal`] is active the app routes every key press to it (see
//! [`Modal::on_key`]) so the underlying screen cannot be navigated until the
//! operator confirms or cancels. Confirming yields the modal's [`Intent`],
//! which the app turns into an operator action.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The operator action a modal performs when confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Permanently delete the selected message.
    DeleteMessage,
    /// Submit the prompt's text as a translation for the selected message.
    TranslateMessage,
    /// Archive (retire) the selected question.
    ArchiveQuestion,
    /// Collect the prompt text for a new question (first create step).
    NewQuestionPrompt,
    /// Collect the FLAC file path for a new question and create it, carrying
    /// the prompt text entered in the first step.
    NewQuestionAudio {
        /// The prompt text gathered by the preceding [`Intent::NewQuestionPrompt`] step.
        prompt: String,
    },
}

/// What the app should do after routing a key to the active modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalDecision {
    /// Keep the modal open and await more input.
    Stay,
    /// Close the modal without acting.
    Cancel,
    /// Close the modal and perform its [`Intent`].
    Confirm,
}

/// A modal overlay that captures key input until dismissed.
#[derive(Debug, Clone)]
pub enum Modal {
    /// A yes/no confirmation of a (typically destructive) action.
    Confirm(ConfirmModal),
    /// A single-line text prompt whose contents feed the action.
    Prompt(PromptModal),
}

/// A yes/no confirmation modal.
#[derive(Debug, Clone)]
pub struct ConfirmModal {
    /// Title shown in the modal's border.
    pub title: String,
    /// Body text describing what will happen on confirmation.
    pub body: String,
    /// The action to perform when confirmed.
    pub intent: Intent,
}

/// A single-line text-entry modal.
#[derive(Debug, Clone)]
pub struct PromptModal {
    /// Title shown in the modal's border.
    pub title: String,
    /// Label shown above the input field.
    pub label: String,
    /// The action to perform with the entered text when submitted.
    pub intent: Intent,
    input: String,
}

impl Modal {
    /// Build a confirmation modal.
    #[must_use]
    pub fn confirm(title: impl Into<String>, body: impl Into<String>, intent: Intent) -> Self {
        Self::Confirm(ConfirmModal {
            title: title.into(),
            body: body.into(),
            intent,
        })
    }

    /// Build an empty single-line prompt modal.
    #[must_use]
    pub fn prompt(title: impl Into<String>, label: impl Into<String>, intent: Intent) -> Self {
        Self::Prompt(PromptModal {
            title: title.into(),
            label: label.into(),
            intent,
            input: String::new(),
        })
    }

    /// The action this modal performs when confirmed.
    #[must_use]
    pub fn intent(&self) -> Intent {
        match self {
            Self::Confirm(modal) => modal.intent.clone(),
            Self::Prompt(modal) => modal.intent.clone(),
        }
    }

    /// The text entered into a prompt (empty for a confirmation).
    #[must_use]
    pub fn input(&self) -> &str {
        match self {
            Self::Confirm(_) => "",
            Self::Prompt(modal) => &modal.input,
        }
    }

    /// Route a key press to the modal and report what the app should do.
    pub fn on_key(&mut self, key: KeyEvent) -> ModalDecision {
        match self {
            Self::Confirm(_) => Self::confirm_key(key),
            Self::Prompt(modal) => modal.on_key(key),
        }
    }

    /// Key handling for a confirmation modal.
    fn confirm_key(key: KeyEvent) -> ModalDecision {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y' | 'Y') => ModalDecision::Confirm,
            KeyCode::Esc | KeyCode::Char('n' | 'N') => ModalDecision::Cancel,
            _ => ModalDecision::Stay,
        }
    }
}

impl PromptModal {
    /// Key handling for a text prompt: edit the buffer, submit, or cancel.
    fn on_key(&mut self, key: KeyEvent) -> ModalDecision {
        match key.code {
            KeyCode::Esc => ModalDecision::Cancel,
            KeyCode::Enter => {
                if self.input.trim().is_empty() {
                    ModalDecision::Stay
                } else {
                    ModalDecision::Confirm
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
                ModalDecision::Stay
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.input.push(c);
                ModalDecision::Stay
            }
            _ => ModalDecision::Stay,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn confirm_modal_confirms_on_y_and_enter() {
        let mut modal = Modal::confirm("Delete?", "Are you sure?", Intent::DeleteMessage);
        assert_eq!(
            modal.on_key(key(KeyCode::Char('y'))),
            ModalDecision::Confirm
        );
        assert_eq!(modal.on_key(key(KeyCode::Enter)), ModalDecision::Confirm);
        assert_eq!(modal.intent(), Intent::DeleteMessage);
    }

    #[test]
    fn confirm_modal_cancels_on_n_and_esc() {
        let mut modal = Modal::confirm("Delete?", "Are you sure?", Intent::DeleteMessage);
        assert_eq!(modal.on_key(key(KeyCode::Char('n'))), ModalDecision::Cancel);
        assert_eq!(modal.on_key(key(KeyCode::Esc)), ModalDecision::Cancel);
    }

    #[test]
    fn prompt_modal_edits_and_submits() {
        let mut modal = Modal::prompt("Translate", "Text", Intent::TranslateMessage);
        for c in "hi".chars() {
            assert_eq!(modal.on_key(key(KeyCode::Char(c))), ModalDecision::Stay);
        }
        assert_eq!(modal.input(), "hi");
        modal.on_key(key(KeyCode::Backspace));
        assert_eq!(modal.input(), "h");
        assert_eq!(modal.on_key(key(KeyCode::Enter)), ModalDecision::Confirm);
    }

    #[test]
    fn prompt_modal_does_not_submit_when_empty() {
        let mut modal = Modal::prompt("Translate", "Text", Intent::TranslateMessage);
        assert_eq!(modal.on_key(key(KeyCode::Enter)), ModalDecision::Stay);
        modal.on_key(key(KeyCode::Char(' ')));
        assert_eq!(
            modal.on_key(key(KeyCode::Enter)),
            ModalDecision::Stay,
            "whitespace-only input must not submit"
        );
    }

    #[test]
    fn prompt_modal_ignores_control_chars() {
        let mut modal = Modal::prompt("Translate", "Text", Intent::TranslateMessage);
        let ctrl_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(modal.on_key(ctrl_a), ModalDecision::Stay);
        assert_eq!(modal.input(), "");
    }
}
