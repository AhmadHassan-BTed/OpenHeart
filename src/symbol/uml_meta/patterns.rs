//! PatternDetector: detects Singleton, Factory, and Observer design patterns in the symbol table.

use crate::core::types::symbol::{SymbolKind, SymbolModifiers, SymbolVisibility};
use crate::symbol::builder::SymbolTableBuilder;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesignPattern {
    Singleton,
    Factory,
    Observer,
}

pub struct PatternDetector;

impl PatternDetector {
    /// Detects if a class symbol implements the Singleton design pattern:
    /// 1. Private static field of the class's own type.
    /// 2. Public static method returning the class's type (e.g. `getInstance()`).
    /// 3. All constructors are private or protected.
    pub fn is_singleton(class_sym_id: u32, builder: &SymbolTableBuilder) -> bool {
        let class_sym = match builder.symbol(class_sym_id) {
            Some(s) if s.kind == SymbolKind::SK_CLASS as u8 => s,
            _ => return false,
        };

        let mut has_private_static_self_field = false;
        let mut has_public_static_factory_method = false;
        let mut has_public_ctor = false;

        let mut cur = class_sym.first_child;
        while cur != u32::MAX {
            if let Some(member) = builder.symbol(cur) {
                if member.kind == SymbolKind::SK_FIELD as u8 {
                    let is_static = (member.modifiers & SymbolModifiers::STATIC) != 0;
                    let is_private = member.visibility == SymbolVisibility::Private as u8;
                    if is_static && is_private && member.type_id == class_sym_id {
                        has_private_static_self_field = true;
                    }
                } else if member.kind == SymbolKind::SK_METHOD as u8 {
                    let is_static = (member.modifiers & SymbolModifiers::STATIC) != 0;
                    let is_public = member.visibility == SymbolVisibility::Public as u8;
                    if is_static && is_public && member.type_id == class_sym_id {
                        has_public_static_factory_method = true;
                    }
                } else if member.kind == SymbolKind::SK_CONSTRUCTOR as u8 {
                    let is_public = member.visibility == SymbolVisibility::Public as u8;
                    if is_public {
                        has_public_ctor = true;
                    }
                }
            }
            cur = builder
                .symbol(cur)
                .map(|s| s.next_sibling)
                .unwrap_or(u32::MAX);
        }

        has_private_static_self_field && has_public_static_factory_method && !has_public_ctor
    }

    /// Detects if a class/interface implements the Factory design pattern:
    /// Has methods returning instances of abstract classes or interfaces.
    pub fn is_factory(class_sym_id: u32, builder: &SymbolTableBuilder) -> bool {
        let class_sym = match builder.symbol(class_sym_id) {
            Some(s)
                if s.kind == SymbolKind::SK_CLASS as u8
                    || s.kind == SymbolKind::SK_INTERFACE as u8 =>
            {
                s
            }
            _ => return false,
        };

        let mut has_factory_method = false;
        let mut cur = class_sym.first_child;
        while cur != u32::MAX {
            if let Some(member) = builder.symbol(cur) {
                if member.kind == SymbolKind::SK_METHOD as u8 {
                    let ret_type_id = member.type_id;
                    if ret_type_id != u32::MAX && ret_type_id != class_sym_id {
                        if let Some(ret_sym) = builder.symbol(ret_type_id) {
                            if ret_sym.kind == SymbolKind::SK_INTERFACE as u8
                                || (ret_sym.modifiers & SymbolModifiers::ABSTRACT) != 0
                            {
                                has_factory_method = true;
                                break;
                            }
                        }
                    }
                }
            }
            cur = builder
                .symbol(cur)
                .map(|s| s.next_sibling)
                .unwrap_or(u32::MAX);
        }
        has_factory_method
    }

    /// Detects if a class/interface implements the Observer design pattern:
    /// Has methods to register or unregister listeners (e.g. `addListener`, `removeListener`).
    pub fn is_observer(class_sym_id: u32, builder: &SymbolTableBuilder) -> bool {
        let class_sym = match builder.symbol(class_sym_id) {
            Some(s)
                if s.kind == SymbolKind::SK_CLASS as u8
                    || s.kind == SymbolKind::SK_INTERFACE as u8 =>
            {
                s
            }
            _ => return false,
        };

        let mut has_listener_method = false;
        let mut cur = class_sym.first_child;
        while cur != u32::MAX {
            if let Some(member) = builder.symbol(cur) {
                if member.kind == SymbolKind::SK_METHOD as u8 {
                    let name = builder
                        .qual_names
                        .lookup_by_id(member.name_id)
                        .unwrap_or("");
                    if (name.starts_with("add") || name.starts_with("remove"))
                        && (name.contains("Listener") || name.contains("Observer"))
                    {
                        has_listener_method = true;
                        break;
                    }
                }
            }
            cur = builder
                .symbol(cur)
                .map(|s| s.next_sibling)
                .unwrap_or(u32::MAX);
        }

        has_listener_method
    }
}
