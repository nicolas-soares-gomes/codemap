//! Subgraph export to text formats (DOT, Mermaid). SVG/HTML are out of MVP scope (SVG would
//! need an external layout engine, which conflicts with the never-install policy).

use crate::query::Subgraph;
use crate::types::Provenance;

/// Graphviz DOT. Edge color encodes provenance (resolved/scip greener, syntactic oranger).
pub fn to_dot(g: &Subgraph) -> String {
    let mut s = String::from("digraph codemap {\n  rankdir=LR;\n  node [shape=box, fontname=\"monospace\"];\n");
    for (id, name) in &g.nodes {
        let extra = if *id == g.root { ", style=filled, fillcolor=\"#e0e7ff\"" } else { "" };
        s.push_str(&format!("  n{id} [label=\"{}\"{extra}];\n", escape_dot(name)));
    }
    for (src, tgt, prov, _res) in &g.edges {
        s.push_str(&format!("  n{src} -> n{tgt} [color=\"{}\"];\n", edge_color(*prov)));
    }
    s.push_str("}\n");
    s
}

/// Mermaid flowchart.
pub fn to_mermaid(g: &Subgraph) -> String {
    let mut s = String::from("flowchart LR\n");
    for (id, name) in &g.nodes {
        s.push_str(&format!("  n{id}[\"{}\"]\n", escape_mermaid(name)));
    }
    for (src, tgt, _prov, _res) in &g.edges {
        s.push_str(&format!("  n{src} --> n{tgt}\n"));
    }
    s
}

fn edge_color(prov: Option<Provenance>) -> &'static str {
    match prov {
        Some(Provenance::Scip) | Some(Provenance::Lsp) => "#16a34a", // resolved
        Some(Provenance::StackGraphs) => "#2563eb",
        _ => "#ea580c", // tree_sitter / text (ambiguous)
    }
}

fn escape_dot(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn escape_mermaid(s: &str) -> String {
    s.replace('"', "&quot;")
}
