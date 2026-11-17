//! Deep-only publication receipts. Native verification is supplied by the existing copier.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
#[derive(Clone)]
pub struct Proof {
    pub identity: (u32, u64),
    pub digest: [u8; 32],
    pub encrypted: bool,
}
struct Receipt {
    target: PathBuf,
    proof: Option<Proof>,
    confirmed: bool,
}
#[derive(Default)]
pub struct Ledger {
    entries: HashMap<PathBuf, Receipt>,
}
impl Ledger {
    pub fn completed(
        &self,
        source: &Path,
        verify: &mut impl FnMut(&Path, &Proof) -> Result<(), String>,
    ) -> Result<bool, String> {
        let Some(receipt) = self.entries.get(source) else {
            return Ok(false);
        };
        if !receipt.confirmed {
            return Err(format!(
                "已保存文件尚未完成复验，未自动重复复制；请从恢复目录检查：{}",
                receipt.target.display()
            ));
        }
        verify(
            &receipt.target,
            receipt
                .proof
                .as_ref()
                .ok_or("已保存文件缺少复验凭据，恢复结果已保留")?,
        )?;
        Ok(true)
    }
    pub fn committed(&mut self, source: &Path, target: &Path) {
        self.entries.insert(
            source.to_owned(),
            Receipt {
                target: target.to_owned(),
                proof: None,
                confirmed: false,
            },
        );
    }
    pub fn proof(&mut self, source: &Path, proof: Proof) {
        if let Some(receipt) = self.entries.get_mut(source) {
            receipt.proof = Some(proof);
        }
    }
    pub fn confirmed(&mut self, source: &Path) {
        if let Some(receipt) = self.entries.get_mut(source) {
            receipt.confirmed = true;
        }
    }
    pub fn len(&self) -> usize {
        self.entries
            .values()
            .filter(|receipt| receipt.confirmed)
            .count()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rename_retry_reuses_exact_committed_destination() {
        let mut ledger = Ledger::default();
        let source = Path::new("stage/a.txt");
        ledger.committed(source, Path::new("output/a (2).txt"));
        ledger.proof(
            source,
            Proof {
                identity: (1, 2),
                digest: [3; 32],
                encrypted: false,
            },
        );
        ledger.confirmed(source);
        let done = ledger
            .completed(source, &mut |target, proof| {
                assert_eq!(target, Path::new("output/a (2).txt"));
                assert_eq!(proof.identity, (1, 2));
                assert_eq!(proof.digest, [3; 32]);
                Ok(())
            })
            .unwrap();
        assert!(done);
        assert_eq!(ledger.len(), 1);
    }
    #[test]
    fn changed_or_missing_output_cannot_discard_recovery() {
        let mut ledger = Ledger::default();
        let source = Path::new("stage/a");
        ledger.committed(source, Path::new("out/a"));
        ledger.proof(
            source,
            Proof {
                identity: (1, 2),
                digest: [3; 32],
                encrypted: false,
            },
        );
        ledger.confirmed(source);
        for error in ["missing", "changed identity", "changed bytes"] {
            assert!(ledger
                .completed(source, &mut |_, _| Err(error.into()))
                .is_err());
        }
        assert_eq!(ledger.len(), 1);
    }
    #[test]
    fn unconfirmed_commit_never_repeats_a_copy_automatically() {
        let mut ledger = Ledger::default();
        let source = Path::new("stage/a");
        ledger.committed(source, Path::new("out/a"));
        assert!(ledger
            .completed(source, &mut |_, _| panic!("must remain recoverable"))
            .is_err());
        assert_eq!(ledger.len(), 0);
    }
}
