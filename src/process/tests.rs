use std::io::Cursor;

use super::*;

/// The line cap: a program writing megabytes with no newline in it must still be
/// *delivered*, in pieces, rather than kept in one growing string nobody ever sees.
#[test]
fn a_line_with_no_end_to_it_is_cut_rather_than_kept() {
    let written = "x".repeat(MAX_LINE as usize * 2 + 7);
    let mut lines = Vec::new();
    stream_lines(Cursor::new(written), Stream::Out, |line| lines.push(line));

    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].text.len(), MAX_LINE as usize);
    assert_eq!(lines[1].text.len(), MAX_LINE as usize);
    assert_eq!(lines[2].text.len(), 7);
    assert!(lines.iter().all(|line| line.stream == Stream::Out));
}

/// The cut falls between characters and not between bytes. A program printing a long line
/// of box-drawing glyphs -- a table, a progress bar -- would otherwise show a replacement
/// character at the seam of every 4 KiB, with the character that was there on neither of
/// the two rows.
#[test]
fn a_character_on_the_cut_is_not_split_between_two_rows() {
    // The cut lands one byte into the `é`, so its two bytes are on either side of it.
    let written = format!("{}é{}", "x".repeat(MAX_LINE as usize - 1), "y".repeat(10));
    let mut lines = Vec::new();
    stream_lines(Cursor::new(written.clone()), Stream::Out, |line| {
        lines.push(line)
    });

    assert_eq!(lines.len(), 2);
    // One byte short of the cap: what the character needed is on the next row instead.
    assert_eq!(lines[0].text.len(), MAX_LINE as usize - 1);
    assert_eq!(&*lines[1].text, format!("é{}", "y".repeat(10)));
    // Nothing added and nothing lost: the rows are the line, in pieces.
    assert_eq!(
        lines.iter().map(|line| &*line.text).collect::<String>(),
        written
    );
}

/// What is not a character is still delivered, lossily, as it always was. The carry is for
/// a cut this module made, and `error_len() == None` is what tells that from output that is
/// simply not UTF-8: bytes that are genuinely invalid must not be held back for a
/// continuation nobody is going to write.
#[test]
fn what_is_not_a_character_is_still_delivered() {
    let mut written = b"a\xffb\n".to_vec();
    // The first two bytes of a box-drawing character, and then the program stops.
    written.extend_from_slice(&[0xe2, 0x94]);

    let mut lines = Vec::new();
    stream_lines(Cursor::new(written), Stream::Err, |line| lines.push(line));

    let text: Vec<&str> = lines.iter().map(|line| &*line.text).collect();
    assert_eq!(text, ["a\u{fffd}b", "\u{fffd}"]);
}

/// The ordinary case, including the two things a naive `read_line` gets wrong: a Windows
/// line ending left in the text, and a last line with no terminator being dropped.
#[test]
fn lines_arrive_without_their_terminators() {
    let mut lines = Vec::new();
    stream_lines(
        Cursor::new("first\r\nsecond\n\nlast"),
        Stream::Err,
        |line| lines.push(line),
    );

    let text: Vec<&str> = lines.iter().map(|line| &*line.text).collect();
    assert_eq!(text, ["first", "second", "", "last"]);
    assert!(lines.iter().all(|line| line.stream == Stream::Err));
}

/// The other bound: the *oldest* goes, and the view can say how much of the story it is
/// missing.
#[test]
fn output_keeps_the_newest_and_counts_what_it_dropped() {
    let mut output = RunOutput::default();
    assert_eq!(output.len(), 0);

    for line in 0..MAX_OUTPUT_LINES + 12 {
        output.push(OutputLine {
            stream: Stream::Out,
            text: Arc::from(line.to_string().as_str()),
        });
    }

    assert_eq!(output.len(), MAX_OUTPUT_LINES);
    assert_eq!(output.dropped(), 12);
    // The oldest kept is the twelfth written, and the newest is the last.
    assert_eq!(&*output.line(0).expect("a line").text, "12");
    assert_eq!(
        &*output.line(MAX_OUTPUT_LINES - 1).expect("a line").text,
        (MAX_OUTPUT_LINES + 11).to_string()
    );
    assert_eq!(output.line(MAX_OUTPUT_LINES), None);
}
