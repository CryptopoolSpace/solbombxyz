# SolBomb 💣

> The fastest memecoin launchpad on Solana. Fair launch. Instant trading. Zero rug.

[![Network](https://img.shields.io/badge/Solana-Devnet-green)](https://solana.com)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

---

## What is SolBomb?

SolBomb is a pump.fun-style memecoin launchpad built on Solana with:

- **Bonding curve** — constant product formula (x × y = k)
- **Fair launch** — no presale, no team allocation
- **Auto graduation** — hit $69k market cap → Raydium listing
- **1% fee** — on every buy and sell
- **Anti-rug** — dev wallet locked until graduation

---

## How It Works

```
User pays ~0.02 SOL → Token created on-chain
         ↓
Bonding curve activated (virtual SOL + token reserves)
         ↓
Anyone can buy/sell instantly
Price rises as more people buy (constant product)
         ↓
Market cap hits $69k (~85 SOL)
         ↓
Token graduates → Raydium DEX
Liquidity burned 🔥
```

---

## Smart Contract

**Program ID:** `SoLBoMb1111111111111111111111111111111111111`  
**Network:** Solana Devnet  
**Framework:** Anchor 0.31.0

### Instructions

| Instruction | Description |
|-------------|-------------|
| `initialize` | Setup platform config + fee receiver |
| `create_token` | Launch new token + bonding curve |
| `buy` | Buy tokens with SOL (bonding curve) |
| `sell` | Sell tokens back for SOL |

### Bonding Curve Math

```
k = virtual_sol × virtual_tokens

Buy:  tokens_out = virtual_tokens - (k / (virtual_sol + sol_in))
Sell: sol_out = virtual_sol - (k / (virtual_tokens + tokens_in))
```

Initial state:
- Virtual SOL: 30 SOL
- Virtual tokens: 1,000,000,000 (1B)

---

## SDK Usage

```typescript
import { SolBombClient, calculateBuy, getPrice } from './sdk';

const client = SolBombClient.devnet();

// Get all tokens
const tokens = await client.getAllCurves();

// Get single token
const curve = await client.getCurve(mintPublicKey);
console.log(`Price: ${curve.price} lamports/token`);
console.log(`Progress: ${curve.progress}%`);
console.log(`Market Cap: ${curve.marketCapSol} SOL`);

// Calculate buy
const result = calculateBuy(curve, new BN(0.5 * LAMPORTS_PER_SOL));
console.log(`You get: ${result.tokensOut} tokens`);
console.log(`Price impact: ${result.priceImpact}%`);
```

---

## Deploy

```bash
# Install deps
npm install
cargo install --git https://github.com/coral-xyz/anchor avm --locked
avm install 0.31.0

# Configure
solana config set --url devnet
solana airdrop 2

# Build + deploy
anchor build
anchor deploy --provider.cluster devnet
```

---

## Fee Structure

| | Amount |
|---|---|
| Platform fee | 1% per trade |
| Launch cost | ~0.02 SOL |
| Graduation threshold | $69,000 market cap |

---

## Links

- 🌐 Landing: [solbomb.xyz](https://solbomb.xyz)
- 🐦 Twitter: [@solbombxyz](https://twitter.com/solbombxyz)
- 📧 Contact: solbombxyz@gmail.com

---

MIT License · Built on Solana
