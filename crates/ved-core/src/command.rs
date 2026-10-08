//! Pure command string normalization shared by conversion and rendering.
pub fn clean_command_string(raw: &str) -> String {
    let mut result_lines: Vec<String> = Vec::new();
    let mut current_line = String::new();

    for line in raw.lines() {
        let trimmed_end = line.trim_end();
        let is_continuation = trimmed_end.ends_with('\\') && !trimmed_end.ends_with(r"\\");

        if is_continuation {
            let stripped = trimmed_end
                .strip_suffix('\\')
                .unwrap_or(trimmed_end)
                .trim_end();
            if current_line.is_empty() {
                current_line.push_str(stripped);
            } else {
                current_line.push(' ');
                current_line.push_str(stripped.trim_start());
            }
        } else {
            if current_line.is_empty() {
                current_line.push_str(trimmed_end);
            } else {
                current_line.push(' ');
                current_line.push_str(trimmed_end.trim_start());
            }
            result_lines.push(current_line);
            current_line = String::new();
        }
    }

    if !current_line.is_empty() {
        result_lines.push(current_line);
    }

    for line in &mut result_lines {
        let t = line.trim_end();
        if t.ends_with('\\') && !t.ends_with(r"\\") {
            *line = t.strip_suffix('\\').unwrap_or(t).trim_end().to_string();
        }
        while line.contains("\\ ") {
            *line = line.replace("\\ ", " ");
        }
        *line = line.trim().to_string();
    }

    result_lines.retain(|l| !l.is_empty());
    result_lines.join("\n")
}
