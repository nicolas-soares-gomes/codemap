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
        Language::TypeScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Language::Python => Some(tree_sitter_python::LANGUAGE.into()),
        Language::Go => Some(tree_sitter_go::LANGUAGE.into()),
        Language::Java => Some(tree_sitter_java::LANGUAGE.into()),
        Language::CSharp => Some(tree_sitter_c_sharp::LANGUAGE.into()),
        Language::Php => Some(tree_sitter_php::LANGUAGE_PHP.into()),
        Language::C => Some(tree_sitter_c::LANGUAGE.into()),
        Language::Cpp => Some(tree_sitter_cpp::LANGUAGE.into()),
        Language::Swift => Some(tree_sitter_swift::LANGUAGE.into()),
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
    let mut scope: Vec<String> = Vec::new();
    match lang {
        Language::Rust => walk_rust(tree.root_node(), src, &mut scope, false, &mut out),
        Language::TypeScript => walk_ts(tree.root_node(), src, &mut scope, &mut out),
        Language::Python => walk_py(tree.root_node(), src, &mut scope, false, &mut out),
        Language::Go => walk_go(tree.root_node(), src, &mut scope, &mut out),
        Language::Java => walk_java(tree.root_node(), src, &mut scope, &mut out),
        Language::CSharp => walk_csharp(tree.root_node(), src, &mut scope, &mut out),
        Language::Php => walk_php(tree.root_node(), src, &mut scope, &mut out),
        Language::C | Language::Cpp => walk_c(tree.root_node(), src, &mut scope, false, &mut out),
        Language::Swift => walk_swift(tree.root_node(), src, &mut scope, false, &mut out),
        _ => {}
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
    match lang {
        Language::Rust => collect_calls_rust(tree.root_node(), src, &mut out),
        Language::TypeScript => collect_calls_ts(tree.root_node(), src, &mut out),
        Language::Python => collect_calls_py(tree.root_node(), src, &mut out),
        Language::Go => collect_calls_go(tree.root_node(), src, &mut out),
        Language::Java => collect_calls_java(tree.root_node(), src, &mut out),
        Language::CSharp => collect_calls_csharp(tree.root_node(), src, &mut out),
        Language::Php => collect_calls_php(tree.root_node(), src, &mut out),
        Language::C | Language::Cpp => collect_calls_c(tree.root_node(), src, &mut out),
        Language::Swift => collect_calls_swift(tree.root_node(), src, &mut out),
        _ => {}
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

// ---- Swift -----------------------------------------------------------------

fn walk_swift(node: Node, src: &[u8], scope: &mut Vec<String>, in_type: bool, out: &mut Vec<Extracted>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "class_declaration" => {
                let is_enum = child.child_by_field_name("body").map(|b| b.kind()) == Some("enum_class_body");
                let kind = if is_enum { SymbolKind::Enum } else { SymbolKind::Class };
                push_named_swift(child, src, scope, kind, out);
            }
            "protocol_declaration" => push_named_swift(child, src, scope, SymbolKind::Interface, out),
            "enum_entry" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    emit(out, node_text(nn, src), scope, SymbolKind::Variant, child, Some(nn));
                }
            }
            "function_declaration" | "protocol_function_declaration" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    let kind = if in_type { SymbolKind::Method } else { SymbolKind::Function };
                    emit(out, node_text(nn, src), scope, kind, child, Some(nn));
                }
            }
            "property_declaration" => {
                if let Some(id) = child.child_by_field_name("name").and_then(|n| first_kind(n, "simple_identifier")) {
                    emit(out, node_text(id, src), scope, SymbolKind::Field, child, Some(id));
                }
            }
            _ => walk_swift(child, src, scope, in_type, out),
        }
    }
}

fn push_named_swift(node: Node, src: &[u8], scope: &mut Vec<String>, kind: SymbolKind, out: &mut Vec<Extracted>) {
    if let Some(nn) = node.child_by_field_name("name") {
        let name = node_text(nn, src).to_string();
        emit(out, &name, scope, kind, node, Some(nn));
        scope.push(name);
        walk_swift(node, src, scope, true, out);
        scope.pop();
    } else {
        walk_swift(node, src, scope, false, out);
    }
}

/// First descendant (or self) of the given node kind.
fn first_kind<'n>(node: Node<'n>, kind: &str) -> Option<Node<'n>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = first_kind(child, kind) {
            return Some(found);
        }
    }
    None
}

fn collect_calls_swift(node: Node, src: &[u8], out: &mut Vec<CallSite>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "call_expression" {
            let callee = child.named_child(0).and_then(|f| match f.kind() {
                "simple_identifier" => Some(node_text(f, src).to_string()),
                "navigation_expression" => first_kind(f, "navigation_suffix")
                    .and_then(|s| first_kind(s, "simple_identifier"))
                    .map(|n| node_text(n, src).to_string()),
                _ => None,
            });
            if let Some(name) = callee {
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
        collect_calls_swift(child, src, out);
    }
}

// ---- TypeScript ------------------------------------------------------------

fn walk_ts(node: Node, src: &[u8], scope: &mut Vec<String>, out: &mut Vec<Extracted>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_declaration" | "generator_function_declaration" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    emit(out, node_text(nn, src), scope, SymbolKind::Function, child, Some(nn));
                }
                walk_ts(child, src, scope, out);
            }
            "class_declaration" | "abstract_class_declaration" => push_named_ts(child, src, scope, SymbolKind::Class, out),
            "interface_declaration" => push_named_ts(child, src, scope, SymbolKind::Interface, out),
            "internal_module" | "module" => push_named_ts(child, src, scope, SymbolKind::Module, out),
            "enum_declaration" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    let name = node_text(nn, src).to_string();
                    emit(out, &name, scope, SymbolKind::Enum, child, Some(nn));
                    scope.push(name);
                    if let Some(body) = child.child_by_field_name("body") {
                        let mut bc = body.walk();
                        for m in body.named_children(&mut bc) {
                            let mn = match m.kind() {
                                "property_identifier" => Some(m),
                                "enum_assignment" => m.child_by_field_name("name"),
                                _ => None,
                            };
                            if let Some(mn) = mn {
                                emit(out, node_text(mn, src), scope, SymbolKind::Variant, m, Some(mn));
                            }
                        }
                    }
                    scope.pop();
                }
            }
            "type_alias_declaration" => emit_named(child, src, scope, SymbolKind::TypeAlias, out),
            "method_definition" | "method_signature" | "abstract_method_signature" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    emit(out, node_text(nn, src), scope, SymbolKind::Method, child, Some(nn));
                }
                walk_ts(child, src, scope, out);
            }
            "public_field_definition" | "property_signature" => emit_named(child, src, scope, SymbolKind::Field, out),
            "variable_declarator" => {
                // Only fn/arrow-valued declarators become symbols (avoids local-variable noise).
                if let (Some(nn), Some(val)) = (child.child_by_field_name("name"), child.child_by_field_name("value")) {
                    if matches!(val.kind(), "arrow_function" | "function" | "function_expression") && nn.kind() == "identifier" {
                        emit(out, node_text(nn, src), scope, SymbolKind::Function, child, Some(nn));
                    }
                }
                walk_ts(child, src, scope, out);
            }
            _ => walk_ts(child, src, scope, out),
        }
    }
}

fn push_named_ts(node: Node, src: &[u8], scope: &mut Vec<String>, kind: SymbolKind, out: &mut Vec<Extracted>) {
    if let Some(nn) = node.child_by_field_name("name") {
        let name = node_text(nn, src).to_string();
        emit(out, &name, scope, kind, node, Some(nn));
        scope.push(name);
        walk_ts(node, src, scope, out);
        scope.pop();
    } else {
        walk_ts(node, src, scope, out);
    }
}

fn collect_calls_ts(node: Node, src: &[u8], out: &mut Vec<CallSite>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(name) = child.child_by_field_name("function").and_then(|f| callee_name_ts(f, src)) {
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
        collect_calls_ts(child, src, out);
    }
}

fn callee_name_ts(func: Node, src: &[u8]) -> Option<String> {
    match func.kind() {
        "identifier" => Some(node_text(func, src).to_string()),
        "member_expression" => func.child_by_field_name("property").map(|n| node_text(n, src).to_string()),
        _ => None,
    }
}

// ---- Python ----------------------------------------------------------------

fn walk_py(node: Node, src: &[u8], scope: &mut Vec<String>, in_class: bool, out: &mut Vec<Extracted>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    let kind = if in_class { SymbolKind::Method } else { SymbolKind::Function };
                    emit(out, node_text(nn, src), scope, kind, child, Some(nn));
                }
                walk_py(child, src, scope, false, out); // nested defs are plain functions
            }
            "class_definition" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    let name = node_text(nn, src).to_string();
                    emit(out, &name, scope, SymbolKind::Class, child, Some(nn));
                    scope.push(name);
                    walk_py(child, src, scope, true, out);
                    scope.pop();
                } else {
                    walk_py(child, src, scope, in_class, out);
                }
            }
            _ => walk_py(child, src, scope, in_class, out),
        }
    }
}

fn collect_calls_py(node: Node, src: &[u8], out: &mut Vec<CallSite>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(name) = child.child_by_field_name("function").and_then(|f| callee_name_py(f, src)) {
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
        collect_calls_py(child, src, out);
    }
}

fn callee_name_py(func: Node, src: &[u8]) -> Option<String> {
    match func.kind() {
        "identifier" => Some(node_text(func, src).to_string()),
        "attribute" => func.child_by_field_name("attribute").map(|n| node_text(n, src).to_string()),
        _ => None,
    }
}

// ---- Go --------------------------------------------------------------------

fn walk_go(node: Node, src: &[u8], scope: &mut Vec<String>, out: &mut Vec<Extracted>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    emit(out, node_text(nn, src), scope, SymbolKind::Function, child, Some(nn));
                }
            }
            "method_declaration" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    if let Some(recv) = recv_type_go(child, src) {
                        scope.push(recv);
                        emit(out, node_text(nn, src), scope, SymbolKind::Method, child, Some(nn));
                        scope.pop();
                    } else {
                        emit(out, node_text(nn, src), scope, SymbolKind::Method, child, Some(nn));
                    }
                }
            }
            "type_declaration" => {
                let mut tc = child.walk();
                for spec in child.named_children(&mut tc) {
                    if spec.kind() == "type_spec" {
                        if let Some(nn) = spec.child_by_field_name("name") {
                            let kind = match spec.child_by_field_name("type").map(|t| t.kind()) {
                                Some("struct_type") => SymbolKind::Struct,
                                Some("interface_type") => SymbolKind::Interface,
                                _ => SymbolKind::TypeAlias,
                            };
                            emit(out, node_text(nn, src), scope, kind, spec, Some(nn));
                        }
                    }
                }
            }
            "const_declaration" => {
                let mut cc = child.walk();
                for spec in child.named_children(&mut cc) {
                    if spec.kind() == "const_spec" {
                        let mut sc = spec.walk();
                        for id in spec.named_children(&mut sc) {
                            if id.kind() == "identifier" {
                                emit(out, node_text(id, src), scope, SymbolKind::Const, id, Some(id));
                            }
                        }
                    }
                }
            }
            _ => walk_go(child, src, scope, out),
        }
    }
}

fn recv_type_go(method: Node, src: &[u8]) -> Option<String> {
    let recv = method.child_by_field_name("receiver")?;
    let mut c = recv.walk();
    for p in recv.named_children(&mut c) {
        if p.kind() == "parameter_declaration" {
            if let Some(ty) = p.child_by_field_name("type") {
                return Some(node_text(ty, src).trim_start_matches('*').to_string());
            }
        }
    }
    None
}

fn collect_calls_go(node: Node, src: &[u8], out: &mut Vec<CallSite>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(name) = child.child_by_field_name("function").and_then(|f| callee_name_go(f, src)) {
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
        collect_calls_go(child, src, out);
    }
}

fn callee_name_go(func: Node, src: &[u8]) -> Option<String> {
    match func.kind() {
        "identifier" => Some(node_text(func, src).to_string()),
        "selector_expression" => func.child_by_field_name("field").map(|n| node_text(n, src).to_string()),
        _ => None,
    }
}

// ---- Java ------------------------------------------------------------------

fn walk_java(node: Node, src: &[u8], scope: &mut Vec<String>, out: &mut Vec<Extracted>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "record_declaration" => push_named_java(child, src, scope, SymbolKind::Class, out),
            "interface_declaration" | "annotation_type_declaration" => push_named_java(child, src, scope, SymbolKind::Interface, out),
            "enum_declaration" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    let name = node_text(nn, src).to_string();
                    emit(out, &name, scope, SymbolKind::Enum, child, Some(nn));
                    scope.push(name);
                    let mut bc = child.walk();
                    for d in child.named_children(&mut bc) {
                        if d.kind() == "enum_body" {
                            let mut ec = d.walk();
                            for c in d.named_children(&mut ec) {
                                if c.kind() == "enum_constant" {
                                    if let Some(en) = c.child_by_field_name("name") {
                                        emit(out, node_text(en, src), scope, SymbolKind::Variant, c, Some(en));
                                    }
                                }
                            }
                        }
                    }
                    walk_java(child, src, scope, out);
                    scope.pop();
                }
            }
            "method_declaration" | "constructor_declaration" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    emit(out, node_text(nn, src), scope, SymbolKind::Method, child, Some(nn));
                }
            }
            "field_declaration" => {
                let mut fc = child.walk();
                for d in child.named_children(&mut fc) {
                    if d.kind() == "variable_declarator" {
                        if let Some(nn) = d.child_by_field_name("name") {
                            emit(out, node_text(nn, src), scope, SymbolKind::Field, d, Some(nn));
                        }
                    }
                }
            }
            _ => walk_java(child, src, scope, out),
        }
    }
}

fn push_named_java(node: Node, src: &[u8], scope: &mut Vec<String>, kind: SymbolKind, out: &mut Vec<Extracted>) {
    if let Some(nn) = node.child_by_field_name("name") {
        let name = node_text(nn, src).to_string();
        emit(out, &name, scope, kind, node, Some(nn));
        scope.push(name);
        walk_java(node, src, scope, out);
        scope.pop();
    } else {
        walk_java(node, src, scope, out);
    }
}

// ---- C ---------------------------------------------------------------------

/// Follow the `declarator` field chain (pointer/function/array/parenthesized) to the name.
fn c_declarator_name<'a, 'n>(n: Node<'n>, src: &'a [u8]) -> Option<(&'a str, Node<'n>)> {
    match n.kind() {
        "identifier" | "field_identifier" | "type_identifier" => Some((node_text(n, src), n)),
        _ => n.child_by_field_name("declarator").and_then(|d| c_declarator_name(d, src)),
    }
}

fn walk_c(node: Node, src: &[u8], scope: &mut Vec<String>, in_type: bool, out: &mut Vec<Extracted>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(d) = child.child_by_field_name("declarator") {
                    if let Some((name, nn)) = c_declarator_name(d, src) {
                        let kind = if in_type { SymbolKind::Method } else { SymbolKind::Function };
                        emit(out, name, scope, kind, child, Some(nn));
                    }
                }
            }
            "struct_specifier" | "union_specifier" | "class_specifier" => {
                if let (Some(nn), Some(_)) = (child.child_by_field_name("name"), child.child_by_field_name("body")) {
                    let name = node_text(nn, src).to_string();
                    let kind = if child.kind() == "class_specifier" { SymbolKind::Class } else { SymbolKind::Struct };
                    emit(out, &name, scope, kind, child, Some(nn));
                    scope.push(name);
                    walk_c(child, src, scope, true, out);
                    scope.pop();
                }
            }
            "namespace_definition" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    let name = node_text(nn, src).to_string();
                    emit(out, &name, scope, SymbolKind::Module, child, Some(nn));
                    scope.push(name);
                    walk_c(child, src, scope, false, out);
                    scope.pop();
                } else {
                    walk_c(child, src, scope, false, out);
                }
            }
            "field_declaration" => {
                if let Some(d) = child.child_by_field_name("declarator") {
                    if let Some((name, nn)) = c_declarator_name(d, src) {
                        let kind = if c_has_function_declarator(d) { SymbolKind::Method } else { SymbolKind::Field };
                        emit(out, name, scope, kind, child, Some(nn));
                    }
                }
            }
            "enum_specifier" => {
                if let Some(body) = child.child_by_field_name("body") {
                    let pushed = if let Some(nn) = child.child_by_field_name("name") {
                        let name = node_text(nn, src).to_string();
                        emit(out, &name, scope, SymbolKind::Enum, child, Some(nn));
                        scope.push(name);
                        true
                    } else {
                        false
                    };
                    let mut bc = body.walk();
                    for e in body.named_children(&mut bc) {
                        if e.kind() == "enumerator" {
                            if let Some(en) = e.child_by_field_name("name").or_else(|| e.named_child(0)) {
                                emit(out, node_text(en, src), scope, SymbolKind::Variant, e, Some(en));
                            }
                        }
                    }
                    if pushed {
                        scope.pop();
                    }
                }
            }
            "type_definition" => {
                if let Some(d) = child.child_by_field_name("declarator") {
                    if let Some((name, nn)) = c_declarator_name(d, src) {
                        emit(out, name, scope, SymbolKind::TypeAlias, child, Some(nn));
                    }
                }
                walk_c(child, src, scope, in_type, out);
            }
            "preproc_function_def" | "preproc_def" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    emit(out, node_text(nn, src), scope, SymbolKind::Macro, child, Some(nn));
                }
            }
            _ => walk_c(child, src, scope, in_type, out),
        }
    }
}

fn c_has_function_declarator(n: Node) -> bool {
    if n.kind() == "function_declarator" {
        return true;
    }
    match n.child_by_field_name("declarator") {
        Some(d) => c_has_function_declarator(d),
        None => false,
    }
}

fn collect_calls_c(node: Node, src: &[u8], out: &mut Vec<CallSite>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(func) = child.child_by_field_name("function") {
                let name = match func.kind() {
                    "identifier" => Some(node_text(func, src).to_string()),
                    "field_expression" => func.child_by_field_name("field").map(|n| node_text(n, src).to_string()),
                    _ => None,
                };
                if let Some(name) = name {
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
        }
        collect_calls_c(child, src, out);
    }
}

// ---- PHP -------------------------------------------------------------------

fn walk_php(node: Node, src: &[u8], scope: &mut Vec<String>, out: &mut Vec<Extracted>) {
    let mut cursor = node.walk();
    let mut ns_pushes = 0usize;
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "namespace_definition" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    let name = node_text(nn, src).to_string();
                    emit(out, &name, scope, SymbolKind::Module, child, Some(nn));
                    if child.child_by_field_name("body").is_some() {
                        // braced `namespace Foo { ... }`
                        scope.push(name);
                        walk_php(child, src, scope, out);
                        scope.pop();
                    } else {
                        // bodyless `namespace Foo;` applies to the following siblings
                        scope.push(name);
                        ns_pushes += 1;
                    }
                }
            }
            "class_declaration" => push_named_php(child, src, scope, SymbolKind::Class, out),
            "interface_declaration" => push_named_php(child, src, scope, SymbolKind::Interface, out),
            "trait_declaration" => push_named_php(child, src, scope, SymbolKind::Trait, out),
            "enum_declaration" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    let name = node_text(nn, src).to_string();
                    emit(out, &name, scope, SymbolKind::Enum, child, Some(nn));
                    scope.push(name);
                    walk_php(child, src, scope, out);
                    scope.pop();
                }
            }
            "enum_case" => emit_named(child, src, scope, SymbolKind::Variant, out),
            "function_definition" => emit_named(child, src, scope, SymbolKind::Function, out),
            "method_declaration" => emit_named(child, src, scope, SymbolKind::Method, out),
            "property_declaration" => {
                let mut pc = child.walk();
                for el in child.named_children(&mut pc) {
                    if el.kind() == "property_element" {
                        if let Some(vn) = el.named_child(0) {
                            let name = node_text(vn, src).trim_start_matches('$');
                            emit(out, name, scope, SymbolKind::Field, el, Some(vn));
                        }
                    }
                }
            }
            "const_declaration" => {
                let mut cc = child.walk();
                for el in child.named_children(&mut cc) {
                    if el.kind() == "const_element" {
                        if let Some(nn) = el.named_child(0) {
                            emit(out, node_text(nn, src), scope, SymbolKind::Const, el, Some(nn));
                        }
                    }
                }
            }
            _ => walk_php(child, src, scope, out),
        }
    }
    for _ in 0..ns_pushes {
        scope.pop();
    }
}

fn push_named_php(node: Node, src: &[u8], scope: &mut Vec<String>, kind: SymbolKind, out: &mut Vec<Extracted>) {
    if let Some(nn) = node.child_by_field_name("name") {
        let name = node_text(nn, src).to_string();
        emit(out, &name, scope, kind, node, Some(nn));
        scope.push(name);
        walk_php(node, src, scope, out);
        scope.pop();
    } else {
        walk_php(node, src, scope, out);
    }
}

fn collect_calls_php(node: Node, src: &[u8], out: &mut Vec<CallSite>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let name = match child.kind() {
            "function_call_expression" => child.child_by_field_name("function").map(|n| {
                node_text(n, src).rsplit(['\\', ':']).next().unwrap_or("").to_string()
            }),
            "member_call_expression" | "nullsafe_member_call_expression" | "scoped_call_expression" => {
                child.child_by_field_name("name").map(|n| node_text(n, src).to_string())
            }
            _ => None,
        };
        if let Some(name) = name {
            if !name.is_empty() {
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
        collect_calls_php(child, src, out);
    }
}

// ---- C# --------------------------------------------------------------------

fn walk_csharp(node: Node, src: &[u8], scope: &mut Vec<String>, out: &mut Vec<Extracted>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "record_declaration" => push_named_csharp(child, src, scope, SymbolKind::Class, out),
            "interface_declaration" => push_named_csharp(child, src, scope, SymbolKind::Interface, out),
            "struct_declaration" => push_named_csharp(child, src, scope, SymbolKind::Struct, out),
            "namespace_declaration" | "file_scoped_namespace_declaration" => {
                push_named_csharp(child, src, scope, SymbolKind::Module, out)
            }
            "enum_declaration" => {
                if let Some(nn) = child.child_by_field_name("name") {
                    let name = node_text(nn, src).to_string();
                    emit(out, &name, scope, SymbolKind::Enum, child, Some(nn));
                    scope.push(name);
                    walk_csharp(child, src, scope, out);
                    scope.pop();
                }
            }
            "enum_member_declaration" => emit_named(child, src, scope, SymbolKind::Variant, out),
            "method_declaration" | "constructor_declaration" | "local_function_statement" => {
                emit_named(child, src, scope, SymbolKind::Method, out)
            }
            "property_declaration" => emit_named(child, src, scope, SymbolKind::Field, out),
            "field_declaration" => {
                let mut fc = child.walk();
                for d in child.named_children(&mut fc) {
                    if d.kind() == "variable_declaration" {
                        let mut vc = d.walk();
                        for v in d.named_children(&mut vc) {
                            if v.kind() == "variable_declarator" {
                                if let Some(nn) = v.child_by_field_name("name").or_else(|| v.named_child(0)) {
                                    emit(out, node_text(nn, src), scope, SymbolKind::Field, v, Some(nn));
                                }
                            }
                        }
                    }
                }
            }
            _ => walk_csharp(child, src, scope, out),
        }
    }
}

fn push_named_csharp(node: Node, src: &[u8], scope: &mut Vec<String>, kind: SymbolKind, out: &mut Vec<Extracted>) {
    if let Some(nn) = node.child_by_field_name("name") {
        let name = node_text(nn, src).to_string();
        emit(out, &name, scope, kind, node, Some(nn));
        scope.push(name);
        walk_csharp(node, src, scope, out);
        scope.pop();
    } else {
        walk_csharp(node, src, scope, out);
    }
}

fn collect_calls_csharp(node: Node, src: &[u8], out: &mut Vec<CallSite>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "invocation_expression" {
            if let Some(func) = child.child_by_field_name("function") {
                let name = match func.kind() {
                    "member_access_expression" => func.child_by_field_name("name").map(|n| node_text(n, src).to_string()),
                    "identifier" => Some(node_text(func, src).to_string()),
                    _ => None,
                };
                if let Some(name) = name {
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
        }
        collect_calls_csharp(child, src, out);
    }
}

fn collect_calls_java(node: Node, src: &[u8], out: &mut Vec<CallSite>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "method_invocation" {
            if let Some(nn) = child.child_by_field_name("name") {
                let sp = child.start_position();
                let ep = child.end_position();
                out.push(CallSite {
                    name: node_text(nn, src).to_string(),
                    range: Range {
                        start_line: sp.row as u32 + 1,
                        start_col: sp.column as u32,
                        end_line: ep.row as u32 + 1,
                        end_col: ep.column as u32,
                    },
                });
            }
        }
        collect_calls_java(child, src, out);
    }
}
