/* A fixture for `crates/analysis/tests/real_object.rs`: three tiny functions on known
 * lines, compiled by a real toolchain so the crate is pinned against the DWARF a compiler
 * actually emits and not only against what `tests/common` synthesizes in memory.
 *
 * `line_fixture.o` beside this file is committed and is never rebuilt by the test suite.
 * Regenerate it, from this directory, with exactly:
 *
 *     gcc -gdwarf-5 -O0 -fdebug-prefix-map="$PWD"=/fixture -c line_fixture.c -o line_fixture.o
 *
 * built with gcc (GCC) 16.1.1 20260515 (Red Hat 16.1.1-2), target x86_64-redhat-linux-gnu.
 *
 * `-fdebug-prefix-map` is what keeps the committed object machine-independent: without it
 * `DW_AT_comp_dir` would name whoever's checkout it was built in. `-O0` keeps each
 * statement on a line-table row of its own. `-gdwarf-5` only pins what this gcc already
 * defaults to, so the fixture cannot silently change DWARF version under a newer compiler.
 *
 * The line numbers below are asserted by the test. Do not renumber this file without
 * rebuilding the object and updating them.
 */

int add(int a, int b)
{
	return a + b;
}

int twice(int n)
{
	return add(n, n);
}

int sum_to(int n)
{
	int total = 0;

	for (int i = 1; i <= n; i++)
		total = add(total, i);

	return total;
}
