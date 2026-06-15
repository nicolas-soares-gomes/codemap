//! Tier0 symbol extraction via tree-sitter. M1: Rust. Other languages land in M5, each
//! with its own extractor and extraction test.

use crate::types::{Language, Range, SymbolKind};
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    pub name: String,
    pub name_path: String,
    pub kind: SymbolKind,
    pub range: Range,
    pub sel_line: u32,
    pub sel_col: u32,
}

pub fn ts_language(lang: Language) -> Option<tree_sitter::Language> {
    match lang {
        Language::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
        _ => None,
    }
}

pub fn extract(lang: Language, source: &str) -> Vec<Extracted> {
    let Some(ts_lang) = ts_language(lang) else {
        return Vec::new();
    };
    let mut parser = Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let src = source.as_bytes();
    let mut out = Vec::new();
    if lang == Language::Rust {
        let mut scope: Vec<String> = Vec::new();
        walk_rust(tree.root_node(), src, &mut scope, false, &mut out);
    }
    out
}

fn walk_rust(node: Node, src: &[u8], scope: &mut Vec<String>, in_typeish: bool, out: &mut Vec<Extracted>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            // function_signature_item = body-less fn (trait, extern).
            "function_item" | "function_signature_item" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    let kind = if in_typeish { SymbolKind::Method } else { SymbolKind::Function };
                    emit(out, node_text(nn, src), scope, kind, child, Some(nn));
                }
                walk_rust(child, src, scope, false, out);
            }
            "struct_item" | "union_item" => push_named(child, src, scope, SymbolKind::Struct, false, out),
            "enum_item" => push_named(child, src, scope, SymbolKind::Enum, false, out),
            "trait_item" => push_named(child, src, scope, SymbolKind::Trait, true, out),
            "mod_item" => push_named(child, src, scope, SymbolKind::Module, false, out),
            "impl_item" => {
                let tname = field_text(child, "type", src)
                    .or_else(|| field_text(child, "trait", src))
                    .unwrap_or_default();
                scope.push(strip_generics(&tname));
                walk_rust(child, src, scope, true, out);
                scope.pop();
            }
            "enum_variant" => emit_named(child, src, scope, SymbolKind::Variant, out),
            "field_declaration" => emit_named(child, src, scope, SymbolKind::Field, out),
            "const_item" | "static_item" => emit_named(child, src, scope, SymbolKind::Const, out),
            "type_item" => emit_named(child, src, scope, SymbolKind::TypeAlias, out),
            "macro_definition" => emit_named(child, src, scope, SymbolKind::Macro, out),
            _ => walk_rust(child, src, scope, in_typeish, out),
        }
    }
}

fn push_named(node: Node, src: &[u8], scope: &mut Vec<String>, kind: SymbolKind, typeish_children: bool, out: &mut Vec<Extracted>) {
    if let Some(nn) = node.child_by_field_name("name") {
        let name = node_text(nn, src).to_string();
        emit(out, &name, scope, kind, node, Some(nn));
        scope.push(name);
        walk_rust(node, src, scope, typeish_children, out);
        scope.pop();
    } else {
        walk_rust(node, src, scope, false, out);
    }
}

fn emit_named(node: Node, src: &[u8], scope: &[String], kind: SymbolKind, out: &mut Vec<Extracted>) {
    if let Some(nn) = node.child_by_field_name("name") {
        emit(out, node_text(nn, src), scope, kind, node, Some(nn));
    }
}

fn emit(out: &mut Vec<Extracted>, name: &str, scope: &[String], kind: SymbolKind, node: Node, name_node: Option<Node>) {
    if name.is_empty() {
        return;
    }
    let name_path = if scope.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", scope.join("/"), name)
    };
    let sp = node.start_position();
    let ep = node.end_position();
    let np = name_node.map(|n| n.start_position()).unwrap_or(sp);
    out.push(Extracted {
        name: name.to_string(),
        name_path,
        kind,
        range: Range {
            start_line: sp.row as u32 + 1,
            start_col: sp.column as u32,
            end_line: ep.row as u32 + 1,
            end_col: ep.column as u32,
        },
        sel_line: np.row as u32 + 1,
        sel_col: np.column as u32,
    });
}

/// A call site: the callee name (rightmost identifier) and the call's range.
#[derive(Debug, Clone)]
pub struct CallSite {
    pub name: String,
    pub range: Range,
}

pub fn extract_calls(lang: Language, source: &str) -> Vec<CallSite> {
    let Some(ts_lang) = ts_language(lang) else {
        return Vec::new();
    };
    let mut parser = Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let src = source.as_bytes();
    let mut out = Vec::new();
    if lang == Language::Rust {
        collect_calls_rust(tree.root_node(), src, &mut out);
    }
    out
}

fn collect_calls_rust(node: Node, src: &[u8], out: &mut Vec<CallSite>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(name) = child.child_by_field_name("function").and_then(|f| callee_name(f, src)) {
                let sp = child.start_position();
                let ep = child.end_position();
                out.push(CallSite {
                    name,
                    range: Range {
                        start_line: sp.row as u32 + 1,
                        start_col: sp.column as u32,
                        end_line: ep.row as u32 + 1,
                        end_col: ep.column as u32,
                    },
                });
            }
        }
        collect_calls_rust(child, src, out);
    }
}

fn callee_name(func: Node, src: &[u8]) -> Option<String> {
    match func.kind() {
        "identifier" => Some(node_text(func, src).to_string()),
        "field_expression" => func.child_by_field_name("field").map(|n| node_text(n, src).to_string()),
        "scoped_identifier" => func.child_by_field_name("name").map(|n| node_text(n, src).to_string()),
        "generic_function" => func.child_by_field_name("function").and_then(|f| callee_name(f, src)),
        _ => None,
    }
}

fn node_text<'a>(n: Node, src: &'a [u8]) -> &'a str {
    std::str::from_utf8(&src[n.byte_range()]).unwrap_or("")
}

fn field_text(n: Node, field: &str, src: &[u8]) -> Option<String> {
    n.child_by_field_name(field).map(|c| node_text(c, src).to_string())
}

fn strip_generics(s: &str) -> String {
    s.split(['<', ' ', '\n']).next().unwrap_or(s).trim_start_matches('&').trim().to_string()
}
