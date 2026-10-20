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

    /// Release builds abort on panic, and the window used to render every
    /// streamed prefix of a diagram, so half-written sources must not panic.
    #[test]
    fn partial_sources_do_not_panic() {
        let full = "graph TD\n  A[Start] --> B{Is it?}\n  B -->|Yes| C[OK]\n  中文 --> 结束\nsequenceDiagram\n  Alice->>John: Hi";
        let chars: Vec<char> = full.chars().collect();
        for end in 0..=chars.len() {
            let source: String = chars[..end].iter().collect();
            let _ = render_mermaid(&source, end % 2 == 0);
        }
    }

    #[test]
    fn renders_a_flowchart() {
        let svg = render_mermaid("graph TD\n  A-->B", false).expect("svg");
        assert!(svg.contains("<svg"));
        assert!(svg.contains("A") || svg.contains("B"));
    }
}
