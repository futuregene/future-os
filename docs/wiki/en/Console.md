# Console Overview

After signing in to the [FutureOS Console](https://future-os.cn), the left side shows the feature menu and the right side shows the content area.

| Menu | Purpose |
|---|---|
| Overview | Account balance, this month's spending, call count, spending trend chart |
| API Key Management | Create and manage API Keys to connect your own systems |
| Top Up | Online payment, corporate bank transfer, redemption code |
| Team Management | Create teams, invite members, allocate budgets, adjust billing order |
| Billing Records | View itemized spending on models and tools |

The top navigation bar lets you browse the **Skill Market**, **Model List**, and **Docs** at any time.

---

# Sign Up & Sign In

## Sign Up

1. Click **Free Trial** in the top-right corner.
2. Enter your email and click **Send Code**, then enter the 6-digit code you receive (valid for 60 seconds).
3. Set a password: 8–20 characters, must include letters and numbers.
4. Confirm the password, agree to the terms, and click **Create Account**.

## Sign In

Enter your email and password, then click **Sign In**. Check **Remember me** to stay signed in for 7 days.

## Forgot Password

Click **Forgot password?** on the sign-in page → enter your email → get the code (valid for 10 minutes) → set a new password.

---

# API Key Management

API Keys let you connect FutureOS models to your own agent, script, or application.

## Create

1. Left menu → **API Key Management** → click **+ Create API Key**.
2. Enter a name, such as "Research Assistant" or "Default App".
3. Click **Create**.

> ⚠️ **The key is shown only once, at creation. Copy and save it immediately.** It cannot be viewed again after you close the dialog.

## Use

Include it in the HTTP request header:

```
Authorization: Bearer <your API Key>
```

## Manage

- **Edit** — change the name.
- **Delete** — takes effect immediately and cannot be undone.

---

# Top Up

Your balance never expires. Three top-up methods are supported.

## Online Payment

1. Left menu → **Top Up** → select **Online Payment**.
2. Choose **WeChat Pay** or **Alipay** and enter the amount.
3. The system automatically calculates the total including the service fee; click **Pay** to complete the payment.

> The service fee (10%) covers cross-model API call costs, payment-channel processing fees, and platform operating costs.

## Corporate Bank Transfer

1. Select the **Corporate Bank Transfer** tab and transfer to the following account:

| Item | Details |
|---|---|
| Account name | 西湖未来基因科技（杭州）有限公司 |
| Bank | 招商银行股份有限公司杭州城西支行 (China Merchants Bank) |
| Account number | 5719 1950 8310 201 |

2. After transferring, contact support to confirm. Receipts submitted on business days between 9:00–18:00 are reviewed and credited **within 2 hours**.

## Redemption Code

Enter a redemption code in the format `FUTR-XXXX-XXXX-XXXX-XXXX`, and the amount is credited directly to your balance.

## Viewing Balance & Records

- Your current balance is shown at the top of the Overview page or the Top Up page.
- The bottom of the Top Up page shows **Top-up Records**, filterable by transaction number, date, and payment method.

---

# Billing Records

View itemized spending for each model call and tool call.

The top of the page shows three stat cards: **Total Spending**, **Model Spending**, and **Tool Spending**.

## Model Spending

Records each model call:

| Field | Description |
|---|---|
| Time | Time of the call |
| Model | Name of the model used |
| API Key | The key used (shows the name; hover to see the full ID) |
| Input Tokens / Output Tokens | Number of input tokens and generated tokens |
| Input / Output / Cache | Input cost, output cost, cache read/write cost |
| Total Charge | Total cost of this call |
| Charged To | Personal account / team name |
| Status | Charge succeeded / failed |

## Tool Spending

Records each tool call: time, tool name, API Key, total charge, charged-to, and status.

Both tables can be filtered by **date range**; click **Query** to refresh.

---

# Team Management

## Invite Members

1. Left menu → **Team Management** → click **+ Invite Member**.
2. Enter the member's email address.
3. Choose a budget rule:

| Rule | Description |
|---|---|
| **Fixed quota** | Allocated once; the balance is frozen when the validity period ends |
| **Periodic reset** | Automatically resets to the fixed quota every day / week / month / quarter / year |

4. Set the amount and validity period, then click **Send Invitation**.

## Manage Members

In the **Member List** tab:

- **Adjust** — change a member's quota amount, reset cycle, or validity period.
- **Remove** — remove the member from the team.
- **Cancel** — withdraw an invitation the recipient has not yet accepted.

## Teams I've Joined

In the **Teams I've Joined** tab:

- Handle received invitations: **Accept** or **Decline**.
- Declined invitations can be **deleted** from the record.
- Click **Leave Team** to leave a team you've joined.

## Billing Order

The bottom of the Team page lists the priority of your personal balance and each team quota. Click the **↑ ↓** buttons to reorder — charges consume each source in this order, automatically switching to the next source once the previous one is used up.

---

# Skill Billing Rules

**Skills themselves are free.** However, if a skill calls a tool provided by FutureOS, it is billed according to the following rules:

| Tool | Description | Billing rule |
|---|---|---|
| `web_search` | Call a search engine | ¥0.01 per call |
| `search_paper` | Search academic papers and extract information | ¥0.05 per call |
| `get_paper` | Get full text by PMID/DOI | ¥0.01 per call |
| `fetch_url` | Fetch URL content | ¥0.01 per call |
| `parse_doc` | Convert PDF/Word to Markdown | ¥0.01 per page |
| `browser` | Open a local web search | Free |
| `image_gen` | Text-to-image | Billed at the model's actual rate |
| `image_edit` | Image editing | Billed at the model's actual rate |
| `read_image` | Image recognition / OCR | Billed at the model's actual rate |

Tool spending details are available under **Billing Records** → the **Tool Spending** tab.
