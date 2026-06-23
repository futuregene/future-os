---
name: future-account
description: View Future account profile and balance, create credit recharge orders via Future CLI tools. Use when the user asks about their account info, credits balance, remaining credits, wants to top up or recharge, or asks "how much credit do I have left".
allowed-tools: Bash(future:*)
---

> **Authentication is automatic.** The `future` CLI reads credentials from `~/.future/agent/auth.json`. You do NOT need to find, configure, or pass API keys — just call the tools below.

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
future tools call account_profile
```

Returns: user ID, email, email verification status, registration date.

### View credit balance

```bash
future tools call account_balance
```

Returns: `balance_credits` in internal credit units.

### Create recharge order

```bash
future tools call account_recharge --args '{"amount_cents": 1000, "channel": "alipay"}'
```

- `amount_cents` (required): recharge amount in CNY cents. 1000 = ¥10.00. Range: 100–1,000,000.
- `channel` (required): `"alipay"` or `"wechat"`.

Returns: order number, amount, channel, status (`pending`), expiration time.

## Pricing

All account tools are **free** (zero credits). They do not consume any balance.

## Notes

- Account data (user profile, wallet balance) is per-user and identified by the API key.
- Recharge orders start as `pending` and must be completed through the payment provider. The CLI does not handle payment execution — it only creates the order.
- Use `--json` flag with `future tools list` to see all available tools including account tools.
