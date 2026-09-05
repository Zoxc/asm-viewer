use super::*;

/// The whole point of the guard: an `assert!` part way down a test body leaves the lines
/// after it unrun, so a directory removed at the foot of one is a directory left behind
/// whenever a test fails. Unwinding runs `Drop`, and that is what is asserted here.
#[test]
fn a_directory_goes_when_the_body_holding_it_panics() {
    let path = std::env::temp_dir().join(format!("viewer-guard-test-{}", std::process::id()));

    let panicked = std::panic::catch_unwind(|| {
        let directory = Temporary::directory(path.clone());
        fs::write(directory.join("written"), b"something").expect("a file");
        assert!(directory.join("written").exists());
        panic!("what a failing test does");
    });

    assert!(panicked.is_err());
    assert!(!path.exists(), "the directory outlived the panic");
}

/// What [`Temporary::under`] owns is the whole of it, so a test whose root has to be
/// called something in particular leaves no parent behind either.
#[test]
fn a_directory_under_another_takes_that_one_with_it() {
    let outer = std::env::temp_dir().join(format!("viewer-guard-outer-{}", std::process::id()));
    let inner = {
        let directory = Temporary::under(outer.clone(), "root");
        assert!(directory.ends_with("root"));
        assert!(directory.exists());
        directory.to_path_buf()
    };

    assert!(!inner.exists());
    assert!(!outer.exists());
}
