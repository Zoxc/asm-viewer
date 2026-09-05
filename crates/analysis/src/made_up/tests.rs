use super::MadeUp;

/// The three spellings, exactly as they are written into `project.toml` and the session
/// file. A bookmark on one of these resolves by its name alone once the binary has been
/// rebuilt, so a changed spelling loses it silently.
#[test]
fn the_three_names_are_spelled_as_they_are_saved() {
    assert_eq!(MadeUp::EntryPoint.to_string(), "<entry point>");
    assert_eq!(
        MadeUp::Function(0x140001000).to_string(),
        "<function 0x140001000>"
    );
    assert_eq!(
        MadeUp::Fragment(0x140001000).to_string(),
        "<fragment 0x140001000>"
    );
    // An address is written `0x0` and not `0`.
    assert_eq!(MadeUp::Function(0).to_string(), "<function 0x0>");
}
