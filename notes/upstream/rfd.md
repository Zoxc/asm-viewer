# rfd 0.17

The file and message dialogs, which are the desktop's own and not drawn by the app.

## Wanted

**A message box that will scroll, and whose text can be selected.** `MessageDialog` takes
one `set_description` string and the backends hand it straight on as a label: on Linux
under the default features that is `zenity --question --text`, a child process (see below);
on Windows `TaskDialogIndirect`; on macOS `NSAlert`. None of them scrolls, none caps the
height, and in none of them is the text selectable -- so a long description grows the window
until its buttons are off the screen, and what is on the screen cannot be copied out.

**Cost:** what the app has to show after a panic is a message and a backtrace, and both are
written by whoever panicked. The record for one real crash here is 14,364 bytes: a 51-line
message (freya's hook-order error, of which the first two lines say what happened) over a
59-frame capture. `src/panics.rs` cuts a box out of that -- the message capped at
`MAX_MESSAGE_LINES`, the panic runtime taken off the top of the capture, the frames capped
at `MAX_FRAMES`, each name cut to `MAX_WIDTH`, and the registry and toolchain prefixes taken
off every path. 14,364 bytes to 1,353, and it is a *choice about what to drop* rather than a
smaller window over everything. What no measurement here can settle is the box's *rendered*
height: the description is one label in a proportional font, so a frame at `MAX_WIDTH` may
wrap to two lines on screen. That number is the knob if it does. Because the text cannot be copied either, the whole record
in the run's panic file is not a convenience but the only way to get at it, and the box's
other button opens that file's folder (`src/reveal.rs`), on the hook's own thread so that
the shutdown behind it cannot kill the call before it has made it.

A `set_scrollable(bool)`, or any bound on the height, would let the box hold the record and
make the trimming a nicety rather than the only thing standing between the reader and a
window with no buttons. Selectable text would make the file a convenience again.

**Zenity, or nothing at all.** The default features are `xdg-portal` and `wayland`, and both
send a *message* dialog to `backend/linux/zenity.rs`, which is `Command::new("zenity")`. On a
system without zenity the call fails, rfd logs it and answers `Cancel`, and the app's crash
box silently does not appear -- the reader gets a window that vanishes and a file they have
not been told about. The `gtk3` feature draws in process instead, at the cost of linking GTK.
Not worked around here; noted so it is not mistaken for our own bug.

That zenity is a child process is also the reason any of this works: the hook runs on the
panicking thread before the unwind, so when the UI thread is the one that died the app is
inside its own render, inside winit's `run_app` callback, with no frame to draw a window of
its own in. A dialog that needs neither our event loop nor our GPU is the whole requirement
(`agents/UI.md` has why a second freya window cannot serve).
