use {
    crate::index::{updater::cache::UpdaterCache, StoreError},
    bitcoin::{OutPoint, ScriptBuf},
    std::collections::{HashMap, HashSet},
    titan_types::SpenderReference,
};

#[derive(Default)]
pub struct AddressUpdater {
    /// All outpoints newly created in this block: (outpoint, script_pubkey)
    new_outpoints: HashMap<OutPoint, ScriptBuf>,

    /// All outpoints spent in this block (except coinbase).
    spent_outpoints: HashMap<OutPoint, SpenderReference>,
}

impl AddressUpdater {
    pub fn new() -> Self {
        Self {
            new_outpoints: HashMap::new(),
            spent_outpoints: HashMap::new(),
        }
    }

    /// Remember a newly created outpoint -> scriptPubKey
    pub fn add_new_outpoint(&mut self, outpoint: OutPoint, script_pubkey: ScriptBuf) {
        if !script_pubkey.is_op_return() {
            self.new_outpoints.insert(outpoint, script_pubkey);
        }
    }

    /// Remember a spent outpoint
    pub fn add_spent_outpoint(&mut self, outpoint: OutPoint, spender_reference: SpenderReference) {
        self.spent_outpoints.insert(outpoint, spender_reference);
    }

    pub fn batch_update_script_pubkey(
        &mut self,
        cache: &mut UpdaterCache,
    ) -> Result<(), StoreError> {
        if cache.settings.mempool {
            self.batch_update_script_pubkeys_for_mempool(cache)?;
        } else {
            self.batch_update_script_pubkeys_for_block(cache)?;
        }

        self.spent_outpoints.clear();
        self.new_outpoints.clear();

        Ok(())
    }

    /// Do a single batched pass to update scriptPubKey entries for
    /// all outpoints in this block. **Important**: we first add
    /// the new outpoints, then remove the spent outpoints. That way,
    /// if an outpoint is created and spent within the same block,
    /// we can find it in ephemeral memory.
    fn batch_update_script_pubkeys_for_block(
        &self,
        cache: &mut UpdaterCache,
    ) -> Result<(), StoreError> {
        // For any spent outpoint that wasn't created in the same block,
        // we need to fetch from DB or ephemeral memory in UpdaterCache.
        let (old_spent_outpoints, new_spent_outpoints): (Vec<_>, Vec<_>) = self
            .spent_outpoints
            .keys()
            .partition(|outpoint| !self.new_outpoints.contains_key(outpoint));

        // ------------------------------------------------------
        // 1. Map all spent outpoints to their scriptPubKey
        // ------------------------------------------------------
        let spent_map = if !old_spent_outpoints.is_empty() {
            cache.get_outpoints_to_script_pubkey(&old_spent_outpoints, false)?
        } else {
            HashMap::new()
        };

        // ------------------------------------------------------
        // 2. Build a combined map: scriptPubKey -> (Vec of new, Vec of spent)
        // ------------------------------------------------------
        //   We'll store new outpoints in (0) and spent outpoints in (1).
        //   This allows us to do all grouping in one structure.
        let mut spk_map: HashMap<ScriptBuf, (Vec<OutPoint>, Vec<OutPoint>)> = HashMap::new();

        // a) Insert spent outpoints that were NOT created in the same flush window
        for (outpoint, script_pubkey) in spent_map {
            let entry = spk_map.entry(script_pubkey).or_default();
            entry.1.push(outpoint); // spent
        }

        // b) Prepare a set with outpoints that are both new and spent within the
        //    current flush window (e.g. created in an earlier block but spent in a later
        //    block before we flush). We must make sure that those outpoints do *not* end
        //    up as "new" and are instead marked as "spent" so that they are removed from
        //    the address index.
        let new_spent_outpoints = new_spent_outpoints.into_iter().collect::<HashSet<_>>();

        // c) Insert new outpoints
        for (outpoint, script_pubkey) in &self.new_outpoints {
            let entry: &mut (Vec<OutPoint>, Vec<OutPoint>) =
                spk_map.entry(script_pubkey.clone()).or_default();

            if new_spent_outpoints.contains(outpoint) {
                // The outpoint was created and later spent in the same flush window – mark it as spent.
                entry.1.push(*outpoint);
            } else {
                // Normal newly-created outpoint.
                entry.0.push(*outpoint);
            }
        }

        cache.set_script_pubkey_entries(spk_map);

        // ------------------------------------------------------
        // 3. Update OutPoint -> ScriptPubKey mapping for *only* those outpoints
        //    that are still considered "new" after the filtering above (i.e. exclude
        //    outpoints that were already spent before the flush).
        // ------------------------------------------------------
        if !self.new_outpoints.is_empty() {
            // Build a filtered map without the outpoints that were spent.
            let outpoints_to_add: HashMap<OutPoint, ScriptBuf> = self
                .new_outpoints
                .iter()
                .filter(|(outpoint, _)| !new_spent_outpoints.contains(outpoint))
                .map(|(outpoint, spk)| (*outpoint, spk.clone()))
                .collect();

            if !outpoints_to_add.is_empty() {
                cache.batch_set_outpoints_to_script_pubkey(outpoints_to_add);
            }
        }

        Ok(())
    }

    fn batch_update_script_pubkeys_for_mempool(
        &self,
        cache: &mut UpdaterCache,
    ) -> Result<(), StoreError> {
        let mut spk_map: HashMap<ScriptBuf, (Vec<OutPoint>, Vec<OutPoint>)> = HashMap::new();

        // a) Insert new outpoints
        for (outpoint, script_pubkey) in &self.new_outpoints {
            let entry = spk_map.entry(script_pubkey.clone()).or_default();
            entry.0.push(*outpoint); // new
        }

        cache.set_script_pubkey_entries(spk_map);
        cache.batch_set_outpoints_to_script_pubkey(self.new_outpoints.clone());

        // b) Insert spent outpoints
        cache.batch_set_spent_outpoints_in_mempool(self.spent_outpoints.clone());

        Ok(())
    }
}
