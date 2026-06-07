use std::collections::HashMap;
use std::fs;

use crate::debug_log;

#[derive(Debug, Clone)]
pub struct ButtonLegend {
    pub key: String,
    pub display: String,
}

#[derive(Debug, Clone)]
pub struct ButtonGroup {
    pub label: Option<String>,
    pub buttons: Vec<ButtonLegend>,
}

#[derive(Debug, Clone)]
pub struct LayoutInfo {
    pub label: String,
    pub groups: Vec<ButtonGroup>,
}

pub struct ConfigParser;

impl ConfigParser {
    /// Parse keyd config and extract layout information
    pub fn parse(config_path: &str) -> Result<HashMap<String, LayoutInfo>, String> {
        let content = fs::read_to_string(config_path)
            .map_err(|e| format!("Failed to read config: {}", e))?;
        Self::parse_from_str(&content)
    }

    pub fn parse_from_str(content: &str) -> Result<HashMap<String, LayoutInfo>, String> {
        let mut layouts = HashMap::new();
        let lines: Vec<&str> = content.lines().collect();

        // Pass 1: Collect group definitions specifically from the [ids] section
        let mut ids_groups = Vec::new();
        let mut in_ids = false;
        for line in &lines {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_ids = trimmed == "[ids]";
                continue;
            }
            if in_ids {
                if let Some(group) = Self::parse_group_comment(line) {
                    ids_groups.push(group);
                }
            }
        }

        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];

            // Check for layout section header [name:layout]
            if let Some(name) = Self::parse_layout_header(line) {
                let label = if i > 0 {
                    Self::parse_label_comment(lines[i - 1])
                        .unwrap_or_else(|| Self::derive_label(&name))
                } else {
                    Self::derive_label(&name)
                };

                let (local_groups, buttons) = Self::extract_layout_content(&lines, &mut i);

                // Combine groups: [ids] groups + local groups
                let mut combined_groups = ids_groups.clone();
                combined_groups.extend(local_groups);

                layouts.insert(
                    name,
                    LayoutInfo {
                        label,
                        groups: Self::organize_groups(combined_groups, buttons),
                    },
                );
            } else {
                i += 1;
            }
        }

        Ok(layouts)
    }

    fn parse_layout_header(line: &str) -> Option<String> {
        let content = line.strip_prefix('[')?.strip_suffix(']')?;
        let name = content.strip_suffix(":layout")?;
        Some(name.to_string())
    }

    fn parse_label_comment(line: &str) -> Option<String> {
        let trimmed = line.trim();
        if let Some(content) = trimmed.strip_prefix("# label:") {
            return Some(content.trim().to_string());
        }
        None
    }

    fn parse_group_comment(line: &str) -> Option<(String, Vec<String>)> {
        let trimmed = line.trim();
        let content = trimmed.strip_prefix("# group:")?;
        let parts: Vec<&str> = content.splitn(2, '=').collect();
        if parts.len() == 2 {
            let label = parts[0].trim().trim_matches('"').to_string();
            let keys = parts[1]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            return Some((label, keys));
        }
        None
    }

    fn derive_label(identifier: &str) -> String {
        identifier
            .split('_')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => {
                        first.to_uppercase().collect::<String>() + chars.as_str()
                    }
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn extract_layout_content(lines: &[&str], current_idx: &mut usize) -> (Vec<(String, Vec<String>)>, Vec<ButtonLegend>) {
        let mut groups = Vec::new();
        let mut buttons = Vec::new();
        *current_idx += 1;

        while *current_idx < lines.len() {
            let line = lines[*current_idx];
            let trimmed = line.trim();

            // Stop at next section or EOF
            if trimmed.starts_with('[') {
                break;
            }

            if let Some(group) = Self::parse_group_comment(line) {
                groups.push(group);
            }

            // Parse binding line: key = action
            if let Some((key, action)) = Self::parse_binding(line) {
                // Skip noop and setlayout bindings
                if action.contains("setlayout") || action == "noop" {
                    *current_idx += 1;
                    continue;
                }

                // Check for label comment on the immediately preceding line
                let label_override = if *current_idx > 0 {
                    Self::parse_label_comment(lines[*current_idx - 1])
                } else {
                    None
                };

                if let Some(label) = label_override {
                    let mut chars = label.chars();
                    let display = chars.next().unwrap_or(' ').to_string();
                    if chars.next().is_some() {
                        debug_log!("[🐛DEBUG] Button label '{}' has extra characters, using first char '{}'", label, display);
                    }
                    buttons.push(ButtonLegend { key, display });
                } else if let Some(display) = Self::extract_display_char(&action) {
                    buttons.push(ButtonLegend { key, display });
                }
            }

            *current_idx += 1;
        }

        (groups, buttons)
    }

    fn organize_groups(group_defs: Vec<(String, Vec<String>)>, mut buttons: Vec<ButtonLegend>) -> Vec<ButtonGroup> {
        let mut result_groups = Vec::new();

        for (label, keys) in group_defs {
            let mut group_buttons = Vec::new();
            for key in keys {
                if let Some(pos) = buttons.iter().position(|b| b.key == key) {
                    group_buttons.push(buttons.remove(pos));
                }
            }
            if !group_buttons.is_empty() {
                result_groups.push(ButtonGroup {
                    label: Some(label),
                    buttons: group_buttons,
                });
            }
        }

        // Catch-all anonymous group - should be first (Rubric 7.7)
        if !buttons.is_empty() {
            result_groups.insert(0, ButtonGroup {
                label: None,
                buttons,
            });
        }

        result_groups
    }

    fn parse_binding(line: &str) -> Option<(String, String)> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }

        let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
        if parts.len() == 2 {
            let key = parts[0].trim().to_string();
            let action = parts[1].trim().to_string();
            return Some((key, action));
        }
        None
    }

    fn extract_display_char(action: &str) -> Option<String> {
        // Try macro2(timeout, repeat_timeout, macro_expr) — check before macro()
        if let Some(start) = action.find("macro2(") {
            let end = action.rfind(')')?;
            if start < end {
                let inner = &action[start + 7..end];
                let parts: Vec<&str> = inner.splitn(3, ',').collect();
                if parts.len() == 3 {
                    let macro_expr = parts[2].trim();
                    return Self::extract_display_char(macro_expr);
                }
            }
        }
        // Try macro(char)
        if let Some(start) = action.find("macro(") {
            let end = action.rfind(')')?;
            if start < end {
                return Some(action[start + 6..end].to_string());
            }
        }
        // Try command(... rologlyphex type char) or command(... xdotool type char)
        for prefix in ["rologlyphex type ", "xdotool type "] {
            if let Some(start) = action.find(prefix) {
                let content = &action[start + prefix.len()..];
                let content = content.trim_end_matches(')').trim();
                if !content.is_empty() {
                    return Some(content.to_string());
                }
            }
        }
        // Bare unicode character (single char, not a multi-char key name)
        let trimmed = action.trim();
        let mut chars = trimmed.chars();
        if let Some(ch) = chars.next() {
            if chars.next().is_none() {
                return Some(ch.to_string());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_label() {
        assert_eq!(ConfigParser::derive_label("my_arrows"), "My Arrows");
        assert_eq!(ConfigParser::derive_label("fire_stars"), "Fire Stars");
        assert_eq!(ConfigParser::derive_label("math"), "Math");
    }

    #[test]
    fn test_parse_label_comment() {
        let comment = "# label: Fire & Stars 🔥";
        assert_eq!(
            ConfigParser::parse_label_comment(comment),
            Some("Fire & Stars 🔥".to_string())
        );

        let non_label = "# This is a regular comment";
        assert_eq!(ConfigParser::parse_label_comment(non_label), None);
    }

    #[test]
    fn test_parse_layout_header() {
        assert_eq!(
            ConfigParser::parse_layout_header("[arrows:layout]"),
            Some("arrows".to_string())
        );

        assert_eq!(
            ConfigParser::parse_layout_header("[main]"),
            None
        );

        // Must not match layout-like suffixes
        assert_eq!(
            ConfigParser::parse_layout_header("[name:layout-extra]"),
            None
        );
    }

    #[test]
    fn test_extract_display_char() {
        assert_eq!(
            ConfigParser::extract_display_char("macro(→)"),
            Some("→".to_string())
        );

        assert_eq!(
            ConfigParser::extract_display_char("macro(🔥)"),
            Some("🔥".to_string())
        );

        assert_eq!(
            ConfigParser::extract_display_char("command(x11-launch xdotool type →)"),
            Some("→".to_string())
        );

        assert_eq!(
            ConfigParser::extract_display_char("command(x11-launch xdotool type 🔥)"),
            Some("🔥".to_string())
        );

        assert_eq!(
            ConfigParser::extract_display_char("command(x11-launch rologlyphex type →)"),
            Some("→".to_string())
        );

        assert_eq!(
            ConfigParser::extract_display_char("command(x11-launch rologlyphex type 🔥)"),
            Some("🔥".to_string())
        );

        assert_eq!(
            ConfigParser::extract_display_char("setlayout(next)"),
            None
        );
    }

    #[test]
    fn test_extract_bare_unicode() {
        assert_eq!(
            ConfigParser::extract_display_char("→"),
            Some("→".to_string())
        );
        assert_eq!(
            ConfigParser::extract_display_char("🔥"),
            Some("🔥".to_string())
        );
        assert_eq!(
            ConfigParser::extract_display_char("!"),
            Some("!".to_string())
        );
        // Multi-char key names should not match
        assert_eq!(
            ConfigParser::extract_display_char("left"),
            None
        );
        assert_eq!(
            ConfigParser::extract_display_char("noop"),
            None
        );
    }

    #[test]
    fn test_extract_macro2() {
        assert_eq!(
            ConfigParser::extract_display_char("macro2(400, 50, →)"),
            Some("→".to_string())
        );
        assert_eq!(
            ConfigParser::extract_display_char("macro2(120, 80, macro(Hello space World))"),
            Some("Hello space World".to_string())
        );
        assert_eq!(
            ConfigParser::extract_display_char("macro2(400, 50, 🔥)"),
            Some("🔥".to_string())
        );
    }

    #[test]
    fn test_parse_group_comment() {
        let comment = "# group: \"G1-6\" = f19, f20, f21, f22, f23, f24";
        let (label, keys) = ConfigParser::parse_group_comment(comment).unwrap();
        assert_eq!(label, "G1-6");
        assert_eq!(keys, vec!["f19", "f20", "f21", "f22", "f23", "f24"]);

        let comment_no_quotes = "# group: SinLoon = f13,f14,f15,f17";
        let (label, keys) = ConfigParser::parse_group_comment(comment_no_quotes).unwrap();
        assert_eq!(label, "SinLoon");
        assert_eq!(keys, vec!["f13", "f14", "f15", "f17"]);
    }

    #[test]
    fn test_button_grouping() {
        let config = "\
[test:layout]
# group: \"Arrows\" = f13, f14
f13 = macro(left)
f14 = macro(right)
f15 = macro(up)
";
        let lines: Vec<&str> = config.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            if ConfigParser::parse_layout_header(lines[i]).is_some() {
                break;
            }
            i += 1;
        }
        let (group_defs, buttons) = ConfigParser::extract_layout_content(&lines, &mut i);
        let groups = ConfigParser::organize_groups(group_defs, buttons);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].label, None);
        assert_eq!(groups[0].buttons.len(), 1);
        assert_eq!(groups[1].label, Some("Arrows".to_string()));
        assert_eq!(groups[1].buttons.len(), 2);
    }

    #[test]
    fn test_global_button_grouping() {
        let config = "\
[ids]
# group: \"Global Group\" = f13
[test:layout]
f13 = macro(left)
f14 = macro(right)
";
        let layouts = ConfigParser::parse_from_str(config).unwrap();
        let layout = layouts.get("test").unwrap();
        
        assert_eq!(layout.groups.len(), 2);
        assert_eq!(layout.groups[0].label, None);
        assert_eq!(layout.groups[0].buttons.len(), 1);
        assert_eq!(layout.groups[0].buttons[0].display, "right");
        assert_eq!(layout.groups[1].label, Some("Global Group".to_string()));
        assert_eq!(layout.groups[1].buttons.len(), 1);
        assert_eq!(layout.groups[1].buttons[0].display, "left");
    }

    #[test]
    fn test_group_outside_ids_ignored() {
        let config = "\
# group: \"Ignored\" = f13
[ids]
1111:2222
[test:layout]
f13 = macro(left)
";
        let layouts = ConfigParser::parse_from_str(config).unwrap();
        let layout = layouts.get("test").unwrap();
        
        // Should only have the anonymous group
        assert_eq!(layout.groups.len(), 1);
        assert_eq!(layout.groups[0].label, None);
    }

    #[test]
    fn test_anonymous_group_first() {
        let content = r#"
[test:layout]
# group: "Named" = f1
f1 = macro(A)
f2 = macro(B)
"#;
        let layout_map = ConfigParser::parse_from_str(content).unwrap();
        let layout = layout_map.get("test").unwrap();

        assert_eq!(layout.groups.len(), 2);
        // Anonymous group should be first (Rubric 7.7)
        assert_eq!(layout.groups[0].label, None);
        assert_eq!(layout.groups[0].buttons[0].key, "f2");

        // Named group should be second
        assert_eq!(layout.groups[1].label, Some("Named".to_string()));
        assert_eq!(layout.groups[1].buttons[0].key, "f1");
    }

    #[test]
    fn test_button_label_override_single_char() {
        let config = "\
[ids]
1189:8890

[test:layout]
# label: ➡
f13 = macro(→)
f14 = macro(⇒)
";
        let lines: Vec<&str> = config.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            if ConfigParser::parse_layout_header(lines[i]).is_some() {
                break;
            }
            i += 1;
        }
        let (_groups, buttons) = ConfigParser::extract_layout_content(&lines, &mut i);
        assert_eq!(buttons[0].display, "➡");
        assert_eq!(buttons[1].display, "⇒");
    }

    #[test]
    fn test_button_label_override_truncates_extra_chars() {
        let config = "\
[ids]
1189:8890

[test:layout]
# label: Right Arrow
f13 = macro(→)
";
        let lines: Vec<&str> = config.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            if ConfigParser::parse_layout_header(lines[i]).is_some() {
                break;
            }
            i += 1;
        }
        let (_groups, buttons) = ConfigParser::extract_layout_content(&lines, &mut i);
        // Takes only first char 'R', warns about extra
        assert_eq!(buttons[0].display, "R");
    }

    #[test]
    fn test_button_label_non_adjacent_ignored() {
        let config = "\
[ids]
1189:8890

[test:layout]
# label: Should Be Ignored

f13 = macro(→)
";
        let lines: Vec<&str> = config.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            if ConfigParser::parse_layout_header(lines[i]).is_some() {
                break;
            }
            i += 1;
        }
        let (_groups, buttons) = ConfigParser::extract_layout_content(&lines, &mut i);
        assert_eq!(buttons[0].display, "→");
    }
}
