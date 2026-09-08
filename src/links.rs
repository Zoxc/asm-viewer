//! Which names in a source file are links, out of what a language server calls them.
//!
//! The server classifies every name in a file at once (`lsp::Talk::semantic_tokens`), and
//! this is the rule that turns its vocabulary into the app's one question: is this a name
//! the reader can follow, and which question does following it ask. Nothing here knows the
//! UI, and nothing here talks to a server.
//!
//! **A name is a link when the server can place it.** What it cannot place is a built-in
//! type -- `u32` is nowhere -- and a name it could not resolve at all; neither is in the
//! set below, so neither is a link. A name **where one is defined** is not a link either,
//! which the `declaration` modifier says outright and which the app used to guess from the
//! `fn` keyword in front of it.
//!
//! The exception is an item in a trait `impl`. It carries `declaration` like any other
//! definition, but there is somewhere to go from it: the trait's own declaration, which is
//! a different question of the server than a definition (`textDocument/declaration`, and
//! the two genuinely disagree -- a call to a trait method is defined in the `impl` that
//! runs and declared in the trait). `declaration` and `trait` together are what say so.
//!
//! One thing this is deliberately loose about. The `!` of a `macro_rules!` is a `macro`
//! token like the `!` of a macro *use*, with nothing to tell them apart, so it is a link
//! that the server then places nowhere -- a click that does nothing, which is what the
//! spec says a link the server cannot place does anyway (`notes/specs/Split View.md`).
//! Telling the two apart would mean reading the row's text here, which is the pane's.

use std::ops::Range;

use crate::lsp;
use crate::shared::Shared;

/// One name the server placed, or could have: where it is, and what following it asks.
///
/// A name with nothing to follow is kept rather than dropped, because the reader can still
/// ask about it -- the menu is offered where a name is **defined** as much as where it is
/// used, that being where a reader asks what refers to it (`notes/specs/Split View.md`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Link {
    /// 1-based, as every line in the app is.
    pub line: u32,
    /// The columns of the name on `line`, as byte offsets into it -- the server's own
    /// units (`lsp::Token`). The pane converts them to the columns it draws in.
    pub columns: Range<u32>,
    /// What following it asks the server, and `None` where there is nothing to follow.
    /// The server's own type, so nothing maps a link's question onto a wire one
    /// (`lsp::Question`).
    pub asks: Option<lsp::Followed>,
}

/// Every name in one file the server had something to say about, in the order they are
/// drawn: by line, and by column inside a line.
pub type Links = Shared<Link>;

impl Links {
    /// The links among `tokens`, as `legend` spells them.
    ///
    /// The answer arrives in the order the server walked the file, which is already this
    /// order; it is sorted all the same, since [`Links::on_line`] is a binary search and a
    /// server that answered out of order would otherwise lose rows rather than draw them
    /// wrongly.
    pub fn of(legend: &lsp::Legend, tokens: &[lsp::Token]) -> Links {
        // What the legend says is worked out once and not per token: which of its types
        // are names, and which bits the two modifiers are. A file is thousands of tokens
        // and the legend is fixed for the life of the conversation.
        let named = legend.kinds_named(&NAMES);
        let declaration = legend.bit("declaration");
        let trait_ = legend.bit("trait");
        let mut links: Vec<Link> = tokens
            .iter()
            .filter(|token| named.get(token.kind as usize) == Some(&true))
            .map(|token| Link {
                line: token.line,
                columns: token.columns.clone(),
                asks: asked_by(token, declaration, trait_),
            })
            .collect();
        links.sort_by(|one, other| {
            (one.line, one.columns.start).cmp(&(other.line, other.columns.start))
        });
        links.into()
    }

    /// The columns of the names on `line` that can be followed, in the order they are
    /// drawn: what a row draws as links, once the pane has counted them in its own units.
    /// Borrowed rather than collected: a row asks on every render.
    pub fn followed_on(&self, line: u32) -> impl Iterator<Item = &Range<u32>> + '_ {
        self.on_line(line)
            .iter()
            .filter(|link| link.asks.is_some())
            .map(|link| &link.columns)
    }

    /// Every name on `line`, 1-based, in the order they are drawn.
    pub fn on_line(&self, line: u32) -> &[Link] {
        let from = self.partition_point(|link| link.line < line);
        let rest = &self[from..];
        let to = rest.partition_point(|link| link.line == line);
        &rest[..to]
    }

    /// The name byte `column` is inside on `line`, and `None` where it is over none.
    /// Followed or not: where a name is defined is where a reader asks what refers to it.
    pub fn at(&self, line: u32, column: u32) -> Option<&Link> {
        self.on_line(line)
            .iter()
            .find(|link| link.columns.contains(&column))
    }
}

/// The token types that are a **name** -- something the reader can put a question to the
/// server about. Everything else it can call a run of text is either lexical -- a keyword,
/// an operator, a string, a comment, punctuation -- or one of the three it places nowhere:
/// `builtinType`, `builtinAttribute` and `unresolvedReference`.
///
/// Longer than the standard list because a server may say more than the standard: the
/// first twelve are the specification's own and the rest are rust-analyzer's, and a name a
/// server does not have simply never arrives. **A local is here too** -- a `let` binding, a
/// parameter, a lifetime -- since the server places those as readily as it places an item,
/// and following one goes to where it was bound.
const NAMES: [&str; 27] = [
    // The specification's own.
    "function",
    "method",
    "macro",
    "struct",
    "enum",
    "interface",
    "typeParameter",
    "enumMember",
    "property",
    "namespace",
    "type",
    "decorator",
    // rust-analyzer's additions.
    "procMacro",
    "union",
    "typeAlias",
    "const",
    "static",
    "derive",
    "deriveHelper",
    "toolModule",
    // Locals, which go to where the name was bound.
    "variable",
    "parameter",
    "constParameter",
    "selfKeyword",
    "selfTypeKeyword",
    "lifetime",
    "label",
];

/// What following `token` asks, and `None` where there is nothing to follow. Asked only
/// of a name: whether a token is one is its type's to say, and settled before this
/// ([`Links::of`]).
///
/// `declaration` and `trait_` are those two modifiers' bits, taken off the legend once
/// (`lsp::Legend::bit`). A modifier the server never declared is `0`, which no token has
/// set, so a name whose declaration cannot be spoken of is taken as a use -- the link the
/// reader can follow rather than the one they cannot.
fn asked_by(token: &lsp::Token, declaration: u32, trait_: u32) -> Option<lsp::Followed> {
    if token.modifiers & declaration == 0 {
        return Some(lsp::Followed::Definition);
    }
    // A definition, so there is nothing to follow -- unless it is an item in a trait
    // `impl`, where the trait declares what this one writes out.
    match token.modifiers & trait_ != 0 {
        true => Some(lsp::Followed::Declaration),
        false => None,
    }
}

#[cfg(test)]
mod tests;
