use std::{borrow::Cow, collections::HashSet};

fn normalized(path: &str) -> Cow<'_, str> {
    if path.contains('\\') {
        Cow::Owned(path.replace('\\', "/"))
    } else {
        Cow::Borrowed(path)
    }
}

pub(crate) struct Selection<'a> {
    exact: HashSet<Cow<'a, str>>,
    descendants: Vec<String>,
}

impl<'a> Selection<'a> {
    pub(crate) fn new(names: &'a [String]) -> Self {
        let exact: HashSet<_> = names.iter().map(|name| normalized(name)).collect();
        let mut roots: Vec<_> = exact
            .iter()
            .map(|name| format!("{}/", name.trim_end_matches('/')))
            .collect();
        roots.sort_unstable();
        // Every retained prefix covers one disjoint interval in lexical order.
        // Nested and duplicate prefixes already belong to the previous interval.
        let mut descendants: Vec<String> = Vec::with_capacity(roots.len());
        for root in roots {
            if !descendants
                .last()
                .is_some_and(|previous| root.starts_with(previous))
            {
                descendants.push(root);
            }
        }
        Self { exact, descendants }
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        if self.exact.is_empty() {
            return true;
        }
        let name = normalized(name);
        if self.exact.contains(name.as_ref()) {
            return true;
        }
        let previous = self
            .descendants
            .partition_point(|root| root.as_str() <= name.as_ref());
        previous > 0 && name.starts_with(&self.descendants[previous - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_matches_original_exact_and_descendant_rules() {
        let paths = [
            "",
            "/",
            "a",
            "a/",
            "a//",
            "a/b",
            "a/b/c",
            "a/bc",
            "a-1",
            "a0",
            "A",
            "a\\b",
            "A/b",
            "a\\b/c/",
            "图像",
            "图像/一.png",
            "图像02",
        ];
        for first in paths {
            for second in paths {
                let names = [first.to_string(), second.to_string()];
                let selection = Selection::new(&names);
                for member in paths {
                    let path = member.replace('\\', "/");
                    let expected = names.iter().any(|name| {
                        let name = name.replace('\\', "/");
                        path == name
                            || path.starts_with(&format!("{}/", name.trim_end_matches('/')))
                    });
                    assert_eq!(selection.contains(member), expected, "{names:?} {member}");
                }
            }
        }
        assert!(Selection::new(&[]).contains("unrelated"));
    }
}
