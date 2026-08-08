//! `lbt alias list`: report every command alias in the tree.

use anyhow::Result;

use crate::cli::tree::Node;

/// Collect every node's canonical name + aliases, walking the whole tree.
fn collect(node: &Node, path: &mut Vec<String>, out: &mut Vec<(String, Vec<String>)>) {
    for child in node.children() {
        path.push(child.name.to_string());
        if !child.aliases.is_empty() {
            out.push((
                path.join(" "),
                child.aliases.iter().map(|a| a.to_string()).collect(),
            ));
        }
        collect(child, path, out);
        path.pop();
    }
}

pub fn list() -> Result<()> {
    let mut out = Vec::new();
    collect(&crate::cli::tree::ROOT, &mut Vec::new(), &mut out);
    let count = out.len();
    let mut rows = out;
    rows.sort();
    if rows.is_empty() {
        println!("no aliases declared");
        return Ok(());
    }
    let width = rows.iter().map(|(p, _)| p.len()).max().unwrap_or(0);
    for (path, aliases) in rows {
        println!("  {:<width$}  {}", path, aliases.join(", "));
    }
    println!("\n{count} commands have aliases");
    Ok(())
}
