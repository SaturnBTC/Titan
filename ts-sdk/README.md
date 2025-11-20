# Titan Indexer Client SDK for TypeScript

This package provides a TypeScript client for interacting with the [Titan Indexer](https://github.com/titan-io/titan-indexer) for Bitcoin. While the indexer itself is written in Rust, this SDK offers convenient HTTP and TCP clients for TypeScript applications to communicate with the indexer.

The HTTP client uses [axios](https://axios-http.com/) to call REST API endpoints, and the TCP client (which works only in Node.js) uses Node's built-in `net` module to subscribe to real-time events.

---

## Features

- **HTTP Client**:
  - Communicate with endpoints like `/status`, `/tip`, `/tx/:txid`, `/address/:address`, etc.
  - Easily fetch block data, transaction details, inscriptions, and more.
- **TCP Client**:
  - Subscribe to events (e.g. `RuneEtched`, `RuneMinted`, etc.) via a TCP socket.
  - Built-in auto-reconnection logic to handle disconnections gracefully.
  - **Note**: This client uses Node's `net` module and will only work in Node.js (not in browser environments).

---

## Requirements

- **Node.js**: Required for the TCP client since it depends on Node's `net` module.
- **axios**: Used for making HTTP requests.
- **TypeScript**: For type safety and development.
- Bitcoin Node 27.0 (https://bitcoincore.org/bin/bitcoin-core-27.0/)
- A running instance of the Titan Indexer (HTTP on a specified port and TCP for event subscriptions).

Detailed Setup: Follow the [Setup Instructions](../SetupInstructions.md) for step-by-step guidance.

---

## Installation

Install the package via npm or yarn:

```bash
npm i @titanbtcio/sdk
```

## Usage

### HTTP Client

The HTTP client uses axios to communicate with the Titan Indexer's REST API endpoints. Create an instance of TitanHttpClient by passing the base URL of your Titan Indexer service and call the available methods.

#### Example

```typescript
import { TitanHttpClient } from "@titanbtcio/sdk";

async function testHttpClient() {
  // Create an HTTP client instance.
  const httpClient = new TitanHttpClient('http://localhost:3030');

  try {
    // Retrieve the node status.
    const status = await httpClient.getStatus();
    console.log('Index Status:', status);

    // Retrieve the current block tip.
    const tip = await httpClient.getTip();
    console.log('Block Tip:', tip);

    // Fetch address data for a given Bitcoin address.
    const addressData = await httpClient.getAddress('your-bitcoin-address');
    console.log('Address Data:', addressData);

    // Fetch a transaction by txid.
    const transaction = await httpClient.getTransaction('txid-here');
    console.log('Transaction:', transaction);
  } catch (error) {
    console.error('HTTP Client Error:', error);
  }
}

testHttpClient();
```

### TCP Client

The TCP client allows you to subscribe to real-time events from the Titan Indexer. It features automatic reconnection logic on disconnection.

**Important**: The TCP client works only in Node.js since it depends on Node's `net` module.

#### Options

```
{
  autoReconnect?: boolean;                  // Reconnect automatically (default: false)
  // Heartbeat
  heartbeatIntervalMs?: number;             // Send PING every N ms (default: 30000ms)
  heartbeatTimeoutMs?: number;              // Wait for PONG for N ms (default: 10000ms)
  // Exponential backoff (always used)
  maxRetries?: number;                      // Max reconnect attempts (undefined => infinite)
  baseDelayMs?: number;                     // Base backoff delay (default: 1000ms)
  maxDelayMs?: number;                      // Max backoff cap (default: 60000ms)
  jitterRatio?: number;                     // +/- jitter fraction (default: 0.3)
  // Safety
  maxLineLengthBytes?: number;              // Max incoming line length (default: 10 MiB)
}
```

#### Events

- `event`: emitted for each server event (JSON)
- `error`: emitted on errors
- `close`: emitted when the socket closes
- `reconnect`: emitted upon successful (re)connect
- `status`: emitted on status transitions: `Connecting | Connected | Reconnecting | Disconnected`

#### Methods

- `subscribe(request: TcpSubscriptionRequest): void`
- `shutdown(): void`
- `shutdownAsync(): Promise<void>`
- `getStatus(): ConnectionStatus`

#### Example

```typescript
import { TitanTcpClient } from '@titanbtcio/sdk';

function testTcpSubscription() {
  const tcpClient = new TitanTcpClient('localhost', 4000, {
    autoReconnect: true,
    // Use exponential backoff with jitter
    baseDelayMs: 1000,
    maxDelayMs: 60_000,
    jitterRatio: 0.3,
    // Heartbeat (PING/PONG)
    heartbeatIntervalMs: 30_000,
    heartbeatTimeoutMs: 10_000,
  });

  tcpClient.on('status', (s) => console.log('Status:', s));
  tcpClient.on('event', (e) => console.log('Event:', e));
  tcpClient.on('error', (err) => console.error('TCP error:', err));

  tcpClient.subscribe({ subscribe: ['RuneEtched', 'RuneMinted'] });

  // Graceful shutdown
  setTimeout(async () => {
    await tcpClient.shutdownAsync();
    console.log('TCP client shut down.');
  }, 30000);
}

testTcpSubscription();
```

## API Reference

### HTTP Client (TitanHttpClient)

- **getStatus()**: `Promise<Status>`
  Retrieves the node's status (network info, block height, etc.).

- **getTip()**: `Promise<BlockTip>`
  Retrieves the current best block tip.

- **getBlock(query: string)**: `Promise<Block | undefined>`
  Fetches a block by its height or hash.

- **getBlockHashByHeight(height: number)**: `Promise<string | undefined>`
  Returns the block hash for a given height.

- **getBlockTxids(query: string)**: `Promise<string[] | undefined>`
  Retrieves a list of transaction IDs for a block.

- **getAddress(address: string)**: `Promise<AddressData>`
  Retrieves address data including balance and transaction outputs.

- **getTransaction(txid: string)**: `Promise<Transaction | undefined>`
  Retrieves detailed information for a given transaction.

- **getTransactionRaw(txid: string)**: `Promise<Uint8Array | undefined>`
  Retrieves the raw binary data of a transaction.

- **getTransactionHex(txid: string)**: `Promise<string | undefined>`
  Retrieves the raw transaction hex.

- **getTransactionStatus(txid: string)**: `Promise<TransactionStatus | undefined>`
  Retrieves transaction status.

- **sendTransaction(txHex: string)**: `Promise<string>`
  Broadcasts a raw transaction hex to the network.

- **getOutput(txid: string, vout: number)**: `Promise<TxOutEntry | undefined>`
  Retrieves data for a specific transaction output.

- **getInscription(inscriptionId: string)**: `Promise<{ headers: any; data: Uint8Array }>`
  Retrieves inscription headers and data.

- **getRunes(pagination?: Pagination)**: `Promise<PaginationResponse<RuneResponse>>`
  Retrieves a paginated list of runes.

- **getRune(rune: string)**: `Promise<RuneResponse | undefined>`
  Retrieves data for a specific rune.

- **getRuneTransactions(rune: string, pagination?: Pagination)**: `Promise<PaginationResponse<string> | undefined>`
  Retrieves a paginated list of transaction IDs involving a specific rune.

- **getMempoolTxids()**: `Promise<string[]>`
  Retrieves the transaction IDs currently in the mempool.

- **getMempoolEntry(txid: string)**: `Promise<MempoolEntry | undefined>`
  Retrieves a specific mempool entry by its txid.

- **getMempoolEntries(txids: string[])**: `Promise<Map<string, MempoolEntry | undefined>>`
  Retrieves multiple mempool entries by their txids.

- **getMempoolEntriesWithAncestors(txids: string[])**: `Promise<Map<string, MempoolEntry>>`
  Retrieves multiple mempool entries with ancestors by their txids.

- **getAllMempoolEntries()**: `Promise<Map<string, MempoolEntry>>`
  Retrieves all mempool entries.

- **getSubscription(id: string)**: `Promise<Subscription | undefined>`
  Retrieves a subscription by its ID.

- **listSubscriptions()**: `Promise<Subscription[]>`
  Lists all subscriptions.

- **addSubscription(subscription: Subscription)**: `Promise<Subscription>`
  Adds a new subscription.

- **deleteSubscription(id: string)**: `Promise<void>`
  Deletes a subscription by its ID.

### TCP Client (TitanTcpClient)

#### Events

- **event**:
  Emitted when a new event is received from the indexer.

- **error**:
  Emitted when an error occurs with the TCP connection.

- **close**:
  Emitted when the TCP connection is closed.

- **reconnect**:
  Emitted when the client reconnects after a disconnection.

#### Methods

- **subscribe(subscriptionRequest: TcpSubscriptionRequest)**: `void`
  Initiates the subscription process using the provided subscription request. The request is stored so it can be re-sent on reconnection. Replaces any existing subscription safely.

- **shutdown()**: `void`
  Immediately shuts down the TCP client and cancels timers.

- **shutdownAsync()**: `Promise<void>`
  Gracefully shuts down and resolves after the socket is closed and timers are cleared.

- **getStatus()**: `ConnectionStatus`
  Returns the current connection status.
