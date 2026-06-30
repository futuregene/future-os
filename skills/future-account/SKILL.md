---
version: 1.0.0
name: future-account
description: View Future account profile and balance, create credit recharge orders via Future CLI. Use when the user asks about their account info, credits balance, remaining credits, wants to top up or recharge, or asks "how much credit do I have left".
allowed-tools: Bash(future:*)
---
version: 1.0.0

> **Authentication is automatic.** The `future` CLI reads credentials from `~/.future/agent/auth.json`. You do NOT need to find, configure, or pass API keys — just call the commands below.

# Account

## When to use this skill

Load this skill when the user asks to:
- Check their account profile or user information
- View their credit balance or remaining credits
- Create a recharge / top-up order
- Ask "how is my account" or "show me my balance"

## Commands

### View account profile

```bash
future account profile
```

Returns: user ID, email, email verification status, registration date.

### View credit balance

```bash
future account balance
```

Returns: balance in credits. Use `--json` for machine-readable output.

### Create recharge order

```bash
future account recharge --amount 10 --channel alipay
```

- `--amount <yuan>` (required): recharge amount in CNY (e.g., `10` = ¥10.00). Range: 1–10,000.
- `--channel <alipay|wechat>` (required): payment channel.

Returns: order number, amount, channel, status (`pending`), pay URL, expiration time.

## Pricing

All account commands are **free** (zero credits). They do not consume any balance.

## Notes

- Account data (user profile, wallet balance) is per-user and identified by the API key.
- Recharge orders start as `pending` and must be completed through the payment provider. The CLI does not handle payment execution — it only creates the order.
- The recharge amount unit is **yuan** (CNY), not cents.
