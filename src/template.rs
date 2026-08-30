// code from string_template 0.2
use regex::Regex;
use serde::Serialize;

#[derive(Clone)]
pub struct Template {
    src: String,
    matches: Vec<(usize, usize)>,
}

impl Template {
    pub fn new(template: &str) -> Self {
        let regex = Regex::new(r"\{\{([^}]*)\}\}").unwrap();

        Template {
            src: template.to_owned(),
            matches: regex
                .find_iter(template)
                .map(|m| (m.start(), m.end()))
                .collect(),
        }
    }

    /// # Examples
    ///
    /// A template containing `Hi, {{name}}` renders to `Hi, Michael` when
    /// `name` is set to `Michael`.
    pub fn render<T: Serialize>(&self, vals: T) -> String {
        self.render_named(vals)
    }

    ///
    /// See render() for examples.
    ///
    pub fn render_named<T: Serialize>(&self, vals: T) -> String {
        let mut parts: Vec<String> = vec![];
        let template_str = &self.src;

        // get index of first arg match or return a copy of the template if no args matched
        let first = match self.matches.first() {
            Some((start, _)) => *start,
            _ => return template_str.clone(),
        };

        // copy from template start to first arg
        if first > 0 {
            parts.push(template_str[0..first].to_string())
        }

        // keeps the index of the previous argument end
        let mut prev_end: Option<usize> = None;

        let vals = serde_json::to_value(&vals).unwrap();
        for (start, end) in self.matches.iter() {
            // copy from previous argument end till current argument start
            if let Some(last_end) = prev_end {
                parts.push(template_str[last_end..*start].to_string())
            }

            // argument name with braces
            let arg = &template_str[*start..*end];
            // just the argument name
            let arg_name = &arg[2..arg.len() - 2];

            match vals.get(arg_name) {
                Some(s) => {
                    if s.is_string() {
                        parts.push(s.as_str().unwrap().to_string());
                    } else {
                        let s = s.to_string();
                        parts.push(s);
                    }
                }
                _ => parts.push(arg.to_string()),
            }

            prev_end = Some(*end);
        }

        let template_len = template_str.len();
        // if last arg end index isn't the end of the string then copy
        // from last arg end till end of template string
        if let Some(last_pos) = prev_end {
            if last_pos < template_len {
                parts.push(template_str[last_pos..template_len].to_string())
            }
        }

        parts.join("")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::Template;

    #[test]
    fn renders_named_values() {
        let template = Template::new("Hi, my name is {{name}} and I'm a {{lang}} developer.");
        let args = HashMap::from([("name", "Michael"), ("lang", "Rust")]);

        assert_eq!(
            template.render(&args),
            "Hi, my name is Michael and I'm a Rust developer."
        );
    }
}
