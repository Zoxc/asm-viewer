//! The box the app says something in outside its own window.
//!
//! It is the desktop's own -- a `zenity` child process on Linux, `TaskDialogIndirect` on
//! Windows, `NSAlert` on macOS -- because the caller it exists for is a panic, which may
//! have left no frame to draw a window of the app's in. The price is that such a box will
//! not scroll and its text cannot be selected, which is why the panic path caps what it
//! puts in one (`crate::panics`).

/// The box: the level, the title and the text, left unshown so the caller can add to it.
///
/// It stops short of showing because the buttons are the caller's, and showing is what
/// waits for one to be pressed.
pub(crate) fn message_box(
    level: rfd::MessageLevel,
    title: impl Into<String>,
    text: impl Into<String>,
) -> rfd::MessageDialog {
    rfd::MessageDialog::new()
        .set_level(level)
        .set_title(title)
        .set_description(text)
}
