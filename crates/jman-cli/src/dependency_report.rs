use std::collections::BTreeMap;

#[derive(Default)]
struct PathNode {
    children: BTreeMap<String, PathNode>,
}

impl PathNode {
    fn insert(&mut self, path: &[String]) {
        let Some((coordinate, remaining)) = path.split_first() else {
            return;
        };
        self.children
            .entry(coordinate.clone())
            .or_default()
            .insert(remaining);
    }

    fn append_lines(&self, indentation: &str, prefix: &str, output: &mut Vec<String>) {
        let child_count = self.children.len();
        for (index, (coordinate, child)) in self.children.iter().enumerate() {
            let last = index + 1 == child_count;
            let branch = if last { "└── " } else { "├── " };
            output.push(format!(
                "{indentation}{prefix}{branch}{}",
                terminal_text(coordinate)
            ));
            let child_prefix = format!("{prefix}{}", if last { "    " } else { "│   " });
            child.append_lines(indentation, &child_prefix, output);
        }
    }
}

pub fn dependency_path_tree(root: &str, paths: &[Vec<String>], indentation: &str) -> String {
    let mut tree = PathNode::default();
    for path in paths {
        tree.insert(path);
    }
    let mut lines = vec![format!("{indentation}{}", terminal_text(root))];
    tree.append_lines(indentation, "", &mut lines);
    lines.join("\n")
}

pub fn terminal_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_single_dependency_path_as_a_tree() {
        assert_eq!(
            dependency_path_tree(
                "com.example:app:1",
                &[vec![
                    "org.example:direct:1".to_owned(),
                    "org.example:target:1".to_owned(),
                ]],
                "",
            ),
            "com.example:app:1\n└── org.example:direct:1\n    └── org.example:target:1"
        );
    }

    #[test]
    fn merges_shared_prefixes_and_sorts_sibling_paths() {
        let paths = vec![
            vec![
                "org.example:z:1".to_owned(),
                "org.example:target:1".to_owned(),
            ],
            vec![
                "org.example:a:1".to_owned(),
                "org.example:c:1".to_owned(),
                "org.example:target:1".to_owned(),
            ],
            vec![
                "org.example:a:1".to_owned(),
                "org.example:b:1".to_owned(),
                "org.example:target:1".to_owned(),
            ],
        ];
        assert_eq!(
            dependency_path_tree("module app", &paths, "  "),
            "  module app\n  ├── org.example:a:1\n  │   ├── org.example:b:1\n  │   │   └── org.example:target:1\n  │   └── org.example:c:1\n  │       └── org.example:target:1\n  └── org.example:z:1\n      └── org.example:target:1"
        );
    }

    #[test]
    fn neutralizes_terminal_control_characters() {
        assert_eq!(terminal_text("safe\u{1b}[31m\ntext"), "safe [31m text");
    }
}
