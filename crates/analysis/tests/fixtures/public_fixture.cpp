/* The second object of the `line_fixture_public` DLL + PDB pair (`tests/pdb.rs`): one
 * function, compiled **without** `/Z7`, so no module in the PDB has a symbol for it and the
 * only name the PDB holds for it is the linker's public (`S_PUB32`) — and, being C++, that
 * name is decorated (`?helper@@YAHXZ`), which is what pins the demangling of a public. The
 * build commands are in `tests/pdb.rs`'s header, beside the two other pairs'.
 */

int helper(void) { return 7; }
