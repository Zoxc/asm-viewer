use super::*;

/// The workspace a canned stream is read against. Only the prefix matters: an artifact is
/// this workspace's or a dependency's by where its manifest is.
fn workspace() -> PathBuf {
    PathBuf::from("/work/app")
}

/// A directory of this test's own under the system temporary directory, named after the
/// line that asked for it, so a failing test leaves something identifiable behind.
fn directory(line: u32) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "assembly-viewer-cargo-test-{}-{line}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("a directory");
    path
}

/// What a failed build reports, over a canned cargo stream — which is why `outcome` is a
/// function of its own. Nothing here shells out.
#[test]
fn a_failed_build_reports_the_compilers_diagnostics() {
    let stdout = concat!(
        r#"{"reason":"compiler-message","message":{"#,
        r#""level":"error","message":"cannot find value `x` in this scope","#,
        r#""rendered":"error[E0425]: cannot find value `x`\n --> src/main.rs:2:5\n","#,
        r#""spans":[{"file_name":"src/main.rs","line_start":2,"column_start":5,"is_primary":true}]}}"#,
        "\n",
        r#"{"reason":"build-finished","success":false}"#,
        "\n",
    );
    let stderr = "   Compiling sketch v0.1.0\nerror: could not compile `sketch`\n";

    let Run::Rejected {
        diagnostics,
        message,
    } = outcome(stdout, stderr, false, &workspace())
    else {
        panic!("a rejection");
    };

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].level, Level::Error);
    assert_eq!(
        diagnostics[0].message,
        "cannot find value `x` in this scope"
    );
    assert!(diagnostics[0].rendered.contains("E0425"));
    assert_eq!(
        diagnostics[0].span,
        Some(Span {
            file: "src/main.rs".into(),
            line: 2,
            column: 5,
        })
    );
    // cargo's own stderr is kept whole: some failures are said there and nowhere else.
    assert!(message.contains("could not compile"));
}

/// The failure with no diagnostics behind it at all — a dependency that names a crate
/// nothing has heard of. cargo says it on stderr and emits no compiler message.
#[test]
fn a_dependency_that_does_not_resolve_is_cargos_own_words() {
    let stderr = "error: no matching package named `not-a-real-crate` found\n\
                      location searched: crates.io index\n";

    assert_eq!(
        outcome("", stderr, false, &workspace()),
        Run::Rejected {
            diagnostics: Vec::new(),
            message: stderr.trim().to_owned(),
        }
    );
}

#[test]
fn a_successful_build_reports_the_artifact_and_its_warnings() {
    let stdout = concat!(
        r#"{"reason":"build-script-executed","package_id":"libc"}"#,
        "\n",
        r#"{"reason":"compiler-message","message":{"level":"warning","#,
        r#""message":"unused variable: `y`","rendered":"warning: unused variable","#,
        r#""spans":[]}}"#,
        "\n",
        r#"{"reason":"compiler-artifact","manifest_path":"/work/app/Cargo.toml","#,
        r#""target":{"name":"sketch","kind":["bin"]},"#,
        r#""executable":"/work/app/target/debug/sketch","#,
        r#""filenames":["/work/app/target/debug/sketch"]}"#,
        "\n",
        r#"{"reason":"build-finished","success":true}"#,
        "\n",
        "warning: some future cargo says something new here\n",
    );

    let Run::Built {
        artifacts,
        diagnostics,
    } = outcome(stdout, "", true, &workspace())
    else {
        panic!("a build");
    };

    assert_eq!(
        artifacts,
        vec![Artifact {
            path: PathBuf::from("/work/app/target/debug/sketch"),
            target: "sketch".to_owned(),
            kind: "bin".to_owned(),
        }]
    );
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].level, Level::Warning);
    assert_eq!(diagnostics[0].span, None);
}

/// A cargo that succeeded and named nothing is an empty list, not an `unwrap`. What the
/// caller makes of that is the caller's: the scratchpad calls it a failure, since its
/// package has a binary in it; a workspace of nothing but libraries has not failed.
#[test]
fn a_build_that_names_no_artifact_says_so() {
    assert_eq!(
        outcome(
            r#"{"reason":"build-finished","success":true}"#,
            "",
            true,
            &workspace()
        ),
        Run::Built {
            artifacts: Vec::new(),
            diagnostics: Vec::new(),
        }
    );
}

/// The rule that makes the list readable at all. cargo reports an artifact for every crate
/// in the graph — 449 of them for this app's own workspace — and all but the workspace's
/// own are somebody else's code the reader did not ask about.
#[test]
fn a_dependencys_artifact_is_not_this_workspaces() {
    let stdout = concat!(
        r#"{"reason":"compiler-artifact","#,
        r#""manifest_path":"/home/reader/.cargo/registry/src/index.crates.io-1949/serde-1.0/Cargo.toml","#,
        r#""target":{"name":"serde","kind":["lib"]},"executable":null,"#,
        r#""filenames":["/work/app/target/debug/deps/libserde.rlib"]}"#,
        "\n",
        // A directory whose name merely starts with the workspace's is not inside it:
        // the match is by path component and not by text.
        r#"{"reason":"compiler-artifact","manifest_path":"/work/apple/Cargo.toml","#,
        r#""target":{"name":"apple","kind":["lib"]},"executable":null,"#,
        r#""filenames":["/work/apple/target/debug/libapple.rlib"]}"#,
        "\n",
        r#"{"reason":"build-finished","success":true}"#,
        "\n",
    );

    let Run::Built { artifacts, .. } = outcome(stdout, "", true, &workspace()) else {
        panic!("a build");
    };

    assert_eq!(artifacts, Vec::new());
}

/// A verbatim directory still reaches the comparison: a reader can type one into the
/// project box, and `path::absolute` hands it back as given. `Path` reads the prefix of
/// `\\?\C:\work\app` as `VerbatimDisk` and cargo's `C:\work\app\Cargo.toml` as `Disk`, so
/// nothing would be inside the directory being built.
///
/// The strip is what is asserted here: only Windows' own `Path` reads the components.
#[test]
fn a_verbatim_path_is_the_plain_one_it_names() {
    let plain = |text| simplified(Path::new(text)).into_owned();

    assert_eq!(plain(r"\\?\C:\work\app"), PathBuf::from(r"C:\work\app"));
    assert_eq!(plain(r"\\?\C:\"), PathBuf::from(r"C:\"));
    assert_eq!(
        plain(r"\\?\UNC\server\share\app"),
        PathBuf::from(r"\\server\share\app")
    );

    // The verbatim forms naming something no drive letter can are left as they are, as is
    // every path that was never verbatim -- on either platform.
    assert_eq!(plain(r"\\?\pipe\cargo"), PathBuf::from(r"\\?\pipe\cargo"));
    assert_eq!(
        plain(r"\\?\Volume{d0e1}\app"),
        PathBuf::from(r"\\?\Volume{d0e1}\app")
    );
    assert_eq!(plain(r"C:\work\app"), PathBuf::from(r"C:\work\app"));
    assert_eq!(plain("/work/app"), PathBuf::from("/work/app"));
}

/// The same rule end to end, on the platform whose `Path` can read the components: an
/// artifact cargo named is inside the verbatim directory it was built in.
#[cfg(windows)]
#[test]
fn a_verbatim_windows_directory_keeps_its_artifacts() {
    let stdout = concat!(
        r#"{"reason":"compiler-artifact","manifest_path":"C:\\work\\app\\Cargo.toml","#,
        r#""target":{"name":"sketch","kind":["bin"]},"#,
        r#""executable":"C:\\work\\app\\target\\debug\\sketch.exe","#,
        r#""filenames":["C:\\work\\app\\target\\debug\\sketch.exe"]}"#,
        "\n",
        r#"{"reason":"build-finished","success":true}"#,
        "\n",
    );

    let Run::Built { artifacts, .. } = outcome(stdout, "", true, Path::new(r"\\?\C:\work\app"))
    else {
        panic!("a build");
    };

    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].target, "sketch");
}

/// The Unix half of the rule, which is why `path::absolute` is not used on both platforms.
/// cargo derives its manifest paths from `getcwd(2)`, and the kernel answers that with
/// symlinks resolved and `..` gone, so the directory they are matched against is resolved
/// too. `path::absolute` leaves a Unix path alone and would match neither spelling.
#[cfg(unix)]
#[test]
fn a_directory_the_reader_spelled_their_own_way_holds_what_cargo_named() {
    let root = directory(line!());
    let real = root.join("real");
    let link = root.join("link");
    fs::create_dir_all(&real).expect("a directory");
    let _ = fs::remove_file(&link);
    std::os::unix::fs::symlink(&real, &link).expect("a symlink");

    // What cargo names, having been started in either spelling of the same directory.
    let manifest = real.join(MANIFEST);

    assert!(inside(&manifest, &as_cargo_names_it(&link)));
    assert!(inside(
        &manifest,
        &as_cargo_names_it(&real.join("..").join("real"))
    ));
}

/// A library names no executable, so its own files are what it contributes: the `.rlib`,
/// which is an archive this app opens like any other, and never the `.rmeta` beside it,
/// which holds no code.
#[test]
fn a_library_contributes_its_archive_and_not_its_metadata() {
    let stdout = concat!(
        r#"{"reason":"compiler-artifact","manifest_path":"/work/app/crates/analysis/Cargo.toml","#,
        r#""target":{"name":"analysis","kind":["lib"]},"executable":null,"#,
        r#""filenames":["/work/app/target/debug/libanalysis-4039b956ee59af9d.rlib","#,
        r#""/work/app/target/debug/libanalysis-4039b956ee59af9d.rmeta"]}"#,
        "\n",
        r#"{"reason":"build-finished","success":true}"#,
        "\n",
    );

    let Run::Built { artifacts, .. } = outcome(stdout, "", true, &workspace()) else {
        panic!("a build");
    };

    assert_eq!(
        artifacts,
        vec![Artifact {
            path: PathBuf::from("/work/app/target/debug/libanalysis-4039b956ee59af9d.rlib"),
            target: "analysis".to_owned(),
            kind: "lib".to_owned(),
        }]
    );
}

/// A `reason` this module has never heard of is skipped, and the artifact after it still
/// arrives: a future cargo must not be able to empty the list.
#[test]
fn a_message_this_module_does_not_know_is_skipped() {
    let stdout = concat!(
        r#"{"reason":"something-cargo-1.99-emits","what":{"deeply":["nested"]}}"#,
        "\n",
        "not JSON at all\n",
        r#"{"reason":"compiler-artifact","manifest_path":"/work/app/Cargo.toml","#,
        r#""target":{"name":"app","kind":["bin"]},"executable":"/work/app/target/release/app","#,
        r#""filenames":["/work/app/target/release/app"]}"#,
        "\n",
        r#"{"reason":"build-finished","success":true}"#,
        "\n",
    );

    let Run::Built { artifacts, .. } = outcome(stdout, "", true, &workspace()) else {
        panic!("a build");
    };

    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].target, "app");
}

/// Cargo's own defaults, which are the whole reason the view offers to add lines: a
/// release build carries none unless the manifest says so, and a dev build carries them
/// unless the manifest says not.
#[test]
fn a_manifest_that_says_nothing_gets_cargos_answer() {
    let directory = directory(line!());
    fs::write(
        directory.join("Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .expect("a manifest");

    assert!(manifest(&directory).is_some());
    assert!(debug_lines(&directory, Profile::Debug));
    assert!(!debug_lines(&directory, Profile::Release));

    fs::remove_dir_all(&directory).ok();
}

/// The three spellings cargo takes, each in the two directions.
#[test]
fn debug_information_is_read_however_it_is_spelled() {
    let directory = directory(line!());
    let manifest_at = directory.join("Cargo.toml");
    let says = |text: &str| {
        fs::write(&manifest_at, format!("[profile.release]\ndebug = {text}\n"))
            .expect("a manifest");
        debug_lines(&directory, Profile::Release)
    };

    assert!(says("true"));
    assert!(says("2"));
    assert!(says("\"line-tables-only\""));
    assert!(says("\"full\""));
    assert!(!says("false"));
    assert!(!says("0"));
    assert!(!says("\"none\""));

    fs::remove_dir_all(&directory).ok();
}

/// The write is an edit of the reader's own file, so what it must not do is reformat it:
/// the comment, the other tables and the key order all stand.
#[test]
fn adding_debug_lines_keeps_the_rest_of_the_manifest() {
    let directory = directory(line!());
    let manifest_at = directory.join("Cargo.toml");
    let before = "# The app.\n\
                  [package]\n\
                  name = \"app\"\n\n\
                  [dependencies]\n\
                  serde = \"1\"    # pinned by hand\n";
    fs::write(&manifest_at, before).expect("a manifest");

    add_debug_lines(&directory, Profile::Release).expect("the write");

    let after = fs::read_to_string(&manifest_at).expect("the file");
    assert!(after.starts_with(before), "{after}");
    assert!(after.contains("[profile.release]"), "{after}");
    assert!(after.contains("debug = \"line-tables-only\""), "{after}");
    // Made implicit, so the file gains the one header it needs and no empty `[profile]`.
    assert!(!after.contains("\n[profile]\n"), "{after}");
    assert!(debug_lines(&directory, Profile::Release));

    fs::remove_dir_all(&directory).ok();
}

/// A profile the manifest already has keeps everything else it said.
#[test]
fn adding_debug_lines_to_a_profile_that_is_there_keeps_its_other_keys() {
    let directory = directory(line!());
    let manifest_at = directory.join("Cargo.toml");
    fs::write(
        &manifest_at,
        "[profile.release]\nlto = true\ndebug = false\n",
    )
    .expect("a manifest");

    add_debug_lines(&directory, Profile::Release).expect("the write");

    let after = fs::read_to_string(&manifest_at).expect("the file");
    assert!(after.contains("lto = true"), "{after}");
    assert!(after.contains("debug = \"line-tables-only\""), "{after}");
    assert!(!after.contains("debug = false"), "{after}");

    fs::remove_dir_all(&directory).ok();
}

/// cargo takes `[profile.*]` from the **workspace root** and ignores a member's own table,
/// with a warning, so a project opened at a member is asked about the root's manifest and
/// the offer to add lines edits that file. Reading the member's own would go on offering
/// lines the root already asks for, and writing it would leave the view saying the lines are
/// there while the build carries none.
#[test]
fn a_members_profiles_are_the_workspace_roots() {
    let root = directory(line!());
    let root_manifest = root.join("Cargo.toml");
    fs::write(
        &root_manifest,
        "[workspace]\nmembers = [\"crates/*\"]\n\n[profile.release]\ndebug = 1\n",
    )
    .expect("the root manifest");

    let member = root.join("crates").join("one");
    fs::create_dir_all(&member).expect("the directory");
    let own = member.join("Cargo.toml");
    fs::write(&own, "[package]\nname = \"one\"\nversion = \"0.1.0\"\n").expect("a manifest");

    assert_eq!(profile_manifest(&member), root_manifest);
    // The root asks for debug information; the member's own file says nothing at all.
    assert!(debug_lines(&member, Profile::Release));

    // And the edit goes where the build reads, leaving the member's manifest as it was.
    let before = fs::read_to_string(&own).expect("the file");
    add_debug_lines(&member, Profile::Debug).expect("the write");

    assert_eq!(fs::read_to_string(&own).expect("the file"), before);
    let after = fs::read_to_string(&root_manifest).expect("the file");
    assert!(after.contains("[profile.dev]"), "{after}");

    fs::remove_dir_all(&root).ok();
}

/// Where the walk up stops. A manifest with a `[workspace]` table of its own **is** a root,
/// which is what keeps a scratchpad reading its own profiles wherever the state directory
/// turns out to be -- its generated manifest carries an empty one for exactly that reason.
/// And a package that names its root outright is taken at its word, ancestor or not.
#[test]
fn a_package_that_is_its_own_workspace_stops_the_walk() {
    let root = directory(line!());
    fs::write(root.join("Cargo.toml"), "[workspace]\n").expect("the root manifest");

    let other = root.join("other");
    fs::create_dir_all(&other).expect("the directory");
    fs::write(
        other.join("Cargo.toml"),
        "[workspace]\n\n[profile.release]\ndebug = 1\n",
    )
    .expect("a manifest");

    // Its own workspace, so the root above it is not asked and its silence is the answer.
    let inner = root.join("inner");
    fs::create_dir_all(&inner).expect("the directory");
    fs::write(
        inner.join("Cargo.toml"),
        "[package]\nname = \"inner\"\n\n[workspace]\n",
    )
    .expect("a manifest");

    assert_eq!(profile_manifest(&inner), inner.join("Cargo.toml"));
    assert!(!debug_lines(&inner, Profile::Release));

    // Named outright: the root the walk would have found says nothing, and this one does.
    let named = root.join("named");
    fs::create_dir_all(&named).expect("the directory");
    fs::write(
        named.join("Cargo.toml"),
        "[package]\nname = \"named\"\nworkspace = \"../other\"\n",
    )
    .expect("a manifest");

    assert!(debug_lines(&named, Profile::Release));

    fs::remove_dir_all(&root).ok();
}

/// A directory with no manifest in it is a placeholder and not an error: nothing to build,
/// nothing to say about its profiles, and a write that fails rather than making one.
#[test]
fn a_directory_with_no_manifest_is_not_a_workspace() {
    let directory = directory(line!());

    assert_eq!(manifest(&directory), None);
    assert!(!debug_lines(&directory, Profile::Release));
    assert!(add_debug_lines(&directory, Profile::Release).is_err());

    fs::remove_dir_all(&directory).ok();
}

fn span(line: usize, column: usize) -> Span {
    Span {
        file: "src/main.rs".to_owned(),
        line,
        column,
    }
}

/// A span is a line and a column the way rustc counts them; a cursor is one number the way
/// an editor counts it. This is the whole of the conversion, and the unit is UTF-16 code
/// units because that is what a cursor position is.
#[test]
fn a_span_is_a_cursor_position() {
    let source = "fn main() {\n    let x = 1;\n}\n";

    // One-based, both halves: line 2 column 5 is the `l` of `let`, which is char 16.
    assert_eq!(span(2, 5).offset_in(source), 16);
    // The first character of the file, which is where a span with no useful place lands.
    assert_eq!(span(1, 1).offset_in(source), 0);
    // The line break is not on the line: the last line is the empty one after it.
    assert_eq!(span(3, 1).offset_in(source), 27);
    assert_eq!(span(4, 1).offset_in(source), source.len());
}

/// A column is counted in characters and a cursor in UTF-16 code units, so a line with an
/// astral character in it is where the two disagree — one character, two code units. A
/// cursor placed by character count would sit one place left of the span for every one of
/// them before it.
#[test]
fn a_column_is_characters_and_a_cursor_is_code_units() {
    // `é` is one char and one code unit; `𝄞` is one char and two.
    let source = "// é𝄞 x\nlet y = 2;\n";

    // Column 7 is the `x`: six characters before it — `/`, `/`, ` `, `é`, `𝄞`, ` ` — which
    // are seven code units, the `𝄞` being two.
    assert_eq!(span(1, 7).offset_in(source), 7);
    // And the line below starts after the whole of the line above, its break included:
    // eight characters, nine code units.
    assert_eq!(span(2, 1).offset_in(source), 9);
}

/// The source is edited under a diagnostic — the reader has usually typed since the build —
/// so a span that no longer fits is clamped rather than dropped. Nowhere near a panic and
/// never past the end of the text.
#[test]
fn a_span_the_source_has_outgrown_is_clamped() {
    let source = "fn main() {}\n";

    // Past the end of its line: the end of that line, and not the line below.
    assert_eq!(span(1, 500).offset_in(source), 12);
    // Past the end of the file: the end of the file.
    assert_eq!(span(99, 1).offset_in(source), source.len());
    // Nothing to point at at all.
    assert_eq!(span(1, 1).offset_in(""), 0);
    // Zero is not a line rustc writes, and is the first line rather than a subtraction
    // that wraps.
    assert_eq!(span(0, 0).offset_in(source), 0);
}
