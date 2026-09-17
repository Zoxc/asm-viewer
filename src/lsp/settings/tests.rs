use super::*;

/// Read a settings file over `/p`, which is the directory `${workspaceFolder}` stands for
/// in every test below.
fn read(text: &str) -> Result<Settings, Unreadable> {
    settings_from(text, Path::new("/p"))
}

/// The two things a server is silent about getting wrong: the prefix has to come off and
/// the dots have to become a tree. What is not the server's is skipped without a word.
#[test]
fn a_name_loses_its_prefix_and_its_dots_become_a_tree() {
    let settings = read(
        r#"{
            "rust-analyzer.cargo.features": ["a"],
            "rust-analyzer.checkOnSave": true,
            "git.detectSubmodulesLimit": 20,
            "files.associations": { "*.rs": "rust" }
        }"#,
    )
    .expect("a file that reads");

    // The project's own `checkOnSave` wins over the app's, and what the project said
    // nothing about is still the app's.
    assert_eq!(
        settings.options(),
        &json!({
            "checkOnSave": true,
            "diagnostics": { "enable": false },
            "cargo": { "features": ["a"] },
        })
    );
    assert_eq!(
        settings.overrides,
        vec![
            ("cargo.features".to_owned(), "[\"a\"]".to_owned()),
            ("checkOnSave".to_owned(), "true".to_owned()),
        ]
    );
}

/// A name given a value and made a table by a longer name is a file saying two things, and
/// which was meant is not this app's to pick -- whichever order they are written in.
#[test]
fn a_name_that_is_both_a_value_and_a_table_is_a_failure() {
    for text in [
        r#"{ "rust-analyzer.cargo": { "noDeps": true }, "rust-analyzer.cargo.features": [] }"#,
        r#"{ "rust-analyzer.cargo.features": [], "rust-analyzer.cargo": { "noDeps": true } }"#,
    ] {
        assert_eq!(read(text), Err(Unreadable::Both("cargo".to_owned())));
    }

    // Two names under one table are not that: they are the table.
    let settings =
        read(r#"{ "rust-analyzer.cargo.features": [], "rust-analyzer.cargo.noDeps": true }"#)
            .expect("a file that reads");
    assert_eq!(
        settings.options()["cargo"],
        json!({ "features": [], "noDeps": true })
    );
}

/// The one variable, wherever it is written: in a string, inside a table, and inside an
/// array. Nothing but strings is touched.
#[test]
fn the_workspace_folder_is_resolved_wherever_it_is_written() {
    let settings = read(
        r#"{
            "rust-analyzer.rustc.source": "${workspaceFolder}/Cargo.toml",
            "rust-analyzer.server.extraEnv": { "RUSTC": "${workspaceFolder}/build/rustc" },
            "rust-analyzer.linkedProjects": ["${workspaceFolder}/library/Cargo.toml", 7]
        }"#,
    )
    .expect("a file that reads");

    assert_eq!(
        settings.options(),
        &json!({
            "checkOnSave": false,
            "diagnostics": { "enable": false },
            "rustc": { "source": "/p/Cargo.toml" },
            "server": { "extraEnv": { "RUSTC": "/p/build/rustc" } },
            "linkedProjects": ["/p/library/Cargo.toml", 7],
        })
    );
}

/// Every other variable is a failure. VS Code leaves one it does not know as it was
/// written; here that is a path reaching the server that silently is not there.
#[test]
fn a_variable_this_cannot_resolve_is_a_failure() {
    for (text, named) in [
        (
            r#"{ "rust-analyzer.cargo.sysrootSrc": "${userHome}/rust" }"#,
            "userHome",
        ),
        (
            r#"{ "rust-analyzer.cargo.extraEnv": { "A": "${env:PATH}" } }"#,
            "env:PATH",
        ),
        (r#"{ "rust-analyzer.x": ["${execPath}"] }"#, "execPath"),
        (
            r#"{ "rust-analyzer.x": "${workspaceFolderBasename}" }"#,
            "workspaceFolderBasename",
        ),
    ] {
        assert_eq!(read(text), Err(Unreadable::Variable(named.to_owned())));
    }

    // A `${` that is never closed is not a variable and is left as it was written.
    let settings = read(r#"{ "rust-analyzer.x": "${workspaceFolder" }"#).expect("a file");
    assert_eq!(settings.options()["x"], json!("${workspaceFolder"));
}

/// A file that is not JSON, and one that is JSON but not an object.
#[test]
fn a_file_that_is_not_an_object_of_json_is_a_failure() {
    let Err(Unreadable::NotJson(_)) = read("{ \"rust-analyzer.x\": }") else {
        panic!("a file that is not JSON was read as some");
    };
    assert_eq!(read("[1, 2]"), Err(Unreadable::NotAnObject));
    assert_eq!(read("\"hello\""), Err(Unreadable::NotAnObject));
}

/// The file is read as **JSONC**, which is what VS Code reads it as and what the files in
/// the wild are written in: comments and a trailing comma are not failures.
#[test]
fn the_comments_and_trailing_commas_an_editor_allows_are_taken() {
    let settings = read(
        r#"{
            // which manifests are this tree's
            "rust-analyzer.linkedProjects": ["Cargo.toml",],
            /* and the compiler
               it is read with */
            "rust-analyzer.server.extraEnv": { "RUSTC": "stage0/rustc", },
        }"#,
    )
    .expect("a file an editor would take");

    assert_eq!(settings.options()["linkedProjects"], json!(["Cargo.toml"]));
    assert_eq!(
        settings.options()["server"]["extraEnv"]["RUSTC"],
        json!("stage0/rustc")
    );
}

/// Nothing inside a string is stripped. A `//` is half of every URL, and a string that
/// ends in an escaped quote or holds a backslash before its closing one must not swallow
/// what comes after it -- a path cut short without a word is the failure this is against.
#[test]
fn nothing_inside_a_string_is_taken_for_a_comment() {
    let settings = read(
        r#"{
            "rust-analyzer.a": "https://example.invalid/x",
            "rust-analyzer.b": "a \" // b",
            "rust-analyzer.c": "c:\\",
            "rust-analyzer.d": "/* not a comment */"
        }"#,
    )
    .expect("a file whose strings hold comment marks");

    assert_eq!(settings.options()["a"], json!("https://example.invalid/x"));
    assert_eq!(settings.options()["b"], json!("a \" // b"));
    assert_eq!(settings.options()["c"], json!("c:\\"));
    assert_eq!(settings.options()["d"], json!("/* not a comment */"));
}

/// A comma inside a string is not the comma the trailing-comma rule takes out. One pass
/// does both jobs, so the string tracking holds the comma rule up as well: a string whose
/// closing quote was missed leaves the text under it outside every string, and a comma
/// there before a bracket is blanked, taking a real value with it.
#[test]
fn a_comma_inside_a_string_is_not_a_trailing_comma() {
    let settings = read(
        r#"{
            "rust-analyzer.a": "a \" b,",
            "rust-analyzer.b": "c:\\",
            "rust-analyzer.c": "// x,",
            "rust-analyzer.linkedProjects": ["a", "b"],
        }"#,
    )
    .expect("a file whose strings hold commas");

    assert_eq!(settings.options()["a"], json!("a \" b,"));
    assert_eq!(settings.options()["b"], json!("c:\\"));
    assert_eq!(settings.options()["c"], json!("// x,"));
    assert_eq!(settings.options()["linkedProjects"], json!(["a", "b"]));
}

/// The shape of the file the user's own tree keeps: a block of `//` lines and then the
/// object, with the editor's own keys in it beside the server's.
#[test]
fn a_header_of_comments_over_the_object_is_read() {
    let settings = read(
        r#"// This config uses a separate build directory for rust-analyzer,
// so that r-a's checks don't block user `x` commands and vice-verse.
//
// ```
// x fmt --check
// ```
{
    "git.detectSubmodulesLimit": 20,
    "rust-analyzer.linkedProjects": ["Cargo.toml"]
}
"#,
    )
    .expect("the shape of a real one");

    assert_eq!(settings.overrides.len(), 1);
    assert_eq!(settings.options()["linkedProjects"], json!(["Cargo.toml"]));
}

/// What a comment is blanked with keeps the lines under it where they were, so what
/// `serde_json` says about a real mistake is about the file the reader wrote.
#[test]
fn a_comment_leaves_the_lines_under_it_where_they_were() {
    let text = "/* one\n   two */\n{\n  \"a\" \"b\"\n}";
    let Err(Unreadable::NotJson(said)) = read(text) else {
        panic!("a file that is not JSON was read as some");
    };

    assert!(said.contains("line 4"), "{said}");
}

/// The merge is per leaf and not per name: what the project says about `cargo.features`
/// leaves what this app said about the rest of `cargo` where it was.
#[test]
fn a_projects_settings_are_laid_over_this_apps_leaf_by_leaf() {
    let over = json!({ "cargo": { "features": ["a"] } });
    let base = json!({ "checkOnSave": false, "cargo": { "noDeps": true, "features": [] } });

    assert_eq!(
        merged(base, over),
        json!({
            "checkOnSave": false,
            "cargo": { "noDeps": true, "features": ["a"] },
        })
    );

    // And what this app asks of every server is under everything a project did not say.
    let settings = read(r#"{ "rust-analyzer.cargo.features": ["a"] }"#).expect("a file");
    assert_eq!(settings.options()["checkOnSave"], json!(false));
}

/// A name spelled in more parts than the tree is walked with is refused rather than
/// recursed over: how deep that goes is not a thing a file gets to say.
#[test]
fn a_name_of_too_many_parts_is_refused() {
    let name = "a.".repeat(DEEPEST + 1);
    let text = format!(r#"{{ "rust-analyzer.{name}b": 1 }}"#);
    let Err(Unreadable::Deep(_)) = read(&text) else {
        panic!("a name of {} parts was taken", DEEPEST + 2);
    };
}

/// `rust-lang/rust`'s own file, which is the tree the whole thing is for: a server told
/// none of this cannot read it.
#[test]
fn the_settings_a_tree_cannot_be_read_without() {
    let settings = read(
        r#"{
            "rust-analyzer.linkedProjects": [
                "Cargo.toml",
                "src/tools/x/Cargo.toml",
                "src/bootstrap/Cargo.toml"
            ],
            "rust-analyzer.rustc.source": "./Cargo.toml",
            "rust-analyzer.cargo.sysrootSrc": "./library",
            "rust-analyzer.cargo.extraEnv": { "RUSTC_BOOTSTRAP": "1" },
            "rust-analyzer.server.extraEnv": {
                "RUSTC": "${workspaceFolder}/build/host/stage0/bin/rustc",
                "CARGO": "${workspaceFolder}/build/host/stage0/bin/cargo"
            },
            "rust-analyzer.procMacro.server": "${workspaceFolder}/build/host/stage0/libexec/rust-analyzer-proc-macro-srv",
            "rust-analyzer.cargo.buildScripts.overrideCommand": ["python3", "x.py", "check"],
            "rust-analyzer.cargo.buildScripts.invocationStrategy": "once",
            "rust-analyzer.check.invocationStrategy": "once",
            "rust-analyzer.rustfmt.overrideCommand": ["./build/host/rustfmt/bin/rustfmt"],
            "git.detectSubmodulesLimit": 20
        }"#,
    )
    .expect("the tree's own settings");

    let options = settings.options();
    assert_eq!(options["cargo"]["sysrootSrc"], json!("./library"));
    assert_eq!(options["cargo"]["extraEnv"]["RUSTC_BOOTSTRAP"], json!("1"));
    assert_eq!(
        options["cargo"]["buildScripts"]["invocationStrategy"],
        json!("once")
    );
    assert_eq!(
        options["server"]["extraEnv"]["CARGO"],
        json!("/p/build/host/stage0/bin/cargo")
    );
    assert_eq!(
        options["procMacro"]["server"],
        json!("/p/build/host/stage0/libexec/rust-analyzer-proc-macro-srv")
    );
    // The app's own is still under it, and the editor's own key is not there at all.
    assert_eq!(options["checkOnSave"], json!(false));
    assert_eq!(options.get("git"), None);
    // Every setting the server was given is one the reader can be shown.
    assert_eq!(settings.overrides.len(), 10);
}

/// No file at all is no overrides: it is what nearly every project has, and a viewer that
/// called it a failure would call every project one.
#[test]
fn a_project_with_no_settings_file_says_nothing() {
    let settings = settings_in(Path::new("/no/such/directory")).expect("no file is no failure");

    assert_eq!(settings, Settings::none());
    assert_eq!(settings.options(), &wanted());
    assert!(settings.overrides.is_empty());
}

/// The file is spelled once: what the Project view names the reader is what [`settings_in`]
/// joins onto the project's directory. A `Path` is compared by its components, so the
/// constant's `/` is a separator on either platform.
#[test]
fn the_file_named_to_the_reader_is_the_file_that_is_read() {
    assert_eq!(
        Path::new("/p").join(SETTINGS),
        Path::new("/p/.vscode/settings.json")
    );
}
