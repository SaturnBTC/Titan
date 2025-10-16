export interface BlockTip {
  height: number;
  hash: string;
  is_at_tip: boolean;
}

export interface Status {
  block_tip: BlockTip;
  runes_count: number;
  mempool_tx_count: number;
}

export interface Block {
  height: number;
  header: string;
  tx_ids: string[];
  etched_runes: string[];
}

export interface RuneAmount {
  rune_id: string;
  amount: string;
}

export interface SpenderReference {
  txid: string;
  vin: number;
}

export interface SpentStatus {
  spent: boolean;
  vin?: SpenderReference;
}

export interface TransactionStatus {
  confirmed: boolean;
  block_height?: number;
  block_hash?: string;
}

export interface OutPoint {
  txid: string;
  vout: number;
}

export interface AddressTxOut extends OutPoint {
  value: number;
  runes: RuneAmount[];
  risky_runes: RuneAmount[];
  status: TransactionStatus;
  spent: SpentStatus;
  size: number;
  weight: number;
}

export interface AddressData {
  value: number;
  runes: RuneAmount[];
  outputs: AddressTxOut[];
}

export interface TxOut {
  value: number;
  script_pubkey: string;
  runes: RuneAmount[];
  risky_runes: RuneAmount[];
  spent: SpentStatus;
}

export interface TxOutEntry {
  runes: RuneAmount[];
  risky_runes: RuneAmount[];
  value: number;
  spent: SpentStatus;
}

export interface PreviousOutputData {
  value: number;
  runes: RuneAmount[];
  risky_runes: RuneAmount[];
}

export interface TxIn {
  previous_output: OutPoint;
  script_sig: string;
  sequence: number;
  witness: string[];
  previous_output_data?: PreviousOutputData;
}

export interface Transaction {
  version: number;
  lock_time: number;
  input: TxIn[];
  output: TxOut[];
  status: TransactionStatus;
  size: number;
  weight: number;
}

export interface MintResponse {
  start?: number;
  end?: number;
  mintable: boolean;
  cap: string;
  amount: string;
  mints: string;
}

export interface RuneResponse {
  id: string;
  block: number;
  burned: string;
  divisibility: number;
  etching: string;
  number: number;
  premine: string;
  supply: string;
  max_supply: string;
  spaced_rune: string;
  symbol?: string;
  mint: MintResponse | null;
  pending_burns: string;
  pending_mints: string;
  inscription_id?: string;
  timestamp: number;
  turbo: boolean;
}

export interface Subscription {
  id: string;
  endpoint: string;
  event_types: TitanEventType[];
  last_success_epoch_secs: number;
}

export interface Pagination {
  skip?: number;
  limit?: number;
}

export interface PaginationResponse<T> {
  items: T[];
  offset: number;
}

export enum TitanEventType {
  RuneEtched = 'RuneEtched',
  RuneMinted = 'RuneMinted',
  RuneBurned = 'RuneBurned',
  RuneTransferred = 'RuneTransferred',
  AddressModified = 'AddressModified',
  TransactionSubmitted = 'TransactionSubmitted',
  TransactionsAdded = 'TransactionsAdded',
  TransactionsReplaced = 'TransactionsReplaced',
  MempoolTransactionsAdded = 'MempoolTransactionsAdded',
  MempoolTransactionsReplaced = 'MempoolTransactionsReplaced',
  MempoolEntriesUpdated = 'MempoolEntriesUpdated',
  NewBlock = 'NewBlock',
  Reorg = 'Reorg',
}

export interface Location {
  mempool: boolean;
  block_height: number | null;
}

export type TitanEvent =
  | {
      type: TitanEventType.RuneEtched;
      data: {
        location: Location;
        rune_id: string;
        txid: string;
      };
    }
  | {
      type: TitanEventType.TransactionSubmitted;
      data: {
        txid: string;
        entry: MempoolEntry;
      };
    }
  | {
      type: TitanEventType.RuneBurned;
      data: {
        amount: string;
        location: Location;
        rune_id: string;
        txid: string;
      };
    }
  | {
      type: TitanEventType.RuneMinted;
      data: {
        amount: string;
        location: Location;
        rune_id: string;
        txid: string;
      };
    }
  | {
      type: TitanEventType.RuneTransferred;
      data: {
        amount: string;
        location: Location;
        outpoint: string;
        rune_id: string;
        txid: string;
      };
    }
  | {
      type: TitanEventType.AddressModified;
      data: {
        address: string;
        location: Location;
      };
    }
  | {
      type: TitanEventType.MempoolTransactionsAdded;
      data: { txids: [string, MempoolEntry][] };
    }
  | {
      type: TitanEventType.TransactionsAdded;
      data: { txids: string[] };
    }
  | {
      type: TitanEventType.MempoolTransactionsReplaced;
      data: { txids: string[] };
    }
  | {
      type: TitanEventType.TransactionsReplaced;
      data: { txids: string[] };
    }
  | {
      type: TitanEventType.MempoolEntriesUpdated;
      data: { txids: [string, MempoolEntry][] };
    }
  | {
      type: TitanEventType.NewBlock;
      data: {
        block_hash: string;
        block_height: number;
      };
    }
  | {
      type: TitanEventType.Reorg;
      data: {
        height: number;
        depth: number;
      };
    };

/**
 * The request object to subscribe to TCP events.
 * For example, a client might send:
 *   { subscribe: ["RuneEtched", "RuneMinted"] }
 */
export interface TcpSubscriptionRequest {
  subscribe: TitanEventType[];
}

export interface MempoolEntryFee {
  base: number;
  modified: number;
  ancestor: number;
}

export interface MempoolEntry {
  vsize: number;
  weight: number | null;
  descendant_count: number;
  descendant_size: number;
  ancestor_count: number;
  ancestor_size: number;
  fees: MempoolEntryFee;
  depends: string[];
  spentby: string[];
}
