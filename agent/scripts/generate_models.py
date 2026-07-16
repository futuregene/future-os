#!/usr/bin/env python3
"""
generate_models.py - Fetches model data from external APIs and generates models_generated.rs

This mirrors Go internal/modelregistry/generate_models.go

Usage:
    python3 scripts/generate_models.py

Data sources:
    - https://models.dev/api.json → all providers except openrouter/vercel
    - https://openrouter.ai/api/v1/models → OpenRouter
    - https://ai-gateway.vercel.sh/v1/models → Vercel AI Gateway

The script filters to tool-capable models only and applies provider-specific
configurations matching Go's generate-models.go exactly.
"""

import json
import urllib.request
import urllib.error
from datetime import datetime
from typing import Dict, Any, List, Optional

# Provider base URLs — comprehensive mapping extracted from the Go model registry.
# Keyed by the provider slug used in models.dev/api.json.
PROVIDER_BASE_URLS = {
    "openai": "https://api.openai.com/v1",
    "openai-codex": "https://api.openai.com/v1",
    "azure-openai-responses": "https://YOUR_RESOURCE.openai.azure.com/openai/v1",
    "anthropic": "https://api.anthropic.com/v1",
    "google": "https://generativelanguage.googleapis.com/v1beta/openai",
    "google-vertex": "https://LOCATION-aiplatform.googleapis.com/v1beta1/projects/PROJECT_ID/locations/LOCATION/endpoints/openapi",
    "deepseek": "https://api.deepseek.com/v1",
    "mistral": "https://api.mistral.ai/v1",
    "amazon-bedrock": "https://bedrock-runtime.us-east-1.amazonaws.com",
    "openrouter": "https://openrouter.ai/api/v1",
    "vercel-ai-gateway": "https://ai-gateway.vercel.sh",
    "vercel-ai": "https://ai-gateway.vercel.sh/v1",
    "xai": "https://api.x.ai/v1",
    "groq": "https://api.groq.com/openai/v1",
    "cerebras": "https://api.cerebras.ai/v1",
    "huggingface": "https://api-inference.huggingface.co/v1",
    "cloudflare-workers-ai": "https://api.cloudflare.com/client/v4/accounts",
    "moonshotai": "https://api.moonshot.ai/v1",
    "moonshotai-cn": "https://api.moonshot.cn/v1",
    "minimax": "https://api.minimax.io/anthropic",
    "minimax-cn": "https://api.minimaxi.com/anthropic",
    "zai": "https://api.z.ai/api/paas/v4",
    "zhipuai": "https://open.bigmodel.cn/api/paas/v4",
    "github-copilot": "https://models.githubcopilot.com/v1",
    "xiaomi": "https://api.xiaomimimo.com/anthropic",
    "xiaomi-token-plan-ams": "https://token-plan-ams.xiaomimimo.com/anthropic",
    "xiaomi-token-plan-cn": "https://token-plan-cn.xiaomimimo.com/anthropic",
    "xiaomi-token-plan-sgp": "https://token-plan-sgp.xiaomimimo.com/anthropic",
    "kimi-coding": "https://api.kimi.com/coding",
    "opencode": "https://opencode.ai/zen",
    "opencode-go": "https://opencode.ai/zen/go",
}

# Provider API types (same as Go)
PROVIDER_APIS = {
    "openai": "chat",
    "anthropic": "chat",
    "google": "chat",
    "deepseek": "chat",
    "mistral": "chat",
    "cohere": "chat",
    "meta": "chat",
    "amazon-bedrock": "chat",
    "openrouter": "chat",
    "vercel-ai": "chat",
}


def fetch_json(url: str, timeout: int = 30) -> Optional[Dict]:
    """Fetch JSON from URL with timeout."""
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "future-agent/1.0"})
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read().decode())
    except Exception as e:
        print(f"Warning: Failed to fetch {url}: {e}")
        return None


def process_models_dev(data: Dict) -> List[Dict]:
    """Process models.dev API response."""
    models = []
    
    # Bedrock exclusions (same as Go/pi)
    bedrock_exclusions = [
        "ai21.jamba",
        "mistral.mistral-7b-instruct-v0",
    ]
    
    for provider_name, provider_data in data.items():
        # Skip openrouter and vercel (fetched separately)
        if provider_name in ("openrouter", "vercel-ai"):
            continue
        # Priority: models.dev `api` field → static PROVIDER_BASE_URLS fallback.
        api_url = (provider_data.get("api") or "").rstrip("/")
        base_url = api_url or PROVIDER_BASE_URLS.get(provider_name, "")
        if not base_url:
            continue

        for model_id, model in provider_data.get("models", {}).items():
            # Only include models that support tool calling
            if not model.get("tool_call", False):
                continue
            
            # Apply Bedrock exclusions (same as Go)
            if provider_name == "amazon-bedrock":
                excluded = False
                for prefix in bedrock_exclusions:
                    if model_id.startswith(prefix):
                        excluded = True
                        break
                if excluded:
                    continue
            
            name = model.get("name") or model_id
            reasoning = model.get("reasoning", False)
            modalities = model.get("modalities", {}).get("input", ["text"])
            
            limit = model.get("limit", {})
            context_window = limit.get("context", 4096)
            max_tokens = limit.get("output", 4096)
            
            cost = model.get("cost", {})
            
            models.append({
                "id": model_id,
                "name": name,
                "provider": provider_name,
                "api": PROVIDER_APIS.get(provider_name, "chat"),
                "base_url": base_url,
                "reasoning": reasoning,
                "input": modalities,
                "context_window": context_window,
                "max_tokens": max_tokens,
                "cost_input": cost.get("input", 0.0),
                "cost_output": cost.get("output", 0.0),
                "cost_cache_read": cost.get("cache_read", 0.0),
                "cost_cache_write": cost.get("cache_write", 0.0),
                "compat_json": "{}",
                "tlm_json": "{}",
                "headers_json": "{}",
            })
    
    return models


def process_openrouter(data: Dict) -> List[Dict]:
    """Process OpenRouter API response."""
    models = []
    
    for model in data.get("data", []):
        model_id = model.get("id", "")
        if not model_id:
            continue
            
        # Check if supports tools
        supported_params = model.get("supported_parameters", [])
        if "tools" not in supported_params:
            continue
            
        name = model.get("name", model_id)
        provider = model.get("id", "").split("/")[0] if "/" in model_id else "openrouter"
        context_window = model.get("context_length", 4096)
        
        pricing = model.get("pricing", {})
        
        models.append({
            "id": model_id,
            "name": name,
            "provider": "openrouter",
            "api": "chat",
            "base_url": PROVIDER_BASE_URLS["openrouter"],
            "reasoning": False,  # OpenRouter doesn't expose this directly
            "input": ["text"],  # Assume text only
            "context_window": context_window,
            "max_tokens": min(context_window, 32768),  # Conservative estimate
            "cost_input": float(pricing.get("input", 0)),
            "cost_output": float(pricing.get("output", 0)),
            "cost_cache_read": 0.0,
            "cost_cache_write": 0.0,
            "compat_json": "{}",
            "tlm_json": "{}",
            "headers_json": "{}",
        })
    
    return models


def process_vercel_ai(data: Dict) -> List[Dict]:
    """Process Vercel AI Gateway API response.

    The API now returns OpenAI-compatible format: {"data": [...], "object": "list"}.
    Each model has: id, name, owned_by, context_window, max_tokens, pricing, etc.
    Vercel models are all assumed to support tool calling (the gateway proxies
    them with tool support).
    """
    models = []

    for model in data.get("data", []):
        if not isinstance(model, dict):
            continue

        model_id = model.get("id", "")
        if not model_id:
            continue

        name = model.get("name") or model_id
        context_window = model.get("context_window", 4096)
        max_tokens = model.get("max_tokens", 4096)
        pricing = model.get("pricing", {})

        models.append({
            "id": model_id,
            "name": name,
            "provider": "vercel-ai",
            "api": "chat",
            "base_url": PROVIDER_BASE_URLS["vercel-ai"],
            "reasoning": False,
            "input": ["text"],
            "context_window": context_window,
            "max_tokens": max_tokens,
            "cost_input": float(pricing.get("input", 0)),
            "cost_output": float(pricing.get("output", 0)),
            "cost_cache_read": 0.0,
            "cost_cache_write": 0.0,
            "compat_json": "{}",
            "tlm_json": "{}",
            "headers_json": "{}",
        })

    return models


def generate_rust_code(models: List[Dict], timestamp: str) -> str:
    """Generate Rust code from model list."""
    
    # Sort by provider then id
    models.sort(key=lambda m: (m["provider"], m["id"]))
    
    # Count providers
    providers = set(m["provider"] for m in models)
    
    output = f"""//! Generated model catalog — auto-generated by generate_models.py
//!
//! Generated at {timestamp}
//! {len(models)} models across {len(providers)} providers
//!
//! Run `python3 scripts/generate_models.py` to regenerate.

use serde::{{Deserialize, Serialize}};

/// Model mirrors the Go types.Model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {{
    pub id: String,
    pub name: String,
    pub provider: String,
    pub api: String,
    pub base_url: String,
    pub reasoning: bool,
    pub input: Vec<String>,
    pub context_window: i32,
    pub max_tokens: i32,
    pub cost_input: f64,
    pub cost_output: f64,
    pub cost_cache_read: f64,
    pub cost_cache_write: f64,
    pub compat_json: String,
    pub tlm_json: String,
    pub headers_json: String,
}}

/// INIT_BUILTIN_MODELS returns the complete built-in model catalog.
pub fn init_builtin_models() -> Vec<Model> {{
    vec![
"""
    
    current_provider = None
    for m in models:
        if current_provider != m["provider"]:
            if current_provider is not None:
                output += "        // ── end -->\n"
            current_provider = m["provider"]
            output += f"        // ── {m['provider']} -->\n"
        
        input_vec = "vec![" + ", ".join(f'String::from("{i}")' for i in m["input"]) + "]"
        
        output += f"""        Model {{
            id: "{m['id']}".into(),
            name: "{m['name']}".into(),
            provider: "{m['provider']}".into(),
            api: "{m['api']}".into(),
            base_url: "{m['base_url']}".into(),
            reasoning: {str(m['reasoning']).lower()},
            input: {input_vec},
            context_window: {m['context_window']},
            max_tokens: {m['max_tokens']},
            cost_input: {m['cost_input']},
            cost_output: {m['cost_output']},
            cost_cache_read: {m['cost_cache_read']},
            cost_cache_write: {m['cost_cache_write']},
            compat_json: String::from("{{}}"),
            tlm_json: String::from("{{}}"),
            headers_json: String::from("{{}}"),
        }},
"""
    
    if current_provider is not None:
        output += "        // ── end -->\n"
    
    output += """    ]
}
"""
    return output


def generate_wiki_docs(models: List[Dict], timestamp: str):
    """Generate docs/wiki/{en,zh}/models.md from the model list."""

    script_dir = __import__('os').path.dirname(__import__('os').path.abspath(__file__))
    repo_root = __import__('os').path.normpath(__import__('os').path.join(script_dir, "../.."))
    wiki_en = __import__('os').path.join(repo_root, "docs/wiki/en")
    wiki_zh = __import__('os').path.join(repo_root, "docs/wiki/zh")
    __import__('os').makedirs(wiki_en, exist_ok=True)
    __import__('os').makedirs(wiki_zh, exist_ok=True)

    # ── Aggregate by provider ──────────────────────────────────────────────
    providers: Dict[str, List[Dict]] = {}
    for m in models:
        providers.setdefault(m["provider"], []).append(m)

    # ── Helpers ────────────────────────────────────────────────────────────
    def fmt_num(n):
        if n >= 1_000_000_000:
            return f"{n / 1_000_000_000:.0f}B"
        if n >= 1_000_000:
            return f"{n / 1_000_000:.0f}M"
        if n >= 1_000:
            return f"{n / 1_000:.0f}K"
        return str(n)

    def image_support(m):
        return "✅" if "image" in m.get("input", []) else "—"

    def reasoning_support(m):
        return "✅" if m.get("reasoning", False) else "—"

    provider_names_cn = {
        "openai": "OpenAI",
        "anthropic": "Anthropic",
        "google": "Google",
        "deepseek": "DeepSeek",
        "mistral": "Mistral",
        "cohere": "Cohere",
        "meta": "Meta",
        "amazon-bedrock": "Amazon Bedrock",
        "openrouter": "OpenRouter",
        "vercel-ai": "Vercel AI Gateway",
    }

    provider_head = "| Provider | Models |"
    provider_sep = "|---|---|"
    en_summary_rows = []
    zh_summary_rows = []
    for pname in sorted(providers.keys(), key=lambda p: p.lower()):
        pmodels = providers[pname]
        label = provider_names_cn.get(pname, pname)
        en_summary_rows.append(f"| {label} | {len(pmodels)} |")
        zh_summary_rows.append(f"| {label} | {len(pmodels)} |")

    # ── Build Markdown (English) ───────────────────────────────────────────
    en = f"# Built-in Model Catalog\n\n{len(models)} models across {len(providers)} providers.\n\n"
    en += f"## Provider Summary\n\n{provider_head}\n{provider_sep}\n"
    en += "\n".join(en_summary_rows)
    en += "\n\n---\n\n## Per-Provider Details\n\n"

    for pname in sorted(providers.keys()):
        pmodels = sorted(providers[pname], key=lambda m: -m["context_window"])
        provider_label = provider_names_cn.get(pname, pname)
        # Collect unique base URLs for this provider
        base_urls = sorted(set(m["base_url"] for m in pmodels if m["base_url"]))
        base_url_str = ", ".join(f"`{u}`" for u in base_urls) if base_urls else "—"
        en += f"### {provider_label}\n\n"
        en += f"**Base URL:** {base_url_str}\n\n"
        en += "| Model ID | Name | Context | Max Output | Image | Reasoning |\n"
        en += "|---|---|---|---|---|---|\n"
        for m in pmodels:
            short_id = m["id"].split("/")[-1] if "/" in m["id"] else m["id"]
            if len(short_id) > 48:
                short_id = short_id[:45] + "..."
            en += f"| `{short_id}` | {m['name']} | {fmt_num(m['context_window'])} | {fmt_num(m['max_tokens'])} | {image_support(m)} | {reasoning_support(m)} |\n"
        en += "\n"

    # ── Build Markdown (Chinese) ───────────────────────────────────────────
    zh = f"# 内置模型目录\n\n{len(models)} 个模型，覆盖 {len(providers)} 个 Provider。\n\n"
    zh += f"## Provider 概览\n\n{provider_head}\n{provider_sep}\n"
    zh += "\n".join(zh_summary_rows)
    zh += "\n\n---\n\n## 各 Provider 详情\n\n"

    for pname in sorted(providers.keys()):
        pmodels = sorted(providers[pname], key=lambda m: -m["context_window"])
        provider_label = provider_names_cn.get(pname, pname)
        base_urls = sorted(set(m["base_url"] for m in pmodels if m["base_url"]))
        base_url_str = ", ".join(f"`{u}`" for u in base_urls) if base_urls else "—"
        zh += f"### {provider_label}\n\n"
        zh += f"**Base URL:** {base_url_str}\n\n"
        zh += "| 模型 ID | 名称 | 上下文 | 最大输出 | 图像 | 推理 |\n"
        zh += "|---|---|---|---|---|---|\n"
        for m in pmodels:
            short_id = m["id"].split("/")[-1] if "/" in m["id"] else m["id"]
            if len(short_id) > 48:
                short_id = short_id[:45] + "..."
            zh += f"| `{short_id}` | {m['name']} | {fmt_num(m['context_window'])} | {fmt_num(m['max_tokens'])} | {image_support(m)} | {reasoning_support(m)} |\n"
        zh += "\n"

    # ── Write ──────────────────────────────────────────────────────────────
    for path, content in [(f"{wiki_en}/Models.md", en), (f"{wiki_zh}/Models.md", zh)]:
        with open(path, "w") as f:
            f.write(content)
        print(f"  Written {path} ({len(content):,} bytes)")


def main():
    print("Fetching models from external APIs...")
    
    all_models = []
    
    # 1. Fetch from models.dev
    print("  - fetching models.dev...")
    data = fetch_json("https://models.dev/api.json")
    if data:
        models = process_models_dev(data)
        print(f"    found {len(models)} models from models.dev")
        all_models.extend(models)
    
    # 2. Fetch from OpenRouter
    print("  - fetching openrouter.ai...")
    data = fetch_json("https://openrouter.ai/api/v1/models")
    if data:
        models = process_openrouter(data)
        print(f"    found {len(models)} models from openrouter")
        all_models.extend(models)
    
    # 3. Fetch from Vercel AI Gateway
    print("  - fetching ai-gateway.vercel.sh...")
    data = fetch_json("https://ai-gateway.vercel.sh/v1/models")
    if data:
        models = process_vercel_ai(data)
        print(f"    found {len(models)} models from vercel")
        all_models.extend(models)
    
    # Sort and dedupe by id.  Known providers (ones with a non-empty base URL)
    # come first so their models win over reseller/aggregator copies of the
    # same model ID.
    def provider_rank(m):
        has_url = 1 if m.get("base_url") else 0
        return -has_url  # negative so known providers sort first

    all_models.sort(key=provider_rank)

    seen = set()
    unique_models = []
    for m in all_models:
        if m["id"] not in seen:
            seen.add(m["id"])
            unique_models.append(m)
    
    print(f"\nTotal unique models: {len(unique_models)}")
    
    # Generate Rust code
    timestamp = datetime.now().strftime("%Y-%m-%dT%H:%M:%S%z")
    rust_code = generate_rust_code(unique_models, timestamp)
    
    # Write to file
    output_path = "src/models/generated/mod.rs"
    with open(output_path, "w") as f:
        f.write(rust_code)
    
    print(f"\nWritten to {output_path}")

    # Generate wiki docs
    print("\nGenerating wiki docs...")
    generate_wiki_docs(unique_models, timestamp)

    # Also compare with Go's generated file
    go_path = "../internal/modelregistry/models_generated.go"
    try:
        with open(go_path, "r") as f:
            go_content = f.read()
        
        # Extract model count from Go file
        import re
        go_models = re.findall(r'mk\("([^"]+)"', go_content)
        print(f"\nGo file has {len(go_models)} models")
        
        rust_models = [m["id"] for m in unique_models]
        
        # Find differences
        go_set = set(go_models)
        rust_set = set(rust_models)
        
        only_in_go = go_set - rust_set
        only_in_rust = rust_set - go_set
        
        if only_in_go:
            print(f"\nModels only in Go ({len(only_in_go)}): {list(only_in_go)[:5]}...")
        if only_in_rust:
            print(f"\nModels only in Rust ({len(only_in_rust)}): {list(only_in_rust)[:5]}...")
        
        if not only_in_go and not only_in_rust:
            print("\n✅ Model lists match!")
            
    except FileNotFoundError:
        print(f"\nGo file not found at {go_path}, skipping comparison")


def parse_rust_models(rust_path: str) -> List[Dict]:
    """Parse existing generated Rust code back into model dicts for wiki-only mode."""
    import re

    with open(rust_path) as f:
        content = f.read()

    # Split by indented "Model {" (instances inside init_builtin_models, not the struct def)
    blocks = [b for b in content.split("\n        Model {")][1:]  # first is before first Model

    models = []
    for block in blocks:
        # Find matching closing brace (rough — sufficient for generated code)
        depth = 1
        end = 0
        for i, ch in enumerate(block):
            if ch == '{':
                depth += 1
            elif ch == '}':
                depth -= 1
                if depth == 0:
                    end = i
                    break
        block = block[:end]

        def field(name: str) -> Optional[str]:
            m = re.search(rf'{name}:\s*(.+?),?\s*$', block, re.MULTILINE)
            return m.group(1).rstrip(',') if m else None

        def str_val(name: str) -> str:
            v = field(name)
            if v is None:
                return ""
            # "value".into() (including empty: "".into())
            m = re.match(r'"([^"]*)"\.into\(\)', v)
            if m:
                return m.group(1)
            return v.strip('"')

        def num_val(name: str) -> float:
            v = field(name)
            return float(v) if v else 0.0

        def bool_val(name: str) -> bool:
            v = field(name)
            return v == "true" if v else False

        def modalities():
            v = field("input")
            if not v:
                return ["text"]
            return re.findall(r'String::from\("(\w+)"\)', v)

        models.append({
            "id": str_val("id"),
            "name": str_val("name"),
            "provider": str_val("provider"),
            "api": "chat",
            "base_url": str_val("base_url"),
            "reasoning": bool_val("reasoning"),
            "input": modalities(),
            "context_window": int(num_val("context_window")),
            "max_tokens": int(num_val("max_tokens")),
            "cost_input": num_val("cost_input"),
            "cost_output": num_val("cost_output"),
            "cost_cache_read": num_val("cost_cache_read"),
            "cost_cache_write": num_val("cost_cache_write"),
            "compat_json": "{}",
            "tlm_json": "{}",
            "headers_json": "{}",
        })

    return models


if __name__ == "__main__":
    import sys

    if "--wiki-only" in sys.argv:
        rust_path = "src/models/generated/mod.rs"
        print(f"Parsing existing models from {rust_path}...")
        models = parse_rust_models(rust_path)
        print(f"  Found {len(models)} models")

        timestamp = datetime.now().strftime("%Y-%m-%dT%H:%M:%S%z")
        print("Generating wiki docs...")
        generate_wiki_docs(models, timestamp)
        print("Done.")
    else:
        main()
