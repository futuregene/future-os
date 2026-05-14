#!/usr/bin/env python3
"""
convert_go_models.py - Convert Go models_generated.go to Rust format

Usage:
    python3 scripts/convert_go_models.py
"""

import re
import sys

def parse_go_models(go_content: str) -> list:
    """Parse mk() calls from Go models_generated.go."""
    
    # Pattern to match mk() calls - handle escaped quotes in JSON fields and negative costs
    # mk("id", "name", "provider", ctx, max, bool, []string{...}, ...)
    pattern = r'mk\(\s*'
    pattern += r'"([^"]+)"\s*,\s*'  # id
    pattern += r'"([^"]*)"\s*,\s*'  # name (allow empty)
    pattern += r'"([^"]+)"\s*,\s*'  # provider
    pattern += r'(\d+)\s*,\s*'  # context_window
    pattern += r'(\d+)\s*,\s*'  # max_tokens
    pattern += r'(true|false)\s*,\s*'  # reasoning
    pattern += r'\[\]string\{([^}]*)\}\s*,\s*'  # input
    pattern += r'(-?[\d.]+)\s*,\s*'  # cost_input (allow negative)
    pattern += r'(-?[\d.]+)\s*,\s*'  # cost_output (allow negative)
    pattern += r'([\d.]+)\s*,\s*'  # cost_cache_read
    pattern += r'([\d.]+)\s*,\s*'  # cost_cache_write
    pattern += r'"((?:[^"\\]|\\.)*)"\s*,\s*'  # compat_json (handle escaped quotes)
    pattern += r'"((?:[^"\\]|\\.)*)"\s*,\s*'  # tlm_json
    pattern += r'"((?:[^"\\]|\\.)*)"\s*,\s*'  # headers_json
    pattern += r'"([^"]+)"\s*,\s*'  # api
    pattern += r'"([^"]+)"'  # base_url
    pattern += r'\)'
    
    models = []
    for match in re.finditer(pattern, go_content):
        (id, name, provider, ctx, max_tok, reasoning, 
         input_str, cost_in, cost_out, cr, cw,
         compat, tlm, headers, api, base_url) = match.groups()
        
        # Parse input array - items are "quoted" strings
        inputs = []
        for item in input_str.split(','):
            item = item.strip().strip('"')
            if item:
                inputs.append(item)
        input_vec = "vec![" + ", ".join(f'String::from("{i}")' for i in inputs) + "]"
        
        # Parse numbers - handle trailing zeros
        cost_in = float(cost_in) if cost_in else 0.0
        cost_out = float(cost_out) if cost_out else 0.0
        cr = float(cr) if cr else 0.0
        cw = float(cw) if cw else 0.0
        
        models.append({
            'id': id,
            'name': name,
            'provider': provider,
            'api': api,
            'base_url': base_url,
            'reasoning': reasoning == 'true',
            'input': input_vec,
            'context_window': int(ctx),
            'max_tokens': int(max_tok),
            'cost_input': cost_in,
            'cost_output': cost_out,
            'cost_cache_read': cr,
            'cost_cache_write': cw,
            'compat_json': compat,
            'tlm_json': tlm,
            'headers_json': headers,
        })
    
    return models


def generate_rust_code(models: list) -> str:
    """Generate Rust code from model list."""
    
    output = """//! Generated model catalog — converted from Go internal/modelregistry/models_generated.go
//!
//! {} models (same as Go)
//!
//! Generated from: internal/modelregistry/models_generated.go

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
""".format(len(models))
    
    current_provider = None
    for m in models:
        if current_provider != m['provider']:
            if current_provider is not None:
                output += "        // ── end -->\n"
            current_provider = m['provider']
            output += f"        // ── {m['provider']} -->\n"
        
        output += f"""        Model {{
            id: "{m['id']}".into(),
            name: "{m['name']}".into(),
            provider: "{m['provider']}".into(),
            api: "{m['api']}".into(),
            base_url: "{m['base_url']}".into(),
            reasoning: {str(m['reasoning']).lower()},
            input: {m['input']},
            context_window: {m['context_window']},
            max_tokens: {m['max_tokens']},
            cost_input: {m['cost_input']},
            cost_output: {m['cost_output']},
            cost_cache_read: {m['cost_cache_read']},
            cost_cache_write: {m['cost_cache_write']},
            compat_json: String::from("{m['compat_json']}"),
            tlm_json: String::from("{m['tlm_json']}"),
            headers_json: String::from("{m['headers_json']}"),
        }},
"""
    
    if current_provider is not None:
        output += "        // ── end -->\n"
    
    output += """    ]
}
"""
    
    return output


def main():
    go_path = "../internal/modelregistry/models_generated.go"
    
    try:
        with open(go_path, "r") as f:
            go_content = f.read()
    except FileNotFoundError:
        print(f"Error: Go file not found at {go_path}", file=sys.stderr)
        print("Run this script from the xihu/agent directory", file=sys.stderr)
        sys.exit(1)
    
    print(f"Reading {go_path}...")
    print(f"File size: {len(go_content)} bytes")
    
    models = parse_go_models(go_content)
    print(f"Parsed {len(models)} models")
    
    # Compare with Go's count
    go_count = go_content.count('mk("')
    print(f"Go mk() count: {go_count}")
    
    if len(models) != go_count:
        print(f"Warning: Parsed {len(models)} models but Go has {go_count}")
        # Find which ones we missed by checking format
        unparsed_lines = []
        for i, line in enumerate(go_content.split('\n')):
            if 'mk("' in line:
                # Check if it was parsed
                if not any(m['id'] in line for m in models[:5]):  # Quick check
                    # Try to extract just the id
                    id_match = re.search(r'mk\("([^"]+)"', line)
                    if id_match:
                        unparsed_lines.append((i+1, id_match.group(1), line[:80]))
        
        if unparsed_lines:
            print(f"\nFirst few unparsed lines:")
            for line_num, model_id, line_text in unparsed_lines[:5]:
                print(f"  Line {line_num}: {model_id}")
                print(f"    {line_text}...")
    
    rust_code = generate_rust_code(models)
    
    output_path = "src/models/generated/mod.rs"
    with open(output_path, "w") as f:
        f.write(rust_code)
    
    print(f"\nWritten to {output_path}")
    print(f"Generated {len(models)} models")


if __name__ == "__main__":
    main()
