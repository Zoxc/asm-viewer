use super::*;

/// The whole point of the guard: an `assert!` part way down a test body leaves the lines
/// after it unrun, so a directory removed at the foot of one is a directory left behind
/// whenever a test fails. Unwinding runs `Drop`, and that is what is asserted here.
#[test]
fn a_directory_goes_when_the_body_holding_it_panics() {
    let path = std::sync::Mutex::new(PathBuf::new());

    let panicked = std::panic::catch_unwind(|| {
        let directory = Temporary::fresh_directory("guard-test");
        *path.lock().unwrap() = directory.to_path_buf();
        fs::write(directory.join("written"), b"something").expect("a file");
        assert!(directory.join("written").exists());
        panic!("what a failing test does");
    });

    assert!(panicked.is_err());
    let path = path.into_inner().unwrap_or_else(|error| error.into_inner());
    assert!(!path.as_os_str().is_empty());
    assert!(!path.exists(), "the directory outlived the panic");
}

/// What [`Temporary::fresh_under`] owns is the whole of it, so a test whose root has to be
/// called something in particular leaves no parent behind either.
#[test]
fn a_directory_under_another_takes_that_one_with_it() {
    let inner = {
        let directory = Temporary::fresh_under("guard-outer", "root");
        assert!(directory.ends_with("root"));
        assert!(directory.exists());
        directory.to_path_buf()
    };

    assert!(!inner.exists());
    assert!(!inner.parent().expect("an outer directory").exists());
}

/// Two calls under one name are two paths, which is what lets tests asking for the same
/// name run at once.
#[test]
fn every_call_is_given_a_path_of_its_own() {
    let (one, two) = (Temporary::fresh("same"), Temporary::fresh("same"));
    assert_ne!(*one, *two);

    let (three, four) = (
        Temporary::fresh_directory("same"),
        Temporary::fresh_under("same", "root"),
    );
    assert_ne!(*three, *one);
    assert_ne!(four.parent(), Some(&*three));
    assert!(one
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("assembly-viewer-same-")));
}
