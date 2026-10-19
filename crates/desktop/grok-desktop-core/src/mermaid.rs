//! Render fenced Mermaid diagrams with the vendored `mermaid-to-svg` engine.

use mermaid_to_svg::{MermaidThemePreset, render_mermaid_to_svg};

pub fn render_mermaid(source: &str, dark: bool) -> Result<String, String> {
    let theme = if dark {
        MermaidThemePreset::Dark.to_theme()
    } else {
        MermaidThemePreset::Default.to_theme()
    };
    render_mermaid_to_svg(source, Some(&theme)).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_flowchart() {
        let svg = render_mermaid("graph TD\n  A-->B", false).expect("svg");
        assert!(svg.contains("<svg"));
        assert!(svg.contains("A") || svg.contains("B"));
    }
}
