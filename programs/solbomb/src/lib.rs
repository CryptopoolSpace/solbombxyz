use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{self, Token, TokenAccount, Mint, MintTo, Transfer};
use anchor_spl::associated_token::AssociatedToken;

declare_id!("SoLBoMb1111111111111111111111111111111111111");

// ── Constants ─────────────────────────────────────────────────────────────────
pub const PLATFORM_FEE_BPS: u64  = 100;           // 1%
pub const GRADUATION_USD:   u64  = 69_000;         // $69k market cap
pub const TOTAL_SUPPLY:     u64  = 1_000_000_000;  // 1 billion tokens
pub const VIRTUAL_SOL:      u64  = 30_000_000_000; // 30 SOL virtual reserves
pub const TOKEN_SEED:       &[u8] = b"token";
pub const CURVE_SEED:       &[u8] = b"curve";
pub const VAULT_SEED:       &[u8] = b"vault";
pub const CONFIG_SEED:      &[u8] = b"config";

#[program]
pub mod solbomb {
    use super::*;

    // ── Initialize platform config ─────────────────────────────────────────
    pub fn initialize(ctx: Context<Initialize>, fee_receiver: Pubkey) -> Result<()> {
        let config = &mut ctx.accounts.config;
        config.authority    = ctx.accounts.authority.key();
        config.fee_receiver = fee_receiver;
        config.fee_bps      = PLATFORM_FEE_BPS;
        config.total_tokens = 0;
        config.total_volume = 0;
        config.bump         = ctx.bumps.config;
        Ok(())
    }

    // ── Create Token + Bonding Curve ───────────────────────────────────────
    pub fn create_token(
        ctx: Context<CreateToken>,
        name:        String,
        symbol:      String,
        uri:         String,   // metadata URI (IPFS)
        description: String,
    ) -> Result<()> {
        require!(name.len() <= 32,   SolBombError::NameTooLong);
        require!(symbol.len() <= 10, SolBombError::SymbolTooLong);
        require!(uri.len() <= 200,   SolBombError::UriTooLong);

        // Init bonding curve state
        let curve = &mut ctx.accounts.curve;
        curve.creator          = ctx.accounts.creator.key();
        curve.mint             = ctx.accounts.mint.key();
        curve.name             = name.clone();
        curve.symbol           = symbol.clone();
        curve.uri              = uri;
        curve.description      = description;
        curve.virtual_sol_reserves  = VIRTUAL_SOL;
        curve.virtual_token_reserves = TOTAL_SUPPLY * 1_000_000; // with decimals
        curve.real_sol_reserves  = 0;
        curve.real_token_reserves = TOTAL_SUPPLY * 1_000_000;
        curve.total_supply       = TOTAL_SUPPLY * 1_000_000;
        curve.graduated          = false;
        curve.bump               = ctx.bumps.curve;
        curve.vault_bump         = ctx.bumps.vault;
        curve.created_at         = Clock::get()?.unix_timestamp;

        // Update config stats
        let config = &mut ctx.accounts.config;
        config.total_tokens += 1;

        emit!(TokenCreated {
            creator: curve.creator,
            mint: curve.mint,
            name,
            symbol,
        });

        Ok(())
    }

    // ── Buy tokens (SOL → Token) ───────────────────────────────────────────
    pub fn buy(
        ctx: Context<Buy>,
        sol_amount: u64,      // SOL to spend (lamports)
        min_tokens: u64,      // slippage protection
    ) -> Result<()> {
        let curve = &mut ctx.accounts.curve;
        require!(!curve.graduated, SolBombError::TokenGraduated);
        require!(sol_amount > 0, SolBombError::ZeroAmount);

        // Calculate fee
        let fee = sol_amount * curve_fee_bps(ctx.accounts.config.fee_bps) / 10_000;
        let sol_in = sol_amount - fee;

        // Bonding curve: constant product formula
        // k = virtual_sol * virtual_tokens
        // tokens_out = virtual_tokens - (k / (virtual_sol + sol_in))
        let k = (curve.virtual_sol_reserves as u128)
            .checked_mul(curve.virtual_token_reserves as u128)
            .ok_or(SolBombError::MathOverflow)?;

        let new_virtual_sol = curve.virtual_sol_reserves
            .checked_add(sol_in)
            .ok_or(SolBombError::MathOverflow)?;

        let new_virtual_tokens = (k / new_virtual_sol as u128) as u64;
        let tokens_out = curve.virtual_token_reserves
            .checked_sub(new_virtual_tokens)
            .ok_or(SolBombError::MathOverflow)?;

        require!(tokens_out >= min_tokens, SolBombError::SlippageExceeded);
        require!(tokens_out <= curve.real_token_reserves, SolBombError::InsufficientTokens);

        // Transfer SOL: buyer → vault
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.buyer.to_account_info(),
                to:   ctx.accounts.vault.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, sol_in)?;

        // Transfer fee: buyer → fee_receiver
        let fee_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.buyer.to_account_info(),
                to:   ctx.accounts.fee_receiver.to_account_info(),
            },
        );
        system_program::transfer(fee_ctx, fee)?;

        // Mint tokens to buyer
        let curve_seeds: &[&[u8]] = &[
            CURVE_SEED,
            curve.mint.as_ref(),
            &[curve.bump],
        ];
        let signer_seeds = &[curve_seeds];

        let mint_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint:      ctx.accounts.mint.to_account_info(),
                to:        ctx.accounts.buyer_token_account.to_account_info(),
                authority: ctx.accounts.curve.to_account_info(),
            },
            signer_seeds,
        );
        token::mint_to(mint_ctx, tokens_out)?;

        // Update curve state
        curve.virtual_sol_reserves    = new_virtual_sol;
        curve.virtual_token_reserves  = new_virtual_tokens;
        curve.real_sol_reserves       = curve.real_sol_reserves.checked_add(sol_in).unwrap();
        curve.real_token_reserves     = curve.real_token_reserves.checked_sub(tokens_out).unwrap();

        // Update volume
        let config = &mut ctx.accounts.config;
        config.total_volume = config.total_volume.saturating_add(sol_amount);

        emit!(TokenBought {
            buyer: ctx.accounts.buyer.key(),
            mint: curve.mint,
            sol_in: sol_amount,
            tokens_out,
            fee,
        });

        // Check graduation
        if should_graduate(curve.real_sol_reserves) {
            curve.graduated = true;
            emit!(TokenGraduated { mint: curve.mint, sol_raised: curve.real_sol_reserves });
        }

        Ok(())
    }

    // ── Sell tokens (Token → SOL) ──────────────────────────────────────────
    pub fn sell(
        ctx: Context<Sell>,
        token_amount: u64,   // tokens to sell
        min_sol:      u64,   // slippage protection
    ) -> Result<()> {
        let curve = &mut ctx.accounts.curve;
        require!(!curve.graduated, SolBombError::TokenGraduated);
        require!(token_amount > 0, SolBombError::ZeroAmount);

        // Bonding curve: constant product
        let k = (curve.virtual_sol_reserves as u128)
            .checked_mul(curve.virtual_token_reserves as u128)
            .ok_or(SolBombError::MathOverflow)?;

        let new_virtual_tokens = curve.virtual_token_reserves
            .checked_add(token_amount)
            .ok_or(SolBombError::MathOverflow)?;

        let new_virtual_sol = (k / new_virtual_sol_from_tokens(new_virtual_tokens) as u128) as u64;
        let sol_out = curve.virtual_sol_reserves
            .checked_sub(new_virtual_sol)
            .ok_or(SolBombError::MathOverflow)?;

        let fee = sol_out * curve_fee_bps(ctx.accounts.config.fee_bps) / 10_000;
        let sol_to_seller = sol_out - fee;

        require!(sol_to_seller >= min_sol, SolBombError::SlippageExceeded);
        require!(sol_to_seller <= curve.real_sol_reserves, SolBombError::InsufficientSol);

        // Burn tokens from seller
        let burn_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            token::Burn {
                mint:      ctx.accounts.mint.to_account_info(),
                from:      ctx.accounts.seller_token_account.to_account_info(),
                authority: ctx.accounts.seller.to_account_info(),
            },
        );
        token::burn(burn_ctx, token_amount)?;

        // Transfer SOL: vault → seller
        let vault_seeds: &[&[u8]] = &[
            VAULT_SEED,
            curve.mint.as_ref(),
            &[curve.vault_bump],
        ];
        let signer_seeds = &[vault_seeds];

        **ctx.accounts.vault.try_borrow_mut_lamports()? -= sol_to_seller + fee;
        **ctx.accounts.seller.try_borrow_mut_lamports()? += sol_to_seller;
        **ctx.accounts.fee_receiver.try_borrow_mut_lamports()? += fee;

        // Update curve
        curve.virtual_sol_reserves   = new_virtual_sol;
        curve.virtual_token_reserves = new_virtual_tokens;
        curve.real_sol_reserves      = curve.real_sol_reserves.checked_sub(sol_out).unwrap();
        curve.real_token_reserves    = curve.real_token_reserves.checked_add(token_amount).unwrap();

        emit!(TokenSold {
            seller: ctx.accounts.seller.key(),
            mint: curve.mint,
            tokens_in: token_amount,
            sol_out: sol_to_seller,
            fee,
        });

        Ok(())
    }
}

// ── Math helpers ──────────────────────────────────────────────────────────────

fn curve_fee_bps(bps: u64) -> u64 { bps }

fn new_virtual_sol_from_tokens(tokens: u64) -> u64 { tokens }

fn should_graduate(real_sol: u64) -> bool {
    // Graduate at ~85 SOL (~$69k at ~$800/SOL)
    real_sol >= 85_000_000_000
}

// ── Account structs ───────────────────────────────────────────────────────────

#[account]
pub struct Config {
    pub authority:    Pubkey,   // 32
    pub fee_receiver: Pubkey,   // 32
    pub fee_bps:      u64,      // 8
    pub total_tokens: u64,      // 8
    pub total_volume: u64,      // 8
    pub bump:         u8,       // 1
}
impl Config { pub const LEN: usize = 8 + 32 + 32 + 8 + 8 + 8 + 1; }

#[account]
pub struct BondingCurve {
    pub creator:                Pubkey,   // 32
    pub mint:                   Pubkey,   // 32
    pub name:                   String,   // 4+32
    pub symbol:                 String,   // 4+10
    pub uri:                    String,   // 4+200
    pub description:            String,   // 4+200
    pub virtual_sol_reserves:   u64,      // 8
    pub virtual_token_reserves: u64,      // 8
    pub real_sol_reserves:      u64,      // 8
    pub real_token_reserves:    u64,      // 8
    pub total_supply:           u64,      // 8
    pub graduated:              bool,     // 1
    pub bump:                   u8,       // 1
    pub vault_bump:             u8,       // 1
    pub created_at:             i64,      // 8
}
impl BondingCurve {
    pub const LEN: usize = 8 + 32 + 32 + (4+32) + (4+10) + (4+200) + (4+200) + 8+8+8+8+8+1+1+1+8;

    pub fn current_price_lamports(&self) -> u64 {
        if self.virtual_token_reserves == 0 { return 0; }
        self.virtual_sol_reserves * 1_000_000 / self.virtual_token_reserves
    }

    pub fn market_cap_sol(&self) -> u64 {
        let price = self.current_price_lamports();
        (self.total_supply as u128 * price as u128 / 1_000_000) as u64
    }
}

// ── Contexts ──────────────────────────────────────────────────────────────────

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init, payer = authority,
        space = Config::LEN,
        seeds = [CONFIG_SEED], bump
    )]
    pub config: Account<'info, Config>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CreateToken<'info> {
    #[account(
        init, payer = creator,
        space = BondingCurve::LEN,
        seeds = [CURVE_SEED, mint.key().as_ref()], bump
    )]
    pub curve: Account<'info, BondingCurve>,

    #[account(
        init, payer = creator,
        mint::decimals = 6,
        mint::authority = curve,
    )]
    pub mint: Account<'info, Mint>,

    /// CHECK: SOL vault PDA
    #[account(
        mut,
        seeds = [VAULT_SEED, mint.key().as_ref()], bump
    )]
    pub vault: UncheckedAccount<'info>,

    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,

    #[account(mut)]
    pub creator: Signer<'info>,

    pub token_program:  Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent:           Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct Buy<'info> {
    #[account(
        mut,
        seeds = [CURVE_SEED, mint.key().as_ref()],
        bump = curve.bump
    )]
    pub curve: Account<'info, BondingCurve>,

    #[account(mut)]
    pub mint: Account<'info, Mint>,

    /// CHECK: vault
    #[account(mut, seeds = [VAULT_SEED, mint.key().as_ref()], bump = curve.vault_bump)]
    pub vault: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = buyer,
        associated_token::mint = mint,
        associated_token::authority = buyer,
    )]
    pub buyer_token_account: Account<'info, TokenAccount>,

    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,

    /// CHECK: fee receiver
    #[account(mut, address = config.fee_receiver)]
    pub fee_receiver: UncheckedAccount<'info>,

    #[account(mut)]
    pub buyer: Signer<'info>,

    pub token_program:           Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program:          Program<'info, System>,
}

#[derive(Accounts)]
pub struct Sell<'info> {
    #[account(
        mut,
        seeds = [CURVE_SEED, mint.key().as_ref()],
        bump = curve.bump
    )]
    pub curve: Account<'info, BondingCurve>,

    #[account(mut)]
    pub mint: Account<'info, Mint>,

    /// CHECK: vault
    #[account(mut, seeds = [VAULT_SEED, mint.key().as_ref()], bump = curve.vault_bump)]
    pub vault: UncheckedAccount<'info>,

    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = seller,
    )]
    pub seller_token_account: Account<'info, TokenAccount>,

    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,

    /// CHECK: fee receiver
    #[account(mut, address = config.fee_receiver)]
    pub fee_receiver: UncheckedAccount<'info>,

    #[account(mut)]
    pub seller: Signer<'info>,

    pub token_program:  Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

// ── Events ────────────────────────────────────────────────────────────────────

#[event] pub struct TokenCreated { pub creator: Pubkey, pub mint: Pubkey, pub name: String, pub symbol: String }
#[event] pub struct TokenBought  { pub buyer: Pubkey, pub mint: Pubkey, pub sol_in: u64, pub tokens_out: u64, pub fee: u64 }
#[event] pub struct TokenSold    { pub seller: Pubkey, pub mint: Pubkey, pub tokens_in: u64, pub sol_out: u64, pub fee: u64 }
#[event] pub struct TokenGraduated { pub mint: Pubkey, pub sol_raised: u64 }

// ── Errors ────────────────────────────────────────────────────────────────────

#[error_code]
pub enum SolBombError {
    #[msg("Token name too long")] NameTooLong,
    #[msg("Symbol too long")] SymbolTooLong,
    #[msg("URI too long")] UriTooLong,
    #[msg("Token already graduated to DEX")] TokenGraduated,
    #[msg("Amount must be greater than zero")] ZeroAmount,
    #[msg("Slippage exceeded")] SlippageExceeded,
    #[msg("Insufficient tokens in curve")] InsufficientTokens,
    #[msg("Insufficient SOL in vault")] InsufficientSol,
    #[msg("Math overflow")] MathOverflow,
}
