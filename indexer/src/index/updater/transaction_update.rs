use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use titan_types::SerializedTxid;

#[derive(Debug, Default, Clone)]
pub struct TransactionUpdate {
    /// The set of transactions that entered (were added to) the mempool in this update
    mempool_added: HashSet<SerializedTxid>,

    /// The set of transactions that left (were removed from) the mempool in this update
    mempool_removed: HashSet<SerializedTxid>,

    /// The set of transactions that were added to (mined in) the current best chain
    block_added: HashSet<SerializedTxid>,

    /// The set of transactions that were removed from the best chain (reorged out)
    block_removed: HashSet<SerializedTxid>,

    /// Count of how many times a transaction was removed from blocks in this update.
    /// Needed to detect the edge case where a tx was added to a block, then reorged out,
    /// then re-mined within the same update cycle.
    block_removed_counts: HashMap<SerializedTxid, u32>,

    /// Count of how many times a transaction was added to blocks in this update.
    /// Needed to detect the edge case where a tx was mined, reorged out, then re-mined
    /// within the same update cycle (appearing twice in `block_added`).
    block_added_counts: HashMap<SerializedTxid, u32>,
}

/// This struct is returned by `categorize()` to hold
/// each distinct category of transaction transitions.
#[derive(Debug, Default, Clone)]
pub struct CategorizedTxs {
    /// Transactions that were *both* in mempool_removed **and** in block_added.
    /// Typically these were mined out of the mempool in the new block.
    pub mined_from_mempool: HashSet<SerializedTxid>,

    /// Transactions that were *both* block_removed **and** mempool_added.
    /// Typically these were reorged out, but showed up again in mempool.
    pub reorged_back_to_mempool: HashSet<SerializedTxid>,

    /// Transactions that were *only* in mempool_added (and not block_removed, etc.).
    /// These are newly seen in the mempool for the first time.
    pub new_in_mempool_only: HashSet<SerializedTxid>,

    /// Transactions that were *only* in block_added (and not mempool_removed),
    /// i.e. we never saw them in mempool first.
    pub new_block_only: HashSet<SerializedTxid>,

    /// Transactions that were in block_removed but *not* re-mined
    /// and *not* re‐added to mempool.  
    /// (They have effectively “disappeared” from the best chain, and are not in mempool.)
    pub reorged_out_entirely: HashSet<SerializedTxid>,

    /// Transactions that were mempool_removed but *not* found in block_added,
    /// so presumably RBF or evicted (or conflicted, or pruned).
    pub mempool_rbf_or_evicted: HashSet<SerializedTxid>,

    /// Transactions that appear in *both* `block_removed` and `block_added`.
    /// That can happen in a reorg where a transaction was removed with the old block
    /// but is also included in the new chain tip block. (Hence it never hits the mempool.)
    pub reorged_out_and_remined: HashSet<SerializedTxid>,
}

pub struct TransactionChangeSet {
    pub removed: HashSet<SerializedTxid>,
    pub added: HashSet<SerializedTxid>,
}

impl TransactionUpdate {
    pub fn new(mempool: TransactionChangeSet, block: TransactionChangeSet) -> Self {
        let mut block_added_counts = HashMap::default();
        for txid in &block.added {
            block_added_counts.insert(*txid, 1);
        }

        let mut block_removed_counts = HashMap::default();
        for txid in &block.removed {
            block_removed_counts.insert(*txid, 1);
        }

        Self {
            mempool_added: mempool.added,
            mempool_removed: mempool.removed,
            block_added: block.added,
            block_removed: block.removed,
            block_added_counts,
            block_removed_counts,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.mempool_added.is_empty()
            && self.mempool_removed.is_empty()
            && self.block_added.is_empty()
            && self.block_removed.is_empty()
    }

    pub fn enough_events_to_send(&self) -> bool {
        self.block_added.len() > 10_000
    }

    pub fn add_block_tx(&mut self, txid: SerializedTxid) {
        self.block_added.insert(txid);
        let entry = self.block_added_counts.entry(txid).or_insert(0);
        *entry += 1;
    }

    pub fn remove_block_tx(&mut self, txid: SerializedTxid) {
        self.block_removed.insert(txid);
        let entry = self.block_removed_counts.entry(txid).or_insert(0);
        *entry += 1;
    }

    pub fn update_mempool(&mut self, mempool: TransactionChangeSet) {
        self.mempool_added.extend(mempool.added);
        self.mempool_removed.extend(mempool.removed);
    }

    pub fn reset(&mut self) {
        self.mempool_added = HashSet::default();
        self.mempool_removed = HashSet::default();
        self.block_added = HashSet::default();
        self.block_removed = HashSet::default();
        self.block_added_counts = HashMap::default();
        self.block_removed_counts = HashMap::default();
    }

    /// Categorize transactions into buckets describing what happened to them
    /// across mempool and block changes.
    pub fn categorize(&self) -> CategorizedTxs {
        let mut result = CategorizedTxs::default();

        // 1. Mined from mempool: present in both mempool_removed and block_added
        for txid in self.mempool_removed.intersection(&self.block_added) {
            result.mined_from_mempool.insert(*txid);
        }

        // 2. Reorged back to mempool: present in both block_removed and mempool_added
        for txid in self.block_removed.intersection(&self.mempool_added) {
            result.reorged_back_to_mempool.insert(*txid);
        }

        // 3. Newly in mempool only (i.e. in mempool_added but *not* also in block_removed)
        //    and not in the intersection from #2
        for txid in &self.mempool_added {
            if !self.block_removed.contains(txid) {
                result.new_in_mempool_only.insert(*txid);
            }
        }
        // You might prefer a set-difference approach:
        //   let new_in_mempool = &self.mempool_added - &self.block_removed;
        //   result.new_in_mempool_only = new_in_mempool - &result.reorged_back_to_mempool;

        // 4. New in block only (i.e. in block_added but not also in mempool_removed)
        //    ignoring the intersection from #1
        for txid in &self.block_added {
            if !self.mempool_removed.contains(txid) {
                result.new_block_only.insert(*txid);
            }
        }

        // 5. Reorged out entirely:
        //    in block_removed, but not reorged_back_to_mempool, not re-mined
        //    So: block_removed ∖ (mempool_added ∪ block_added).
        for txid in &self.block_removed {
            let is_reorged_back = self.mempool_added.contains(txid);
            let is_remined = self.block_added.contains(txid);
            if !is_reorged_back && !is_remined {
                result.reorged_out_entirely.insert(*txid);
            }
        }

        // 6. Mempool RBF or eviction:
        //    in mempool_removed, but *not* in block_added (so not mined).
        //    This lumps all mempool removals that aren't a direct “mined” event.
        //    Some of these might be genuine RBF replaced, some might be eviction, conflict, etc.
        for txid in &self.mempool_removed {
            if !self.block_added.contains(txid) {
                result.mempool_rbf_or_evicted.insert(*txid);
            }
        }

        // 7. Reorged out and re-mined:
        //    in block_removed as well as block_added. That means it was removed
        //    (the old block got replaced) but also added in the new chain tip block.
        //    No time in mempool. It's fairly unusual but can happen.
        for txid in self.block_removed.intersection(&self.block_added) {
            result.reorged_out_and_remined.insert(*txid);
        }

        result
    }

    /// Categorize into exactly two “buckets”:
    ///
    /// 1. **Newly added**:
    ///    TXs that were **not** previously in mempool or chain,
    ///    but are **now** in mempool or chain.
    ///
    /// 2. **Fully removed**:
    ///    TXs that **were** in mempool or chain,
    ///    but are now in **neither**.
    ///
    /// Everything else (e.g. reorg from chain to mempool, or mempool to chain)
    /// does *not* appear in these two sets.
    ///
    /// Returns `(newly_added, fully_removed)`.
    pub fn categorize_to_change_set(&self) -> TransactionChangeSet {
        // We consider the union of `mempool_added` and `block_added` to be
        // “transactions that ended up in the system (mempool or chain) *now*”.
        //
        // We consider the union of `mempool_removed` and `block_removed` to be
        // “transactions that left the system *now*”.
        //
        // However, we only want:
        //   - newly_added: those that are *only* in the added sets and *not*
        //                  in the removed sets (so they truly came from outside).
        //
        //   - fully_removed: those that are *only* in the removed sets
        //                    and *not* in the added sets (so they truly left entirely).
        //
        // If a transaction is both in `mempool_added` and `mempool_removed`,
        // that implies it was in the mempool before, left, and re-entered in the same update,
        // so it is *not* “brand-new from outside.” Likewise for `block_added` + `block_removed`.

        let in_added = &self.mempool_added | &self.block_added; // union
        let in_removed = &self.mempool_removed | &self.block_removed; // union

        // 1. Newly added: present in `in_added` but NOT present in `in_removed`.
        //    This means they were not in our system *before* this update,
        //    but have arrived in mempool or chain now.
        let newly_added = &in_added - &in_removed;

        // 2. Fully removed: present in `in_removed` but NOT present in `in_added`.
        //    This means they were in our system *before* (mempool or chain),
        //    but they are absent now.
        let fully_removed = &in_removed - &in_added;

        TransactionChangeSet {
            removed: fully_removed,
            added: newly_added,
        }
    }

    pub fn categorize_to_mempool(&self) -> TransactionChangeSet {
        TransactionChangeSet {
            added: self.mempool_added.clone(),
            removed: self.mempool_removed.clone(),
        }
    }

    /// Detect txids that were reorged out and then re-mined within the same update,
    /// i.e., present in `block_removed` and added to `block_added` at least twice.
    pub fn block_added_after_mined_and_reorg(&self) -> HashSet<SerializedTxid> {
        self.block_added_counts
            .iter()
            .filter_map(
                |(txid, added_count)| match self.block_removed_counts.get(txid) {
                    Some(removed_count) if *removed_count > 0 && *added_count > *removed_count => {
                        Some(*txid)
                    }
                    _ => None,
                },
            )
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_hash::FxHashSet as HashSet;
    use std::iter::FromIterator;

    fn tx(i: u8) -> SerializedTxid {
        SerializedTxid::from([i; 32])
    }

    fn set<const N: usize>(items: [SerializedTxid; N]) -> HashSet<SerializedTxid> {
        HashSet::from_iter(items.into_iter())
    }

    fn cs<A: IntoIterator<Item = SerializedTxid>, R: IntoIterator<Item = SerializedTxid>>(
        added: A,
        removed: R,
    ) -> TransactionChangeSet {
        TransactionChangeSet {
            added: added.into_iter().collect(),
            removed: removed.into_iter().collect(),
        }
    }

    #[test]
    fn categorize_to_change_set_newly_added_in_mempool_only() {
        let a = tx(1);
        let update = TransactionUpdate::new(
            cs([a].into_iter(), [].into_iter()),
            cs([].into_iter(), [].into_iter()),
        );

        let out = update.categorize_to_change_set();
        assert_eq!(out.added, set([a]));
        assert!(out.removed.is_empty());
    }

    #[test]
    fn categorize_to_change_set_newly_added_in_block_only() {
        let b = tx(2);
        let update = TransactionUpdate::new(
            cs([].into_iter(), [].into_iter()),
            cs([b].into_iter(), [].into_iter()),
        );

        let out = update.categorize_to_change_set();
        assert_eq!(out.added, set([b]));
        assert!(out.removed.is_empty());
    }

    #[test]
    fn categorize_to_change_set_neither_if_mempool_removed_and_block_added() {
        let x = tx(3);
        let update = TransactionUpdate::new(
            cs([].into_iter(), [x].into_iter()), // mempool_removed
            cs([x].into_iter(), [].into_iter()), // block_added
        );

        let out = update.categorize_to_change_set();
        assert!(out.added.is_empty());
        assert!(out.removed.is_empty());
    }

    #[test]
    fn categorize_to_change_set_neither_if_block_removed_and_mempool_added() {
        let y = tx(4);
        let update = TransactionUpdate::new(
            cs([y].into_iter(), [].into_iter()), // mempool_added
            cs([].into_iter(), [y].into_iter()), // block_removed
        );

        let out = update.categorize_to_change_set();
        assert!(out.added.is_empty());
        assert!(out.removed.is_empty());
    }

    #[test]
    fn categorize_to_change_set_removed_if_block_removed_only() {
        let r = tx(5);
        let update = TransactionUpdate::new(
            cs([].into_iter(), [].into_iter()),
            cs([].into_iter(), [r].into_iter()), // block_removed
        );

        let out = update.categorize_to_change_set();
        assert!(out.added.is_empty());
        assert_eq!(out.removed, set([r]));
    }

    #[test]
    fn categorize_to_change_set_removed_if_mempool_removed_only() {
        let r = tx(6);
        let update = TransactionUpdate::new(
            cs([].into_iter(), [r].into_iter()), // mempool_removed
            cs([].into_iter(), [].into_iter()),
        );

        let out = update.categorize_to_change_set();
        assert!(out.added.is_empty());
        assert_eq!(out.removed, set([r]));
    }

    // Additional edge cases

    #[test]
    fn categorize_to_change_set_neither_if_block_removed_and_block_added_same_tx() {
        let t = tx(7);
        let update = TransactionUpdate::new(
            cs([].into_iter(), [].into_iter()),
            cs([t].into_iter(), [t].into_iter()),
        );
        let out = update.categorize_to_change_set();
        assert!(out.added.is_empty());
        assert!(out.removed.is_empty());
    }

    #[test]
    fn categorize_to_change_set_neither_if_mempool_removed_and_added_same_tx() {
        let t = tx(8);
        let update = TransactionUpdate::new(
            cs([t].into_iter(), [t].into_iter()),
            cs([].into_iter(), [].into_iter()),
        );
        let out = update.categorize_to_change_set();
        assert!(out.added.is_empty());
        assert!(out.removed.is_empty());
    }

    #[test]
    fn categorize_to_change_set_mixed_multiple_transactions() {
        let a = tx(10); // mempool-only new -> added
        let b = tx(11); // block-only new -> added
        let c = tx(12); // mempool_removed + block_added -> neither
        let d = tx(13); // block_removed + mempool_added -> neither
        let e = tx(14); // block_removed only -> removed
        let f = tx(15); // mempool_removed only -> removed

        let update = TransactionUpdate::new(
            cs([a, d].into_iter(), [c, f].into_iter()),
            cs([b, c].into_iter(), [d, e].into_iter()),
        );
        let out = update.categorize_to_change_set();

        assert_eq!(out.added, set([a, b]));
        assert_eq!(out.removed, set([e, f]));
    }

    #[test]
    fn helper_is_empty_and_reset() {
        let a = tx(21);
        let b = tx(22);
        let mut update = TransactionUpdate::new(
            cs([a].into_iter(), [].into_iter()),
            cs([b].into_iter(), [].into_iter()),
        );
        assert!(!update.is_empty());
        update.reset();
        assert!(update.is_empty());
    }

    #[test]
    fn helper_update_mempool_extends_sets() {
        let a1 = tx(31);
        let a2 = tx(32);
        let r1 = tx(33);
        let r2 = tx(34);

        let mut update = TransactionUpdate::new(
            cs([a1].into_iter(), [r1].into_iter()),
            cs([].into_iter(), [].into_iter()),
        );
        update.update_mempool(cs([a2].into_iter(), [r2].into_iter()));

        let mem = update.categorize_to_mempool();
        assert_eq!(mem.added, set([a1, a2]));
        assert_eq!(mem.removed, set([r1, r2]));
    }

    #[test]
    fn helper_categorize_to_mempool_reflects_mempool_sets_only() {
        let a = tx(41);
        let r = tx(42);
        let ba = tx(43);
        let br = tx(44);

        let update = TransactionUpdate::new(
            cs([a].into_iter(), [r].into_iter()),
            cs([ba].into_iter(), [br].into_iter()),
        );
        let mem = update.categorize_to_mempool();
        assert_eq!(mem.added, set([a]));
        assert_eq!(mem.removed, set([r]));
    }

    #[test]
    fn helper_enough_events_to_send_threshold() {
        let mut update = TransactionUpdate::default();
        // Exactly 10_000 distinct txids -> false
        for i in 0..10_000u32 {
            let bytes4 = i.to_be_bytes();
            let mut bytes32 = [0u8; 32];
            // Repeat the 4 bytes 8 times to make a 32-byte unique pattern per i
            for k in 0..8 {
                bytes32[k * 4..(k + 1) * 4].copy_from_slice(&bytes4);
            }
            update.add_block_tx(SerializedTxid::from(bytes32));
        }
        assert!(!update.enough_events_to_send());

        // Add one more -> true
        update.add_block_tx(tx(200));
        assert!(update.enough_events_to_send());
    }

    #[test]
    fn no_event_if_reorged_twice_and_remined_twice() {
        // Start with tx in a block, then reorged out twice and re-mined twice within the same update
        let t = tx(50);

        let mut update = TransactionUpdate::new(
            cs([t].into_iter(), [].into_iter()),
            cs([].into_iter(), [].into_iter()),
        );

        // Simulate two reorg outs
        update.remove_block_tx(t);
        update.remove_block_tx(t);

        // Simulate two re-mines
        update.add_block_tx(t);
        update.add_block_tx(t);

        // Our helper should not flag this as an event-worthy re-add
        let flagged = update.block_added_after_mined_and_reorg();
        assert!(flagged.is_empty());
    }

    #[test]
    fn event_if_direct_block_add_then_reorg_once_and_remined_once() {
        let t = tx(51);

        // New tx added directly to a block
        let mut update = TransactionUpdate::new(
            cs([].into_iter(), [].into_iter()),
            cs([t].into_iter(), [].into_iter()),
        );

        // One reorg out
        update.remove_block_tx(t);

        // One re-mine
        update.add_block_tx(t);

        // Should be flagged as event-worthy (added > removed)
        let flagged = update.block_added_after_mined_and_reorg();
        assert!(flagged.contains(&t));
    }

    #[test]
    fn no_event_if_reorged_once_and_not_remined() {
        let t = tx(52);
        let mut update = TransactionUpdate::new(
            cs([t].into_iter(), [].into_iter()),
            cs([].into_iter(), [].into_iter()),
        );

        // Reorg out once, not re-mined
        update.remove_block_tx(t);

        let flagged = update.block_added_after_mined_and_reorg();
        assert!(flagged.is_empty());
    }
}
