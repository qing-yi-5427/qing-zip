//! In-memory deep-extraction state machine. No filesystem or decoder side effects.
//! Depth counts the root archive as 1. Archive count counts distinct nodes,
//! including refused/failed nodes; retrying a node does not count it twice.
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct Limits {
    pub depth: usize,
    pub archives: usize,
    pub files: u64,
    /// Cumulative decoded work, including intermediate archives and failed attempts.
    pub bytes: u64,
    pub reserve_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            depth: 8,
            archives: 256,
            files: 100_000,
            bytes: 20 * 1024 * 1024 * 1024,
            reserve_bytes: 1024 * 1024 * 1024,
        }
    }
}
impl Limits {
    pub fn validate(&self) -> Result<(), String> {
        if self.depth == 0
            || self.depth > 64
            || self.archives == 0
            || self.archives > 10_000
            || self.files == 0
            || self.bytes == 0
        {
            return Err("深度、压缩包数量、文件数量和累计解码量必须大于零；最大深度为 64，最多 10000 个压缩包".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Ready,
    Waiting,
    Pending(String),
    Done,
    Protected,
    Skipped,
}
#[derive(Clone, Debug)]
pub struct Node {
    /// Opaque source identity: the native adapter resolves this within its owned stage.
    pub source: String,
    pub name: String,
    pub parent: Option<usize>,
    pub depth: usize,
    pub state: State,
    pub children: Vec<usize>,
    pub tree: Option<String>,
    /// Collapse archive-only wrappers; never trim the final content directory.
    pub archive_only: bool,
    pub unscanned: bool,
}
#[derive(Clone, Debug)]
pub struct Candidate {
    pub source: String,
    pub name: String,
}
pub struct Plan {
    pub limits: Limits,
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
    pub decoded_bytes: u64,
    pub decoded_files: u64,
    /// Sources left untouched after the bounded archive queue filled.
    pub undiscovered_sources: u64,
    seen: BTreeSet<String>,
}
impl Plan {
    pub fn new(roots: Vec<Candidate>, limits: Limits) -> Result<Self, String> {
        limits.validate()?;
        let mut plan = Self {
            limits,
            nodes: vec![],
            roots: vec![],
            decoded_bytes: 0,
            decoded_files: 0,
            undiscovered_sources: 0,
            seen: BTreeSet::new(),
        };
        for root in roots {
            if let Some(id) = plan.add(root, None) {
                plan.roots.push(id);
            }
        }
        if plan.roots.is_empty() {
            return Err("请选择压缩包".into());
        }
        Ok(plan)
    }
    fn add(&mut self, candidate: Candidate, parent: Option<usize>) -> Option<usize> {
        // Keep at most one overflow explanation; do not retain unbounded source identities.
        if self.nodes.len() > self.limits.archives {
            self.undiscovered_sources += 1;
            return None;
        }
        if !self.seen.insert(candidate.source.clone()) {
            return None;
        }
        let depth = parent.map_or(1, |id| self.nodes[id].depth + 1);
        let state = if depth > self.limits.depth {
            State::Pending("已达到最大解压层数".into())
        } else if self.nodes.len() >= self.limits.archives {
            State::Pending("已达到压缩包数量限制".into())
        } else {
            State::Ready
        };
        let id = self.nodes.len();
        self.nodes.push(Node {
            source: candidate.source,
            name: candidate.name,
            parent,
            depth,
            state,
            children: vec![],
            tree: None,
            archive_only: false,
            unscanned: false,
        });
        Some(id)
    }
    /// Children run before subsequent roots, but a pending child never blocks its siblings.
    pub fn next(&self) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.state == State::Ready)
            .max_by_key(|(id, n)| (n.depth, std::cmp::Reverse(*id)))
            .map(|(id, _)| id)
    }
    pub fn remaining_bytes(&self) -> u64 {
        self.limits.bytes.saturating_sub(self.decoded_bytes)
    }
    pub fn preflight(&self, files: u64, bytes: Option<u64>) -> Result<(), String> {
        if files > self.limits.files.saturating_sub(self.decoded_files) {
            return Err("已达到累计解码文件数量限制".into());
        }
        if bytes.is_some_and(|bytes| bytes > self.remaining_bytes()) {
            return Err("已达到累计解码量限制".into());
        }
        Ok(())
    }
    /// Every decoding attempt calls this once, including failures and retries.
    /// A decoder must enforce its runtime write bound; this detects accounting errors too.
    pub fn charge(&mut self, files: u64, bytes: u64) -> Result<(), String> {
        self.decoded_bytes = self
            .decoded_bytes
            .checked_add(bytes)
            .ok_or("累计解码量溢出")?;
        self.decoded_files = self
            .decoded_files
            .checked_add(files)
            .ok_or("累计文件数量溢出")?;
        self.preflight(0, Some(0))?;
        if self.decoded_bytes > self.limits.bytes || self.decoded_files > self.limits.files {
            return Err("累计解码工作量超过限制".into());
        }
        Ok(())
    }
    pub fn pending(&mut self, id: usize, error: String) {
        self.nodes[id].state = State::Pending(error);
    }
    pub fn decoded(
        &mut self,
        id: usize,
        tree: String,
        candidates: Vec<Candidate>,
        archive_only: bool,
    ) {
        self.nodes[id].tree = Some(tree);
        self.nodes[id].archive_only = archive_only && candidates.len() == 1;
        for candidate in candidates {
            if let Some(child) = self.add(candidate, Some(id)) {
                self.nodes[id].children.push(child);
            }
        }
        self.nodes[id].state = State::Waiting;
    }
    pub fn assembly_ready(&self) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, node)| {
                node.state == State::Waiting
                    && node.children.iter().all(|&id| {
                        matches!(
                            self.nodes[id].state,
                            State::Done | State::Protected | State::Skipped
                        )
                    })
            })
            .map(|(id, _)| id)
    }
    pub fn assembled(&mut self, id: usize, tree: String) {
        self.nodes[id].tree = Some(tree);
        self.nodes[id].state = State::Done;
    }
    /// Done only means scheduling finished. Cleanup requires this stronger outcome.
    pub fn fully_expanded(&self, id: usize) -> bool {
        !self.nodes[id].unscanned
            && matches!(self.nodes[id].state, State::Done | State::Protected)
            && self.nodes[id]
                .children
                .iter()
                .all(|&child| self.fully_expanded(child))
    }
    pub fn retry(&mut self, id: usize) -> Result<(), String> {
        let node = self.nodes.get_mut(id).ok_or("深度解压项目不存在")?;
        if !matches!(node.state, State::Pending(_)) {
            return Err("该项目无需重试".into());
        }
        if node.depth > self.limits.depth || id >= self.limits.archives {
            return Err("请先提高限制，或保留此压缩包继续".into());
        }
        node.state = if node.tree.is_some() {
            State::Waiting
        } else {
            State::Ready
        };
        Ok(())
    }
    /// A skipped nested archive stays in its parent's verified output tree.
    pub fn skip(&mut self, id: usize) -> Result<(), String> {
        let node = self.nodes.get_mut(id).ok_or("深度解压项目不存在")?;
        if !matches!(node.state, State::Pending(_)) {
            return Err("只能保留等待处理的压缩包".into());
        }
        node.state = State::Skipped;
        Ok(())
    }
    pub fn complete(&self) -> bool {
        self.roots.iter().all(|&id| {
            matches!(
                self.nodes[id].state,
                State::Done | State::Protected | State::Skipped
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn c(name: &str) -> Candidate {
        Candidate {
            source: name.into(),
            name: name.into(),
        }
    }
    fn plan() -> Plan {
        Plan::new(vec![c("outer")], Limits::default()).unwrap()
    }
    #[test]
    fn password_retry_keeps_successful_children_and_exact_pending_node() {
        let mut p = plan();
        p.decoded(0, "root-stage".into(), vec![c("a"), c("b")], false);
        p.decoded(1, "a-stage".into(), vec![], false);
        p.assembled(1, "a-stage".into());
        p.pending(2, "password required".into());
        assert_eq!(p.next(), None);
        assert_eq!(p.assembly_ready(), None);
        p.retry(2).unwrap();
        assert_eq!(p.next(), Some(2));
        assert_eq!(p.nodes[1].state, State::Done);
        p.decoded(2, "b-stage".into(), vec![], false);
        p.assembled(2, "b-stage".into());
        assert_eq!(p.assembly_ready(), Some(0));
        p.assembled(0, "resolved".into());
        assert!(p.complete());
    }
    #[test]
    fn failed_child_can_be_retained_without_stopping_siblings() {
        let mut p = plan();
        p.decoded(0, "root".into(), vec![c("bad"), c("good")], false);
        p.pending(1, "broken".into());
        p.skip(1).unwrap();
        assert_eq!(p.next(), Some(2));
        p.decoded(2, "good".into(), vec![], false);
        p.assembled(2, "good".into());
        assert_eq!(p.assembly_ready(), Some(0));
    }
    #[test]
    fn depth_and_archive_count_include_root_and_retry_is_not_a_new_archive() {
        let mut p = Plan::new(
            vec![c("root")],
            Limits {
                depth: 1,
                archives: 1,
                ..Limits::default()
            },
        )
        .unwrap();
        p.pending(0, "password".into());
        p.retry(0).unwrap();
        assert_eq!(p.nodes.len(), 1);
        p.decoded(0, "root".into(), vec![c("child")], true);
        assert!(matches!(p.nodes[1].state, State::Pending(_)));
        assert!(p.retry(1).is_err());
        p.skip(1).unwrap();
        assert_eq!(p.assembly_ready(), Some(0));
    }
    #[test]
    fn failed_attempts_raw_tar_and_retries_all_spend_the_same_budget() {
        let mut p = plan();
        p.limits.bytes = 100;
        p.limits.files = 3;
        p.preflight(1, Some(50)).unwrap();
        p.charge(1, 20).unwrap(); // failed stream attempt
        p.charge(1, 50).unwrap(); // retry decodes intermediate TAR
        assert!(p.preflight(1, Some(31)).is_err());
        p.charge(1, 30).unwrap();
        assert_eq!(p.remaining_bytes(), 0);
        assert!(p.preflight(1, Some(0)).is_err());
    }
    #[test]
    fn only_archive_wrappers_are_collapsible() {
        let mut p = plan();
        p.decoded(0, "root".into(), vec![c("wrapper/inner.zip")], true);
        p.decoded(1, "final/照片".into(), vec![], false);
        p.assembled(1, "final/照片".into());
        assert!(p.nodes[0].archive_only);
        assert!(!p.nodes[1].archive_only);
        // The native adapter returns the whole child's tree: its final 照片 folder survives.
        p.assembled(0, p.nodes[1].tree.clone().unwrap());
        assert_eq!(p.nodes[0].tree.as_deref(), Some("final/照片"));
    }
    #[test]
    fn volume_aliases_and_repeated_roots_are_only_scheduled_once() {
        let p = Plan::new(
            vec![
                Candidate {
                    source: "file-volume:123".into(),
                    name: "A.ZIP".into(),
                },
                Candidate {
                    source: "file-volume:123".into(),
                    name: "a.zip.001".into(),
                },
            ],
            Limits::default(),
        )
        .unwrap();
        assert_eq!(p.roots.len(), 1);
    }
    #[test]
    fn skipped_grandchild_never_authorizes_ancestor_package_cleanup() {
        let mut p = plan();
        p.decoded(0, "root".into(), vec![c("child")], true);
        p.decoded(1, "child".into(), vec![c("grandchild")], true);
        p.pending(2, "password".into());
        p.skip(2).unwrap();
        p.assembled(1, "child".into());
        p.assembled(0, "root".into());
        assert!(!p.fully_expanded(0));
        assert!(!p.fully_expanded(1));
    }
    #[test]
    fn archive_queue_has_one_bounded_overflow_explanation() {
        let mut p = Plan::new(
            vec![c("root")],
            Limits {
                archives: 2,
                ..Limits::default()
            },
        )
        .unwrap();
        p.decoded(
            0,
            "root".into(),
            (0..10000)
                .map(|n| Candidate {
                    source: format!("child{n}"),
                    name: format!("child{n}"),
                })
                .collect(),
            false,
        );
        assert_eq!(p.nodes.len(), 3);
        assert_eq!(p.undiscovered_sources, 9998);
    }
    #[test]
    fn protected_document_is_a_successful_terminal_file_and_keeps_other_roots_independent() {
        let mut p = Plan::new(vec![c("one"), c("two")], Limits::default()).unwrap();
        p.decoded(0, "one-tree".into(), vec![c("paper.docx")], true);
        p.nodes[2].state = State::Protected;
        assert_eq!(p.assembly_ready(), Some(0));
        p.assembled(0, "one-tree".into());
        p.nodes[1].unscanned = true;
        p.undiscovered_sources = 1;
        assert!(p.fully_expanded(0));
        assert!(!p.fully_expanded(1));
    }
    #[test]
    fn conservative_known_size_failure_cannot_bypass_budget_on_retry() {
        let mut p = plan();
        p.limits.bytes = 100;
        p.preflight(1, Some(70)).unwrap();
        p.charge(1, 70).unwrap(); // full selected declaration charged despite decoder cleanup
        assert!(p.preflight(1, Some(70)).is_err());
        assert_eq!(p.remaining_bytes(), 30);
    }
}
