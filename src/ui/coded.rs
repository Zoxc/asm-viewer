//! Which lines of the file the Source pane is showing produced code: what the gutter
//! marks, and the question the analysis worker answers.
//!
//! The pane writes the file it draws into one state and an effect here turns that into
//! the question, the way its text and its links are asked for too (`ShowingFile`,
//! `src/ui/source_view.rs`). The third of the three states an answer about that file
//! lands in, beside [`Sourced`] (`src/ui/highlight.rs`) and [`Linked`]
//! (`src/ui/linking.rs`).

use super::*;

/// The gutter's marks, shared through context: the analysis worker writes the lines of
/// the file [`ShowingFile`] names.
#[derive(Clone, Copy)]
pub(crate) struct Coding(pub(crate) State<Coded>);

/// The lines of one source file the open objects have code for: what the gutter marks,
/// and the answer to the [`Question::Marks`] asked for whatever file [`ShowingFile`]
/// names.
///
/// **Every line that produced code, not the drawn symbol's own.** A source-driven tab has
/// no drawn symbol until a line is clicked, so a mark bounded by one would be a gutter
/// that stayed bare until the reader guessed where to click -- which is the thing the
/// mark exists to save them. So it says the file's own fact: this line produced code, in
/// something. Which symbol is the pair's question and the Locations panel's.
///
/// There is no `pending` field, for [`Located`]'s reason: a file is being looked for
/// exactly while it is showing and the answer is not about it.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Coded {
    /// The file the lines below are of, and the lines. An empty set is an answer.
    pub(crate) found: Option<(Arc<Path>, Arc<HashSet<u32>>)>,
    /// The objects the answer was worked out over, by pointer, which is what identity is
    /// here. Held as addresses and not as `Arc`s: a set of line numbers has nothing in it
    /// to sweep for a binary that has since closed, so the way this stays true is to be
    /// asked again when what is open changes -- and a state keeping the objects alive to
    /// notice that would be the state stopping them from closing.
    pub(crate) over: Vec<usize>,
}

/// The objects `open` are, by pointer, in their own order.
pub(crate) fn object_ids(open: &[Arc<Object>]) -> Vec<usize> {
    open.iter()
        .map(|object| Arc::as_ptr(object).addr())
        .collect()
}

impl Coded {
    /// Whether a question is owed for `showing`: the answer is about another file, or was
    /// worked out over other objects than `open`.
    pub(crate) fn pending(&self, showing: &Arc<Path>, open: &[Arc<Object>]) -> bool {
        !matches!(&self.found, Some((file, _)) if file == showing && self.over == object_ids(open))
    }

    /// Take `lines` as the answer about `file`, worked out `over` those objects. Whether
    /// anything changed, so the caller writes only then ([`write_if`]).
    ///
    /// The locate's rule, against `showing`, the file the pane is showing *now*: a reader
    /// who moved on while the index built is not given the file they left. There is no
    /// per-object sweep, the answer being lines and not symbols -- what keeps it true as
    /// binaries come and go is `over` and the effect that reads it.
    pub(crate) fn take(
        &mut self,
        showing: Option<&Arc<Path>>,
        file: Arc<Path>,
        lines: Arc<HashSet<u32>>,
        over: Vec<usize>,
    ) -> bool {
        if showing != Some(&file) {
            return false;
        }
        self.found = Some((file, lines));
        self.over = over;
        true
    }

    /// The lines of `file` that have code, and nothing where the answer is about another
    /// file -- which is what a pane draws in the beat between moving and being answered.
    pub(crate) fn lines_in(&self, file: &Path) -> Option<&Arc<HashSet<u32>>> {
        match &self.found {
            Some((of, lines)) if &**of == file => Some(lines),
            _ => None,
        }
    }
}

/// The gutter marks' question, asked for whichever file the Source pane last said it was
/// showing. The objects are part of the question here and not only of the answer, unlike
/// the locate's: an answer is a set of bare line numbers with nothing in it to sweep for a
/// closed binary, so the way it stays true is to ask again whenever the open objects
/// change -- which a load finishing also is, and which is what puts marks in a gutter that
/// was drawn before its binary had been read. They are in it by their ids, an
/// `Arc<Object>` having no equality for the memo to compare and the ids being what
/// [`Coded`] judges the answer against anyway.
///
/// Called at the root beside [`use_analysis_with`], which starts the worker and hands back
/// `requests`, the way to ask it.
pub(crate) fn use_mark_asks(
    coded: State<Coded>,
    showing: State<Option<Arc<Path>>>,
    objects: State<Vec<Arc<Object>>>,
    requests: Requests<Question>,
) {
    use_asking(
        // All three read and none peeked, and read in the memo, which is what subscribes
        // it to them: the pane moving to another file, an answer landing and a load
        // finishing are what wake this.
        move || {
            let open = objects.read().clone();
            let file = showing.read().clone()?;
            coded
                .read()
                .pending(&file, &open)
                .then(|| (file, object_ids(&open)))
        },
        unmarked,
        move |(file, _)| {
            requests.send(Question::Marks {
                file,
                objects: objects.peek().clone(),
            });
        },
    );
}

#[cfg(test)]
mod tests;
