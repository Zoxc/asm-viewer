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
///
/// The only test that reads [`WALKS`], which counts for the whole process.
#[test]
fn every_module_is_decoded_in_one_walk_of_the_module_list() {
    let (bytes, path) = fixture();
    let file = object::File::parse(&*bytes).expect("a PE");
    let pdb = Pdb::load(&file, &path).expect("the .pdb beside it");

    let before = WALKS.load(Relaxed);
    let mut rows = 0;
    pdb.each_row(&mut |_, _, _| rows += 1);
    assert_eq!(WALKS.load(Relaxed) - before, 1);
    assert!(rows > 0);

    // Every module is remembered, so their count is what one walk each would have cost --
    // and it is more than one, or the two would not differ.
    let modules = pdb.modules.lock().expect("no panic under the lock");
    assert!(modules.len() > 1, "{} modules", modules.len());
}
