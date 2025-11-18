use {
    super::*,
    crate::models::Inscription,
    bitcoin::Transaction,
    titan_types_core::{InscriptionId, SerializedTxid},
};

pub fn index_rune_icon(
    tx: &Transaction,
    txid: SerializedTxid,
) -> Option<(InscriptionId, Inscription)> {
    let envelopes = ParsedEnvelope::from_transaction(tx);

    // In Rune etching, we just want to index the first envelope.
    let envelope = envelopes.into_iter().next();

    //
    if let Some(envelope) = envelope {
        let inscription = envelope.payload;
        let media = inscription.media();

        if media.is_unknown() {
            return None;
        }

        let inscription_id = InscriptionId { txid, index: 0 };

        return Some((inscription_id, inscription));
    }

    None
}
