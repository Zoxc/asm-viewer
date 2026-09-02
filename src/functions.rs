//! The functions a source file defines, by the lines each one spans, and which of them a
//! line is inside.
//!
//! "The function the reader is on" is a question about the *source* and not about the
//! debug info: DWARF's `DW_AT_decl_line` is one line and disagrees with the line program
//! by a line where the prologue sits, and a symbol's own rows over-cover wherever the
//! compiler inlined something into it. The file is already parsed by tree-sitter when it
//! is loaded, so the answer is read off that parse -- the grammar's function node, from
//! its first line to its last -- and kept as a list of spans, the tree itself being
//! dropped with the highlighter's.
//!
//! What counts as a function is the grammar's own definition for C and C++
//! (`function_definition`), and for Rust the `fn` keyword and the block after its
//! signature, found by a scanner of this module's own ([`rust`]) because the Rust
//! grammar is behind the compiler and its error recovery loses whole files. A closure or
//! a lambda is not one -- the reader asking about a closure's line is asking about the
//! function it is written inside -- and a bodiless declaration has no lines to have been
//! compiled from.
//!
//! Framework-free and unit-tested: the walk is over a tree tree-sitter built from the
//! file's bytes, and it is iterative, since the depth of that tree is the file's to
//! decide.

use std::ops::RangeInclusive;

use tree_sitter::{Node, Tree};

pub mod rust;

/// One function definition: what the source calls it, and the lines it spans, 1-based
/// as DWARF's are. The name is the identifier alone -- `new`, not `Vec::new` -- since
/// that is all the grammar is asked for and all the heading it goes into needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub name: String,
    pub lines: RangeInclusive<u32>,
}

/// Every function `tree` defines, in the order they begin -- an enclosing function
/// before the ones inside it, which is what [`enclosing`] leans on. For a C or C++
/// tree; Rust is [`rust::functions`], answering in the same shape.
///
/// `text` is the bytes the tree was parsed from; a node's name is read out of it.
/// A function whose position does not fit a `u32` is left out rather than clamped.
pub fn functions(tree: &Tree, text: &[u8]) -> Vec<Function> {
    let mut found = Vec::new();
    let mut cursor = tree.walk();

    // Pre-order, with the cursor and no recursion: down into the first child while
    // there is one, else across to the next sibling, else up until there is one.
    loop {
        let node = cursor.node();
        if let Some(function) = function_of(node, text) {
            found.push(function);
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return found;
            }
        }
    }
}

/// The innermost of `functions` whose lines hold `line`, or `None` between functions.
///
/// The last span containing the line, since [`functions`] lists an enclosing function
/// before the ones nested in it and spans nest properly.
pub fn enclosing(functions: &[Function], line: u32) -> Option<&Function> {
    functions
        .iter()
        .rev()
        .find(|function| function.lines.contains(&line))
}

/// `node` as a [`Function`], if it is one.
fn function_of(node: Node, text: &[u8]) -> Option<Function> {
    let named = match node.kind() {
        "function_item" => node.child_by_field_name("name")?,
        "function_definition" => innermost_declarator(node)?,
        _ => return None,
    };
    let name = named.utf8_text(text).ok()?.to_owned();
    let line = |row: usize| u32::try_from(row).ok()?.checked_add(1);
    let first = line(node.start_position().row)?;
    let last = line(node.end_position().row)?;
    Some(Function {
        name,
        lines: first..=last,
    })
}

/// The name inside a C or C++ declarator chain: `int *f(void)` is a pointer declarator
/// around a function declarator around `f`, and `T &ns::f()` a reference declarator
/// around one around `ns::f`. Down the `declarator` field where the grammar names one,
/// and where it does not (`reference_declarator`, `parenthesized_declarator`) to the
/// child that is a declarator, else the first that is neither a modifier nor a parse
/// error -- `int (S::*member())(int)` leaves an `ERROR` beside the declarator, the
/// grammar not knowing a pointer to member -- until the node is no declarator at all:
/// an identifier, a qualified name, an `operator<<`, a destructor.
///
/// Bounded, since how deep the chain goes is the file's decision.
fn innermost_declarator(node: Node) -> Option<Node> {
    let mut node = node.child_by_field_name("declarator")?;
    for _ in 0..64 {
        if !node.kind().ends_with("_declarator") {
            return Some(node);
        }
        node = match node.child_by_field_name("declarator") {
            Some(inner) => inner,
            None => {
                let mut cursor = node.walk();
                let children: Vec<Node> = node.named_children(&mut cursor).collect();
                let declarator = children
                    .iter()
                    .find(|child| child.kind().ends_with("_declarator"));
                let named = children
                    .iter()
                    .find(|child| !is_modifier(child) && !child.is_error());
                *declarator.or(named)?
            }
        };
    }
    None
}

/// The named children of a declarator that are not the declarator inside it.
fn is_modifier(node: &Node) -> bool {
    matches!(
        node.kind(),
        "ms_call_modifier" | "ms_pointer_modifier" | "ms_based_modifier" | "type_qualifier"
    )
}

#[cfg(test)]
mod tests;
