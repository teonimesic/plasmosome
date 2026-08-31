use std::collections::BTreeMap;
use std::fmt;

use proc_macro2::TokenTree;
use syn::visit::{self, Visit};

const SHARED_MEMORY_NAMES: &[&str] = &[
    "Arc",
    "Rc",
    "Weak",
    "Mutex",
    "MutexGuard",
    "RwLock",
    "RwLockReadGuard",
    "RwLockWriteGuard",
    "ReentrantLock",
    "Condvar",
    "Barrier",
    "OnceLock",
    "OnceCell",
    "LazyLock",
    "LazyCell",
    "Lazy",
    "Cell",
    "RefCell",
    "UnsafeCell",
    "SyncUnsafeCell",
    "AtomicCell",
    "AtomicBool",
    "AtomicPtr",
    "AtomicI8",
    "AtomicI16",
    "AtomicI32",
    "AtomicI64",
    "AtomicIsize",
    "AtomicU8",
    "AtomicU16",
    "AtomicU32",
    "AtomicU64",
    "AtomicUsize",
    "thread_local",
    "lazy_static",
    "once_cell",
    "parking_lot",
    "arc_swap",
    "dashmap",
    "crossbeam",
];

const RAW_POINTER: &str = "a raw pointer";

const STATIC_MUT: &str = "static mut";

/// One use of shared memory found by reading a Rust file as code.
///
/// A caller reports it as written: `construct` is the name the standard library or a crate gives
/// the thing, `alias` is the local name the file reached it through when it renamed it, and `item`
/// is the item the use sits in. The caller must add the path of the file itself; a use does not
/// know which file it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedMemoryUse {
    /// The construct under its own name, such as `RefCell`, `AtomicUsize` or `static mut`.
    pub construct: String,
    /// The local name the file gave the construct, when it reached it through an alias.
    pub alias: Option<String>,
    /// The item the use sits in, such as `InstanceRecord` or `Reconciler::converge`.
    pub item: Option<String>,
}

impl fmt::Display for SharedMemoryUse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.alias {
            Some(alias) => write!(
                formatter,
                "`{alias}`, a local alias for `{}`",
                self.construct
            )?,
            None => write!(formatter, "`{}`", self.construct)?,
        }
        match &self.item {
            Some(item) => write!(formatter, " in `{item}`"),
            None => Ok(()),
        }
    }
}

/// Reads `source` as Rust and returns every shared-memory construct it uses, in the order found.
///
/// The caller must pass the whole text of one Rust file. Only positions where Rust means a path, a
/// type, an import, an attribute or macro argument, or a `static` are inspected, so a comment, a
/// doc block, a test name or a string literal that names a lock is not a use and is not returned.
/// Identifiers are compared whole: `CellRecord` is not `Cell`.
///
/// Aliases the file declares itself are followed — `use std::sync::Mutex as Guard;` and
/// `type Guard = Mutex<u32>;` both make a later `Guard<u32>` a use. An alias declared in another
/// file or another crate is **not** followed and will not be returned: deciding what an imported
/// name resolves to is name resolution, which needs the compiler, not a parser.
///
/// A name is matched on its spelling alone, so an identifier that is one of these words but means
/// something else — an enum variant `SyncPoint::Barrier`, a grid `Cell`, a file-local
/// `use crate::registry::Handle as Weak;` — is returned as a use it is not. Telling those apart is
/// the same name resolution the paragraph above needs a compiler for, so the two limits are one
/// limit read from its two sides: an unresolvable name is either missed or over-reported, and this
/// rule chooses to over-report. Narrow the construct list if a wire file has to carry such a
/// word; do not reach for a regex.
///
/// Returns the parse error when `source` is not valid Rust.
pub fn shared_memory_uses(source: &str) -> Result<Vec<SharedMemoryUse>, syn::Error> {
    let file = syn::parse_file(source)?;
    let aliases = local_aliases(&file);
    let mut scan = Scan {
        aliases: &aliases,
        item: Vec::new(),
        found: Vec::new(),
    };
    scan.visit_file(&file);
    Ok(scan.found)
}

fn is_shared_memory_name(name: &str) -> bool {
    SHARED_MEMORY_NAMES.contains(&name)
}

fn local_aliases(file: &syn::File) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    loop {
        let before = map.len();
        let mut collect = CollectAliases { map: &mut map };
        collect.visit_file(file);
        if map.len() == before {
            return map;
        }
    }
}

fn resolve(name: &str, aliases: &BTreeMap<String, String>) -> Option<(String, Option<String>)> {
    if is_shared_memory_name(name) {
        return Some((name.to_string(), None));
    }
    aliases
        .get(name)
        .map(|construct| (construct.clone(), Some(name.to_string())))
}

struct CollectAliases<'a> {
    map: &'a mut BTreeMap<String, String>,
}

impl<'a> CollectAliases<'a> {
    fn record(&mut self, local: String, construct: &str) {
        if !is_shared_memory_name(&local) {
            self.map.insert(local, construct.to_string());
        }
    }
}

impl<'ast> Visit<'ast> for CollectAliases<'_> {
    fn visit_use_rename(&mut self, node: &'ast syn::UseRename) {
        let target = node.ident.to_string();
        if let Some((construct, _)) = resolve(&target, self.map) {
            self.record(node.rename.to_string(), &construct);
        }
        visit::visit_use_rename(self, node);
    }

    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        let mut names = CollectPathNames { names: Vec::new() };
        names.visit_type(&node.ty);
        let found = names
            .names
            .iter()
            .find_map(|name| resolve(name, self.map).map(|(construct, _)| construct));
        if let Some(construct) = found {
            self.record(node.ident.to_string(), &construct);
        }
        visit::visit_item_type(self, node);
    }
}

struct CollectPathNames {
    names: Vec<String>,
}

impl<'ast> Visit<'ast> for CollectPathNames {
    fn visit_path(&mut self, node: &'ast syn::Path) {
        for segment in &node.segments {
            self.names.push(segment.ident.to_string());
        }
        visit::visit_path(self, node);
    }
}

struct Scan<'a> {
    aliases: &'a BTreeMap<String, String>,
    item: Vec<String>,
    found: Vec<SharedMemoryUse>,
}

impl Scan<'_> {
    fn note(&mut self, name: &str) {
        if let Some((construct, alias)) = resolve(name, self.aliases) {
            self.record(construct, alias);
        }
    }

    fn record(&mut self, construct: String, alias: Option<String>) {
        let item = if self.item.is_empty() {
            None
        } else {
            Some(self.item.join("::"))
        };
        self.found.push(SharedMemoryUse {
            construct,
            alias,
            item,
        });
    }

    fn note_tokens(&mut self, tokens: &proc_macro2::TokenStream) {
        for tree in tokens.clone() {
            match tree {
                TokenTree::Ident(ident) => self.note(&ident.to_string()),
                TokenTree::Group(group) => self.note_tokens(&group.stream()),
                TokenTree::Punct(_) | TokenTree::Literal(_) => {}
            }
        }
    }

    fn within<F: FnOnce(&mut Self)>(&mut self, name: Option<String>, body: F) {
        match name {
            Some(name) => {
                self.item.push(name);
                body(self);
                self.item.pop();
            }
            None => body(self),
        }
    }
}

impl<'ast> Visit<'ast> for Scan<'_> {
    fn visit_item(&mut self, node: &'ast syn::Item) {
        let name = item_name(node);
        self.within(name, |scan| visit::visit_item(scan, node));
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        let name = Some(node.sig.ident.to_string());
        self.within(name, |scan| visit::visit_impl_item_fn(scan, node));
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        for segment in &node.segments {
            self.note(&segment.ident.to_string());
        }
        visit::visit_path(self, node);
    }

    fn visit_use_path(&mut self, node: &'ast syn::UsePath) {
        self.note(&node.ident.to_string());
        visit::visit_use_path(self, node);
    }

    fn visit_use_name(&mut self, node: &'ast syn::UseName) {
        self.note(&node.ident.to_string());
        visit::visit_use_name(self, node);
    }

    fn visit_use_rename(&mut self, node: &'ast syn::UseRename) {
        let target = node.ident.to_string();
        if let Some((construct, _)) = resolve(&target, self.aliases) {
            self.record(construct, Some(node.rename.to_string()));
        }
        visit::visit_use_rename(self, node);
    }

    fn visit_type_ptr(&mut self, node: &'ast syn::TypePtr) {
        self.record(RAW_POINTER.to_string(), None);
        visit::visit_type_ptr(self, node);
    }

    fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
        if matches!(node.mutability, syn::StaticMutability::Mut(_)) {
            self.record(STATIC_MUT.to_string(), None);
        }
        visit::visit_item_static(self, node);
    }

    fn visit_foreign_item_static(&mut self, node: &'ast syn::ForeignItemStatic) {
        if matches!(node.mutability, syn::StaticMutability::Mut(_)) {
            self.record(STATIC_MUT.to_string(), None);
        }
        visit::visit_foreign_item_static(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        visit::visit_macro(self, node);
        self.note_tokens(&node.tokens);
    }

    fn visit_meta_list(&mut self, node: &'ast syn::MetaList) {
        visit::visit_meta_list(self, node);
        self.note_tokens(&node.tokens);
    }

    fn visit_item_extern_crate(&mut self, node: &'ast syn::ItemExternCrate) {
        self.note(&node.ident.to_string());
        visit::visit_item_extern_crate(self, node);
    }
}

fn item_name(item: &syn::Item) -> Option<String> {
    match item {
        syn::Item::Const(item) => Some(item.ident.to_string()),
        syn::Item::Enum(item) => Some(item.ident.to_string()),
        syn::Item::Fn(item) => Some(item.sig.ident.to_string()),
        syn::Item::Mod(item) => Some(item.ident.to_string()),
        syn::Item::Static(item) => Some(item.ident.to_string()),
        syn::Item::Struct(item) => Some(item.ident.to_string()),
        syn::Item::Trait(item) => Some(item.ident.to_string()),
        syn::Item::Type(item) => Some(item.ident.to_string()),
        syn::Item::Union(item) => Some(item.ident.to_string()),
        syn::Item::Impl(item) => self_type_name(&item.self_ty),
        _ => None,
    }
}

fn self_type_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    }
}
