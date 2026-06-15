//! Shared core types. Enum discriminants are stable `i64` (persisted in SQLite) —
//! never reorder/renumber without a migration.

use serde::{Deserialize, Serialize};

macro_rules! stable_enum {
    ($(#[$m:meta])* $vis:vis enum $name:ident { $($variant:ident = $val:expr),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[repr(i64)]
        $vis enum $name { $($variant = $val),+ }

        impl $name {
            #[inline]
            pub fn as_i64(self) -> i64 { self as i64 }

            pub fn from_i64(v: i64) -> Option<Self> {
                match v { $($val => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

/// Stable symbol id (integer surrogate, never reused across reindex). Rendered as `sym:N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SymbolId(pub i64);

impl std::fmt::Display for SymbolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sym:{}", self.0)
    }
}

stable_enum! {
    pub enum Language {
        JavaScript = 0,
        TypeScript = 1,
        Python = 2,
        Go = 3,
        Rust = 4,
        Java = 5,
        Kotlin = 6,
        Swift = 7,
        C = 8,
        Cpp = 9,
        CSharp = 10,
        Php = 11,
        Clojure = 12,
    }
}

stable_enum! {
    pub enum SymbolKind {
        Function = 0,
        Method = 1,
        Class = 2,
        Struct = 3,
        Enum = 4,
        Interface = 5,
        Trait = 6,
        Module = 7,
        Field = 8,
        Variable = 9,
        Const = 10,
        Test = 11,
        Macro = 12,
        TypeAlias = 13,
        Variant = 14,
    }
}

stable_enum! {
    pub enum Provenance {
        TreeSitter = 0,
        StackGraphs = 1,
        Scip = 2,
        Lsp = 3,
        Text = 4,
    }
}

impl Provenance {
    /// Trust rank (higher = more trustworthy): scip > lsp > stack_graphs > tree_sitter > text.
    pub fn trust_rank(self) -> u8 {
        match self {
            Provenance::Scip => 4,
            Provenance::Lsp => 3,
            Provenance::StackGraphs => 2,
            Provenance::TreeSitter => 1,
            Provenance::Text => 0,
        }
    }

    pub fn abbrev(self) -> &'static str {
        match self {
            Provenance::TreeSitter => "ts",
            Provenance::StackGraphs => "sg",
            Provenance::Scip => "scip",
            Provenance::Lsp => "lsp",
            Provenance::Text => "text",
        }
    }
}

stable_enum! {
    pub enum Resolution {
        Resolved = 0,
        Ambiguous = 1,
        Unresolved = 2,
    }
}

impl Resolution {
    pub fn abbrev(self) -> &'static str {
        match self {
            Resolution::Resolved => "resolved",
            Resolution::Ambiguous => "ambiguous",
            Resolution::Unresolved => "unresolved",
        }
    }
}

stable_enum! {
    pub enum Role {
        Definition = 0,
        Reference = 1,
        Read = 2,
        Write = 3,
        Call = 4,
    }
}

stable_enum! {
    pub enum EdgeKind {
        Contains = 0,
        Calls = 1,
        References = 2,
        Reads = 3,
        Writes = 4,
        Imports = 5,
        Extends = 6,
        Implements = 7,
    }
}

/// Source range, 1-based lines, separate columns (no bit-packing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub source: SymbolId,
    pub target: SymbolId,
    pub kind: EdgeKind,
    pub provenance: Provenance,
    pub resolution: Resolution,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_roundtrip_is_stable() {
        for v in 0..=12 {
            assert_eq!(Language::from_i64(v).unwrap().as_i64(), v);
        }
        assert_eq!(Provenance::from_i64(2), Some(Provenance::Scip));
        assert_eq!(Provenance::from_i64(99), None);
        assert_eq!(EdgeKind::Calls.as_i64(), 1);
    }

    #[test]
    fn trust_order_scip_beats_treesitter() {
        assert!(Provenance::Scip.trust_rank() > Provenance::TreeSitter.trust_rank());
        assert!(Provenance::Lsp.trust_rank() > Provenance::StackGraphs.trust_rank());
        assert!(Provenance::TreeSitter.trust_rank() > Provenance::Text.trust_rank());
    }

    #[test]
    fn symbol_id_renders_as_sym() {
        assert_eq!(SymbolId(204).to_string(), "sym:204");
    }
}
