//! AST audit of production output boundaries across all platform branches.
//! Tests/test-support are excluded; macro DSL bodies (including tokio::select!)
//! are scanned as tokens so nested diagnostics cannot bypass the guard.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use syn::{spanned::Spanned, visit::Visit, Attribute, ItemMod, Meta};
fn cfg(meta: &Meta) -> Option<bool> {
    match meta {
        Meta::Path(path) if path.is_ident("test") => Some(false),
        Meta::NameValue(value) if value.path.is_ident("feature") => match &value.value {
            syn::Expr::Lit(v) if matches!(&v.lit,syn::Lit::Str(s) if s.value()=="test-support") => {
                Some(false)
            }
            _ => None,
        },
        Meta::List(list) => {
            use syn::parse::Parser;
            let items = syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated
                .parse2(list.tokens.clone())
                .ok()?;
            let values: Vec<_> = items.iter().map(cfg).collect();
            if list.path.is_ident("all") {
                if values.contains(&Some(false)) {
                    Some(false)
                } else if values.iter().all(|v| *v == Some(true)) {
                    Some(true)
                } else {
                    None
                }
            } else if list.path.is_ident("any") {
                if values.contains(&Some(true)) {
                    Some(true)
                } else if values.iter().all(|v| *v == Some(false)) {
                    Some(false)
                } else {
                    None
                }
            } else if list.path.is_ident("not") {
                values.first().copied().flatten().map(|v| !v)
            } else {
                None
            }
        }
        _ => None,
    }
}
fn production(attrs: &[Attribute]) -> bool {
    !attrs.iter().any(|attr| {
        if attr.path().is_ident("test") {
            return true;
        }
        if attr.path().is_ident("cfg") {
            if let Meta::List(list) = &attr.meta {
                return syn::parse2::<Meta>(list.tokens.clone())
                    .ok()
                    .as_ref()
                    .and_then(cfg)
                    == Some(false);
            }
        }
        false
    })
}

fn output_macro(name: &str, tokens: &proc_macro2::TokenStream) -> bool {
    if matches!(
        name,
        "print"
            | "println"
            | "eprint"
            | "eprintln"
            | "eprint_log"
            | "eprintln_log"
            | "remote_log"
            | "report"
            | "dbg"
    ) {
        return true;
    }
    if !matches!(name, "write" | "writeln") {
        return false;
    }
    use syn::parse::Parser;
    let Ok(args) = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
        .parse2(tokens.clone())
    else {
        return false;
    };
    fn stdio(expr: &syn::Expr) -> bool {
        match expr {
            syn::Expr::Reference(expr) => stdio(&expr.expr),
            syn::Expr::MethodCall(expr) => stdio(&expr.receiver),
            syn::Expr::Call(expr) => match &*expr.func {
                syn::Expr::Path(expr) => {
                    let parts: Vec<_> = expr
                        .path
                        .segments
                        .iter()
                        .map(|part| part.ident.to_string())
                        .collect();
                    parts == ["std", "io", "stderr"] || parts == ["std", "io", "stdout"]
                }
                _ => false,
            },
            _ => false,
        }
    }
    args.first().is_some_and(stdio)
}
struct Audit {
    file: PathBuf,
    dir: PathBuf,
    seen: BTreeSet<PathBuf>,
    rows: Vec<serde_json::Value>,
    scope: Vec<String>,
}
impl Audit {
    fn file(&mut self, file: PathBuf, dir: PathBuf) {
        if !self.seen.insert(file.clone()) {
            return;
        }
        let source = std::fs::read_to_string(&file).unwrap_or_else(|error| {
            panic!("Missing production module {}: {error}", file.display())
        });
        let parsed = syn::parse_file(&source).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        if !production(&parsed.attrs) {
            return;
        }
        let old_file = std::mem::replace(&mut self.file, file);
        let old_dir = std::mem::replace(&mut self.dir, dir);
        self.visit_file(&parsed);
        self.file = old_file;
        self.dir = old_dir;
    }
}
impl Audit {
    fn scan_tokens(&mut self, stream: proc_macro2::TokenStream) {
        use proc_macro2::TokenTree;
        let tokens: Vec<_> = stream.into_iter().collect();
        let mut index = 0;
        while index < tokens.len() {
            if let TokenTree::Ident(first) = &tokens[index] {
                let mut names = vec![first.to_string()];
                let mut end = index + 1;
                while end + 2 < tokens.len()
                    && matches!(&tokens[end],TokenTree::Punct(p) if p.as_char()==':')
                    && matches!(&tokens[end+1],TokenTree::Punct(p) if p.as_char()==':')
                {
                    if let TokenTree::Ident(part) = &tokens[end + 2] {
                        names.push(part.to_string());
                        end += 3;
                    } else {
                        break;
                    }
                }
                let last = names.last().unwrap().as_str();
                let path = names.join("::");
                if end + 1 < tokens.len()
                    && matches!(&tokens[end],TokenTree::Punct(p) if p.as_char()=='!')
                {
                    if let TokenTree::Group(group) = &tokens[end + 1] {
                        if output_macro(last, &group.stream()) {
                            let start = first.span().start();
                            let finish = group.span().end();
                            self.rows.push(serde_json::json!({"file":self.file,"scope":self.scope,"macro":path,"line":start.line,"column":start.column,"endLine":finish.line,"endColumn":finish.column,"tokens":group.stream().to_string(),"nested":true}));
                        }
                        self.scan_tokens(group.stream());
                        index = end + 2;
                        continue;
                    }
                }
                if (path.ends_with("environment::report")
                    || path.ends_with("host_environment::report"))
                    && end < tokens.len()
                {
                    if let TokenTree::Group(group) = &tokens[end] {
                        use syn::parse::Parser;
                        let args=syn::punctuated::Punctuated::<syn::Expr,syn::Token![,]>::parse_terminated.parse2(group.stream());
                        if let Ok(args) = args {
                            if let Some(first_arg) = args.first() {
                                let mut arg = first_arg;
                                if let syn::Expr::Reference(reference) = arg {
                                    arg = &reference.expr
                                }
                                let (kind, content) = match arg {
                                    syn::Expr::Macro(expr) => {
                                        ("format", expr.mac.tokens.to_string())
                                    }
                                    syn::Expr::Lit(expr) => {
                                        if let syn::Lit::Str(text) = &expr.lit {
                                            ("literal", text.value())
                                        } else {
                                            ("other", String::new())
                                        }
                                    }
                                    _ => ("other", String::new()),
                                };
                                let start = first.span().start();
                                let finish = group.span().end();
                                self.rows.push(serde_json::json!({"file":self.file,"scope":self.scope,"call":path,"line":start.line,"column":start.column,"endLine":finish.line,"endColumn":finish.column,"argKind":kind,"content":content,"nested":true}));
                                self.scan_tokens(group.stream());
                                index = end + 1;
                                continue;
                            }
                        }
                    }
                }
            }
            if let TokenTree::Group(group) = &tokens[index] {
                self.scan_tokens(group.stream())
            }
            index += 1;
        }
    }
}
impl<'a> Visit<'a> for Audit {
    fn visit_item_mod(&mut self, item: &'a ItemMod) {
        if !production(&item.attrs) {
            return;
        }
        self.scope.push(item.ident.to_string());
        if let Some((_, items)) = &item.content {
            let old = self.dir.clone();
            self.dir = self.dir.join(item.ident.to_string());
            for item in items {
                self.visit_item(item)
            }
            self.dir = old;
        } else {
            let explicit = item.attrs.iter().find_map(|a| {
                if a.path().is_ident("path") {
                    if let Meta::NameValue(v) = &a.meta {
                        if let syn::Expr::Lit(v) = &v.value {
                            if let syn::Lit::Str(s) = &v.lit {
                                return Some(s.value());
                            }
                        }
                    }
                    None
                } else {
                    None
                }
            });
            let mut path = explicit
                .map(|s| self.file.parent().unwrap().join(s))
                .unwrap_or_else(|| self.dir.join(format!("{}.rs", item.ident)));
            if !path.is_file() {
                path = self.dir.join(item.ident.to_string()).join("mod.rs");
            }
            let dir = if path.file_name().unwrap() == "mod.rs" {
                path.parent().unwrap().to_owned()
            } else {
                path.with_extension("")
            };
            self.file(path, dir);
        }
        self.scope.pop();
    }
    fn visit_item_impl(&mut self, item: &'a syn::ItemImpl) {
        if production(&item.attrs) {
            syn::visit::visit_item_impl(self, item);
        }
    }
    fn visit_item_fn(&mut self, item: &'a syn::ItemFn) {
        if !production(&item.attrs) {
            return;
        }
        self.scope.push(item.sig.ident.to_string());
        syn::visit::visit_item_fn(self, item);
        self.scope.pop();
    }
    fn visit_impl_item_fn(&mut self, item: &'a syn::ImplItemFn) {
        if !production(&item.attrs) {
            return;
        }
        self.scope.push(item.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, item);
        self.scope.pop();
    }
    fn visit_expr_call(&mut self, item: &'a syn::ExprCall) {
        if let syn::Expr::Path(func) = &*item.func {
            let name = func
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            if name.ends_with("environment::report") || name.ends_with("host_environment::report") {
                let mut arg = item.args.first().unwrap();
                if let syn::Expr::Reference(reference) = arg {
                    arg = &reference.expr
                }
                let (kind, content) = match arg {
                    syn::Expr::Macro(expr) => ("format", expr.mac.tokens.to_string()),
                    syn::Expr::Lit(expr) => {
                        if let syn::Lit::Str(text) = &expr.lit {
                            ("literal", text.value())
                        } else {
                            ("other", String::new())
                        }
                    }
                    _ => ("other", String::new()),
                };
                let span = item.span();
                self.rows.push(serde_json::json!({"file":self.file,"scope":self.scope,"call":name,"line":span.start().line,"column":span.start().column,"endLine":span.end().line,"endColumn":span.end().column,"argKind":kind,"content":content}));
            }
        }
        syn::visit::visit_expr_call(self, item);
    }
    fn visit_macro(&mut self, item: &'a syn::Macro) {
        let name = item
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        if output_macro(
            &item.path.segments.last().unwrap().ident.to_string(),
            &item.tokens,
        ) {
            let span = item.span();
            let path = item.path.span();
            self.rows.push(serde_json::json!({"file":self.file,"scope":self.scope,"macro":name,"line":span.start().line,"column":span.start().column,"endLine":span.end().line,"endColumn":span.end().column,"pathEndLine":path.end().line,"pathEndColumn":path.end().column,"tokens":item.tokens.to_string()}));
        }
        self.scan_tokens(item.tokens.clone());
    }
}
pub struct Report {
    pub rows: Vec<serde_json::Value>,
    pub modules: usize,
}

pub fn collect(root: &Path) -> Report {
    let mut audit = Audit {
        file: PathBuf::new(),
        dir: PathBuf::new(),
        seen: BTreeSet::new(),
        rows: Vec::new(),
        scope: Vec::new(),
    };
    for entry in [
        "agent/src/lib.rs",
        "agent/src/main.rs",
        "host/src/lib.rs",
        "desktop/src-tauri/src/lib.rs",
        "desktop/src-tauri/src/main.rs",
    ] {
        let file = root.join(entry);
        let dir = file.parent().unwrap().to_owned();
        audit.file(file, dir);
    }
    for row in &mut audit.rows {
        let file = Path::new(row["file"].as_str().unwrap())
            .strip_prefix(root)
            .unwrap();
        row["file"] = serde_json::json!(file.to_string_lossy().replace('\\', "/"));
    }
    Report {
        rows: audit.rows,
        modules: audit.seen.len(),
    }
}

fn key(row: &serde_json::Value) -> String {
    let function = row["function"]
        .as_str()
        .or_else(|| {
            row["scope"]
                .as_array()
                .and_then(|scope| scope.last())
                .and_then(|v| v.as_str())
        })
        .unwrap_or("<module>");
    serde_json::json!([row["file"], function, row["macro"], row["tokens"]]).to_string()
}

pub fn check(report: &Report) -> Result<(), String> {
    use std::collections::BTreeMap;
    let allowed: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("output_boundaries.json")).unwrap();
    let mut observed: BTreeMap<String, usize> = BTreeMap::new();
    for row in &report.rows {
        let entry = allowed.iter().find(|entry| key(entry) == key(row));
        if entry.is_none() {
            return Err(format!(
                "Diagnostic bypasses tracing: {}:{} ({})",
                row["file"],
                row["line"],
                row.get("macro").or_else(|| row.get("call")).unwrap()
            ));
        }
        *observed.entry(key(row)).or_default() += 1;
    }
    for entry in allowed {
        if observed.get(&key(&entry)).copied().unwrap_or(0)
            != entry["count"].as_u64().unwrap() as usize
        {
            return Err(format!(
                "Output boundary changed: {} / {} ({})",
                entry["file"], entry["function"], entry["reason"]
            ));
        }
    }
    Ok(())
}
