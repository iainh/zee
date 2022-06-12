use zi::{
    prelude::{KeyCode, KeyEvent, KeyModifiers},
    Bindings, EndsWith, FlexDirection,
};

use super::{Editor, FileSource, Message};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct KeySequenceSlice<'a> {
    keys: &'a [KeyEvent],
    prefix: bool,
}

impl<'a> KeySequenceSlice<'a> {
    pub fn new(keys: &'a [KeyEvent], prefix: bool) -> Self {
        Self { keys, prefix }
    }
}

impl<'a> std::fmt::Display for KeySequenceSlice<'a> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::result::Result<(), std::fmt::Error> {
        for (index, key) in self.keys.iter().enumerate() {
            let modifier_string = if !key.modifiers.is_empty() {
                let mut prefix = vec![];
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    prefix.push("C");
                }
                if key.modifiers.contains(KeyModifiers::ALT) {
                    prefix.push("A");
                }
                let prefix = prefix.join("-");
                Some(prefix.clone())
            } else {
                None
            };

            if let Some(mod_string) = modifier_string {
                write!(formatter, "{}-", mod_string)?
            }

            match key.code {
                KeyCode::Char(' ') => write!(formatter, "SPC")?,
                KeyCode::Enter => write!(formatter, "RET")?,
                KeyCode::Tab => write!(formatter, "TAB")?,
                KeyCode::Up => write!(formatter, "UP")?,
                KeyCode::Down => write!(formatter, "DOWN")?,
                KeyCode::Left => write!(formatter, "LEFT")?,
                KeyCode::Right => write!(formatter, "RIGHT")?,
                KeyCode::PageUp => write!(formatter, "PAGE UP")?,
                KeyCode::PageDown => write!(formatter, "PAGE DOWN")?,
                KeyCode::Char(char) => write!(formatter, "{}", char)?,
                KeyCode::F(number) => write!(formatter, "F{}", number)?,
                KeyCode::Esc => write!(formatter, "ESC")?,
                key => write!(formatter, "{:?}", key)?,
            }
            if index < self.keys.len().saturating_sub(1) {
                write!(formatter, " ")?;
            } else if self.prefix {
                write!(formatter, "-")?;
            }
        }
        Ok(())
    }
}

pub(super) fn initialize(bindings: &mut Bindings<Editor>) {
    bindings.set_focus(true);
    bindings.set_notify(true);

    // Cancel
    bindings.add(
        "cancel",
        EndsWith(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)),
        || Message::Cancel,
    );

    // Open a file
    bindings.add(
        "find-file",
        [
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        ],
        || Message::OpenFilePicker(FileSource::Directory),
    );
    bindings.add(
        "find-file-in-repo",
        [
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
        ],
        || Message::OpenFilePicker(FileSource::Repository),
    );

    // Buffer management
    bindings.add(
        "switch-buffer",
        [
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
        ],
        || Message::SelectBufferPicker,
    );
    bindings.add(
        "kill-buffer",
        [
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
        ],
        || Message::KillBufferPicker,
    );

    // Window management
    //
    // Change focus
    bindings
        .command("focus-next-window", || Message::FocusNextWindow)
        .with([
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::from(KeyCode::Char('o')),
        ])
        .with([
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
        ]);
    bindings
        .command("focus-previous-window", || Message::FocusPreviousWindow)
        .with([
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::from(KeyCode::Char('i')),
        ])
        .with([
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL),
        ]);

    // Make current window fullscreen
    bindings
        .command("fullscreen-window", || Message::FullscreenWindow)
        .with([
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::from(KeyCode::Char('1')),
        ])
        .with([
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::CONTROL),
        ]);

    // Split window below (column)
    bindings
        .command("split-window-below", || {
            Message::SplitWindow(FlexDirection::Column)
        })
        .with([
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::from(KeyCode::Char('2')),
        ])
        .with([
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::CONTROL),
        ]);

    // Split window right (row)
    bindings
        .command("split-window-right", || {
            Message::SplitWindow(FlexDirection::Row)
        })
        .with([
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::from(KeyCode::Char('3')),
        ])
        .with([
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('3'), KeyModifiers::CONTROL),
        ]);

    // Delete window
    bindings
        .command("delete-window", || Message::DeleteWindow)
        .with([
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::from(KeyCode::Char('0')),
        ])
        .with([
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('0'), KeyModifiers::CONTROL),
        ]);

    // Theme
    bindings.add(
        "change-theme",
        [
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL),
        ],
        || Message::ChangeTheme,
    );

    // Quit
    bindings.add(
        "quit",
        [
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ],
        || Message::Quit,
    );
}
