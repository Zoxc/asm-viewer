use super::*;
use std::sync::atomic::Ordering::Relaxed;

/// The committed pair `tests/pdb.rs` reads, as the image's bytes and the path it was read
/// from, which is what [`Pdb::load`] finds the `.pdb` beside.
fn fixture() -> (Vec<u8>, PathBuf) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/line_fixture_public.dll");
    let bytes = std::fs::read(&path).expect("the committed line_fixture_public.dll");
    (bytes, path)
}

/// One question over every module walks the DBI module list once. Walking it per module
/// costs the square of a count the file states, which a hostile PDB turns into a hang.
#[test]
fn every_module_is_decoded_in_one_walk_of_the_module_list() {
    let (bytes, path) = fixture();
    let file = object::File::parse(&*bytes).expect("a PE");
    let pdb = Pdb::load(&file, &path).expect("the .pdb beside it");

    let before = pdb.walks.load(Relaxed);
    let mut rows = 0;
    pdb.each_row(&mut |_, _, _| rows += 1);
    assert_eq!(pdb.walks.load(Relaxed) - before, 1);
    assert!(rows > 0);

    // Every module is remembered, so their count is what one walk each would have cost --
    // and it is more than one, or the two would not differ.
    let modules = pdb.modules.lock().expect("no panic under the lock");
    assert!(modules.len() > 1, "{} modules", modules.len());
}

/// Two modules at one address, neither decoded, cost one walk of the module list between
/// them when their extent is asked for, as a question over every module does.
#[test]
fn the_modules_at_an_address_are_decoded_in_one_walk() {
    let (bytes, path) = fixture();
    let file = object::File::parse(&*bytes).expect("a PE");
    let mut pdb = Pdb::load(&file, &path).expect("the .pdb beside it");
    // An address no procedure begins at, so neither module answers and both are read.
    let start = SectionAddress::new(1);
    let end = SectionAddress::new(2);
    pdb.contributions = Intervals::new([(start..end, 0), (start..end, 1)]);

    let before = pdb.walks.load(Relaxed);
    assert_eq!(pdb.extent(PlacedAddress::new(1)), None);
    assert_eq!(pdb.walks.load(Relaxed) - before, 1);
}

/// The path a binary records is a name, not a place to reach. On Windows a UNC path is
/// absolute, and opening `\\host\share\x.pdb` logs the machine in to `host` over SMB before
/// a byte comes back, so nothing outside the binary's own directory is tried for one.
#[test]
fn a_unc_path_a_binary_records_is_never_a_candidate() {
    let binary = Path::new("/tmp/somewhere/foo.dll");
    for recorded in [r"\\server\share\foo.pdb", "//server/share/foo.pdb"] {
        let tried = candidates(recorded, binary);
        assert_eq!(
            tried,
            [PathBuf::from("/tmp/somewhere/foo.pdb")],
            "{recorded}"
        );
    }
}

/// A plain absolute path is still tried — a build directory that is still there holds the
/// `.pdb` the binary was linked against — but after the two names beside the binary, which
/// name a file the reader already has. Unix spelling: Windows's absolute path is `C:\...`.
#[cfg(unix)]
#[test]
fn the_recorded_path_is_tried_after_the_names_beside_the_binary() {
    let tried = candidates("/build/dir/bar.pdb", Path::new("/tmp/somewhere/foo.dll"));
    assert_eq!(
        tried,
        [
            PathBuf::from("/tmp/somewhere/bar.pdb"),
            PathBuf::from("/tmp/somewhere/foo.pdb"),
            PathBuf::from("/build/dir/bar.pdb"),
        ]
    );
}
