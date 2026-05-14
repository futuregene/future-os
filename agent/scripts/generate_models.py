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

# Provider base URLs (same as Go)
PROVIDER_BASE_URLS = {
    "openai": "https://api.openai.com/v1",
    "anthropic": "https://api.anthropic.com",
    "google": "https://generativelanguage.googleapis.com/v1beta",
    "deepseek": "https://api.deepseek.com",
    "mistral": "https://api.mistral.ai/v1",
    "cohere": "https://api.cohere.ai/v1",
    "meta": "https://api.llama-cloud.ai/v1",
    "amazon-bedrock": "https://bedrock-runtime.us-east-1.amazonaws.com",
    "openrouter": "https://openrouter.ai/api/v1",
    "vercel-ai": "https://ai-gateway.vercel.sh/v1",
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
            
        base_url = PROVIDER_BASE_URLS.get(provider_name, "")
        
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
    """Process Vercel AI Gateway API response."""
    models = []
    
    for provider, provider_models in data.items():
        if not isinstance(provider_models, dict):
            continue
            
        for model_id, model in provider_models.items():
            if not isinstance(model, dict):
                continue
                
            # Check if supports tools
            capabilities = model.get("capabilities", {})
            if not capabilities.get("tools", False):
                continue
                
            name = model.get("name", model_id)
            context_window = model.get("contextWindow", model.get("context_length", 4096))
            
            pricing = model.get("pricing", {})
            
            models.append({
                "id": model_id,
                "name": name,
                "provider": "vercel-ai",
                "api": "chat",
                "base_url": PROVIDER_BASE_URLS["vercel-ai"],
                "reasoning": False,
                "input": model.get("modalities", ["text"]),
                "context_window": context_window,
                "max_tokens": model.get("maxOutputTokens", 4096),
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
    
    # Sort and dedupe by id
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


if __name__ == "__main__":
    main()
