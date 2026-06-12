/**
 * SolBomb SDK — TypeScript client for interacting with SolBomb bonding curve
 */

import {
  Connection, PublicKey, Keypair, Transaction,
  SystemProgram, LAMPORTS_PER_SOL, clusterApiUrl
} from '@solana/web3.js';
import * as anchor from '@coral-xyz/anchor';
import BN from 'bn.js';

// ── Constants ─────────────────────────────────────────────────────────────────

export const SOLBOMB_PROGRAM_ID = new PublicKey('SoLBoMb1111111111111111111111111111111111111');
export const VIRTUAL_SOL_RESERVES  = new BN(30 * LAMPORTS_PER_SOL);
export const TOTAL_SUPPLY          = new BN(1_000_000_000).mul(new BN(1_000_000)); // 1B with 6 decimals
export const GRADUATION_SOL        = 85 * LAMPORTS_PER_SOL; // ~$69k
export const PLATFORM_FEE_BPS      = 100; // 1%

// ── PDA helpers ───────────────────────────────────────────────────────────────

export function findCurvePDA(mint: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from('curve'), mint.toBuffer()],
    SOLBOMB_PROGRAM_ID
  );
}

export function findVaultPDA(mint: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from('vault'), mint.toBuffer()],
    SOLBOMB_PROGRAM_ID
  );
}

export function findConfigPDA(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from('config')],
    SOLBOMB_PROGRAM_ID
  );
}

// ── Bonding curve math ────────────────────────────────────────────────────────

export interface CurveState {
  virtualSolReserves:   BN;
  virtualTokenReserves: BN;
  realSolReserves:      BN;
  realTokenReserves:    BN;
}

/**
 * Calculate tokens out for a given SOL input
 * Uses constant product formula: k = virtual_sol * virtual_tokens
 */
export function calculateBuy(curve: CurveState, solIn: BN): {
  tokensOut: BN;
  newPrice:  BN;
  priceImpact: number;
} {
  const fee     = solIn.muln(PLATFORM_FEE_BPS).divn(10_000);
  const solNet  = solIn.sub(fee);

  const k = curve.virtualSolReserves.mul(curve.virtualTokenReserves);
  const newVirtualSol = curve.virtualSolReserves.add(solNet);
  const newVirtualTokens = k.div(newVirtualSol);
  const tokensOut = curve.virtualTokenReserves.sub(newVirtualTokens);

  const oldPrice  = getPrice(curve);
  const newCurve  = {
    ...curve,
    virtualSolReserves:   newVirtualSol,
    virtualTokenReserves: newVirtualTokens,
  };
  const newPrice  = getPrice(newCurve);
  const priceImpact = newPrice.sub(oldPrice).muln(100).div(oldPrice).toNumber();

  return { tokensOut, newPrice, priceImpact };
}

/**
 * Calculate SOL out for a given token input (sell)
 */
export function calculateSell(curve: CurveState, tokensIn: BN): {
  solOut:    BN;
  newPrice:  BN;
  priceImpact: number;
} {
  const k = curve.virtualSolReserves.mul(curve.virtualTokenReserves);
  const newVirtualTokens = curve.virtualTokenReserves.add(tokensIn);
  const newVirtualSol = k.div(newVirtualTokens);
  const solGross = curve.virtualSolReserves.sub(newVirtualSol);
  const fee = solGross.muln(PLATFORM_FEE_BPS).divn(10_000);
  const solOut = solGross.sub(fee);

  const oldPrice = getPrice(curve);
  const newCurve = {
    ...curve,
    virtualSolReserves:   newVirtualSol,
    virtualTokenReserves: newVirtualTokens,
  };
  const newPrice = getPrice(newCurve);
  const priceImpact = oldPrice.sub(newPrice).muln(100).div(oldPrice).toNumber();

  return { solOut, newPrice, priceImpact };
}

/**
 * Get current token price in lamports per token
 */
export function getPrice(curve: CurveState): BN {
  if (curve.virtualTokenReserves.isZero()) return new BN(0);
  return curve.virtualSolReserves
    .muln(1_000_000)
    .div(curve.virtualTokenReserves);
}

/**
 * Get graduation progress (0–100%)
 */
export function getGraduationProgress(realSolReserves: BN): number {
  const target = new BN(GRADUATION_SOL);
  if (realSolReserves.gte(target)) return 100;
  return realSolReserves.muln(100).div(target).toNumber();
}

/**
 * Get market cap in SOL
 */
export function getMarketCapSol(curve: CurveState): number {
  const price = getPrice(curve);
  return TOTAL_SUPPLY.mul(price).divn(1_000_000).toNumber() / LAMPORTS_PER_SOL;
}

// ── SolBomb Client ────────────────────────────────────────────────────────────

export class SolBombClient {
  constructor(
    public readonly connection: Connection,
    public readonly programId: PublicKey = SOLBOMB_PROGRAM_ID
  ) {}

  static devnet(): SolBombClient {
    return new SolBombClient(
      new Connection(clusterApiUrl('devnet'), 'confirmed')
    );
  }

  static mainnet(rpcUrl?: string): SolBombClient {
    return new SolBombClient(
      new Connection(rpcUrl || clusterApiUrl('mainnet-beta'), 'confirmed')
    );
  }

  // Fetch all bonding curves (tokens)
  async getAllCurves(): Promise<BondingCurveAccount[]> {
    const accounts = await this.connection.getProgramAccounts(this.programId, {
      filters: [{ dataSize: 892 }]
    });
    return accounts
      .map(({ pubkey, account }) => {
        try { return this.deserializeCurve(pubkey, account.data); }
        catch { return null; }
      })
      .filter((c): c is BondingCurveAccount => c !== null);
  }

  // Fetch single curve by mint
  async getCurve(mint: PublicKey): Promise<BondingCurveAccount | null> {
    const [curvePDA] = findCurvePDA(mint);
    const account = await this.connection.getAccountInfo(curvePDA);
    if (!account) return null;
    return this.deserializeCurve(curvePDA, account.data);
  }

  // Get vault balance (real SOL in curve)
  async getVaultBalance(mint: PublicKey): Promise<number> {
    const [vaultPDA] = findVaultPDA(mint);
    const balance = await this.connection.getBalance(vaultPDA);
    return balance / LAMPORTS_PER_SOL;
  }

  private deserializeCurve(pubkey: PublicKey, data: Buffer): BondingCurveAccount {
    let offset = 8; // skip discriminator

    const creator = new PublicKey(data.slice(offset, offset + 32)); offset += 32;
    const mint    = new PublicKey(data.slice(offset, offset + 32)); offset += 32;

    const nameLen = data.readUInt32LE(offset); offset += 4;
    const name    = data.slice(offset, offset + nameLen).toString('utf8'); offset += nameLen;

    const symLen  = data.readUInt32LE(offset); offset += 4;
    const symbol  = data.slice(offset, offset + symLen).toString('utf8'); offset += symLen;

    const uriLen  = data.readUInt32LE(offset); offset += 4;
    const uri     = data.slice(offset, offset + uriLen).toString('utf8'); offset += uriLen;

    const descLen = data.readUInt32LE(offset); offset += 4;
    const description = data.slice(offset, offset + descLen).toString('utf8'); offset += descLen;

    const virtualSolReserves   = new BN(data.slice(offset, offset + 8), 'le'); offset += 8;
    const virtualTokenReserves = new BN(data.slice(offset, offset + 8), 'le'); offset += 8;
    const realSolReserves      = new BN(data.slice(offset, offset + 8), 'le'); offset += 8;
    const realTokenReserves    = new BN(data.slice(offset, offset + 8), 'le'); offset += 8;
    const totalSupply          = new BN(data.slice(offset, offset + 8), 'le'); offset += 8;
    const graduated            = data[offset] === 1; offset += 1;
    const bump                 = data[offset]; offset += 1;
    const vaultBump            = data[offset]; offset += 1;
    const createdAt            = new BN(data.slice(offset, offset + 8), 'le');

    const curveState: CurveState = { virtualSolReserves, virtualTokenReserves, realSolReserves, realTokenReserves };

    return {
      pubkey, creator, mint, name, symbol, uri, description,
      virtualSolReserves, virtualTokenReserves,
      realSolReserves, realTokenReserves, totalSupply,
      graduated, bump, vaultBump, createdAt,
      price:       getPrice(curveState),
      marketCapSol: getMarketCapSol(curveState),
      progress:    getGraduationProgress(realSolReserves),
    };
  }
}

export interface BondingCurveAccount {
  pubkey:               PublicKey;
  creator:              PublicKey;
  mint:                 PublicKey;
  name:                 string;
  symbol:               string;
  uri:                  string;
  description:          string;
  virtualSolReserves:   BN;
  virtualTokenReserves: BN;
  realSolReserves:      BN;
  realTokenReserves:    BN;
  totalSupply:          BN;
  graduated:            boolean;
  bump:                 number;
  vaultBump:            number;
  createdAt:            BN;
  price:                BN;
  marketCapSol:         number;
  progress:             number;
}
