//! XMIExporter — exports the class model as OMG XMI 2.5.1 / UML 2.5.1 (§10.4).
//!
//! The model contains nested packages, classes, interfaces and enumerations (with literals),
//! attributes with type references, multiplicities and aggregation kinds, operations with
//! parameters and return parameters, generalizations, interface realizations, associations
//! with both member ends, «use» dependencies and nested classifiers. Element ids are stable
//! hashes of qualified names (`_` + 16 hex digits), so they survive rebuilds of unchanged
//! code. Every text value is XML-escaped. Types referenced from libraries are emitted in an
//! `(external)` package; primitive types reference the UML 2.5.1 PrimitiveTypes library.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::core::types::symbol::SymbolModifiers;
use crate::scpg::diagram::common::DiagramUtils as U;
use crate::scpg::diagram::context::DiagramContext;
use crate::scpg::diagram::views::{multiplicity, TypeIndex};
use crate::uma::types::*;

pub const XMI_NAMESPACE: &str = "http://www.omg.org/spec/XMI/20131001";
pub const UML_NAMESPACE: &str = "http://www.omg.org/spec/UML/20161101";
const PRIMITIVE_TYPES: &str = "http://www.omg.org/spec/UML/20161101/PrimitiveTypes.xmi";

/// Minimal pretty-printing XML writer with attribute escaping.
struct XmlWriter {
    out: String,
    depth: usize,
}

impl XmlWriter {
    fn new() -> Self {
        Self {
            out: String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"),
            depth: 0,
        }
    }

    fn start(&mut self, tag: &str, attrs: &[(&str, String)], empty: bool) {
        self.out.push_str(&"  ".repeat(self.depth));
        self.out.push('<');
        self.out.push_str(tag);
        for (name, value) in attrs {
            self.out.push(' ');
            self.out.push_str(name);
            self.out.push_str("=\"");
            self.out.push_str(&U::xml_escape(value));
            self.out.push('"');
        }
        if empty {
            self.out.push_str("/>\n");
        } else {
            self.out.push_str(">\n");
            self.depth += 1;
        }
    }

    fn open(&mut self, tag: &str, attrs: &[(&str, String)]) {
        self.start(tag, attrs, false);
    }

    fn empty(&mut self, tag: &str, attrs: &[(&str, String)]) {
        self.start(tag, attrs, true);
    }

    fn close(&mut self, tag: &str) {
        self.depth = self.depth.saturating_sub(1);
        self.out.push_str(&"  ".repeat(self.depth));
        self.out.push_str("</");
        self.out.push_str(tag);
        self.out.push_str(">\n");
    }
}

fn xmi_id(parts: &[&str]) -> String {
    format!("_{}", U::short_hash(parts))
}

fn bool_attr(value: bool) -> String {
    String::from(if value { "true" } else { "false" })
}

/// UML primitive type of a Java/Kotlin type name, when it has one.
fn primitive_type(name: &str) -> Option<&'static str> {
    match name {
        "int" | "long" | "short" | "byte" | "Integer" | "Long" | "Short" | "Byte" | "Int"
        | "BigInteger" => Some("Integer"),
        "boolean" | "Boolean" => Some("Boolean"),
        "String" | "CharSequence" | "char" | "Character" | "Char" => Some("String"),
        "float" | "double" | "Float" | "Double" | "BigDecimal" => Some("Real"),
        _ => None,
    }
}

/// How a referenced type is written in the model.
enum TypeRef {
    /// `type="id"` of a declared or external classifier.
    Id(String),
    /// `<type href="PrimitiveTypes.xmi#Name"/>`.
    Primitive(&'static str),
}

struct Association {
    id: String,
    owned_end_id: String,
    member_end_id: String,
    owner_id: String,
}

struct Usage {
    id: String,
    client: String,
    supplier: String,
}

#[derive(Default)]
struct PackageTree {
    children: BTreeMap<String, PackageTree>,
    types: Vec<u32>,
    associations: Vec<Association>,
    usages: Vec<Usage>,
}

impl PackageTree {
    fn at(&mut self, package: Option<&str>) -> &mut PackageTree {
        let mut node = self;
        if let Some(pkg) = package {
            for part in pkg.split('.').filter(|p| !p.is_empty()) {
                node = node.children.entry(part.to_string()).or_default();
            }
        }
        node
    }
}

/// Model builder state shared by the element writers.
struct ModelWriter<'c, 'a> {
    ctx: &'c DiagramContext<'a>,
    types: TypeIndex,
    nested: BTreeMap<u32, Vec<u32>>,
    external: BTreeMap<String, u32>,
    xml: XmlWriter,
}

impl<'c, 'a> ModelWriter<'c, 'a> {
    fn type_id(&self, sym: u32) -> Option<String> {
        self.types.key(sym).map(|k| xmi_id(&["type", k]))
    }

    fn type_ref(&mut self, sym: u32) -> Option<TypeRef> {
        if sym == UNKNOWN_SYM {
            return None;
        }
        if let Some(id) = self.type_id(sym) {
            return Some(TypeRef::Id(id));
        }
        let name = self.ctx.nested_type_name(sym)?;
        if let Some(p) = primitive_type(&name) {
            return Some(TypeRef::Primitive(p));
        }
        if name == "void" || name == "Unit" {
            return None;
        }
        self.external.entry(name.clone()).or_insert(sym);
        Some(TypeRef::Id(xmi_id(&["external", &name])))
    }

    fn typed_attrs(attrs: &mut Vec<(&'static str, String)>, type_ref: &Option<TypeRef>) {
        if let Some(TypeRef::Id(id)) = type_ref {
            attrs.push(("type", id.clone()));
        }
    }

    fn primitive_child(&mut self, type_ref: &Option<TypeRef>) {
        if let Some(TypeRef::Primitive(p)) = type_ref {
            self.xml
                .empty("type", &[("href", format!("{}#{}", PRIMITIVE_TYPES, p))]);
        }
    }

    fn source_comment(&mut self, owner_id: &str, sym: u32) {
        let span = self
            .ctx
            .class_record(sym)
            .and_then(|c| self.ctx.span_of_link(&c.uml_link))
            .or_else(|| self.ctx.symbol_span(sym));
        if let Some(s) = span {
            let body = format!(
                "source: {}:{}-{}",
                s.path.as_deref().unwrap_or("<unknown file>"),
                s.line_start,
                s.line_end
            );
            self.xml.empty(
                "ownedComment",
                &[
                    ("xmi:type", "uml:Comment".to_string()),
                    ("xmi:id", xmi_id(&[owner_id, "source"])),
                    ("body", body),
                ],
            );
        }
    }

    fn write_package(&mut self, name: &str, path: &str, tree: &PackageTree) {
        let id = xmi_id(&["package", path]);
        self.xml.open(
            "packagedElement",
            &[
                ("xmi:type", "uml:Package".to_string()),
                ("xmi:id", id),
                ("name", name.to_string()),
            ],
        );
        self.write_package_contents(path, tree);
        self.xml.close("packagedElement");
    }

    fn write_package_contents(&mut self, path: &str, tree: &PackageTree) {
        for (child, sub) in &tree.children {
            let child_path = if path.is_empty() {
                child.clone()
            } else {
                format!("{}.{}", path, child)
            };
            self.write_package(child, &child_path, sub);
        }
        for &sym in &tree.types {
            self.write_classifier(sym, "packagedElement");
        }
        for assoc in &tree.associations {
            self.xml.open(
                "packagedElement",
                &[
                    ("xmi:type", "uml:Association".to_string()),
                    ("xmi:id", assoc.id.clone()),
                    (
                        "memberEnd",
                        format!("{} {}", assoc.member_end_id, assoc.owned_end_id),
                    ),
                ],
            );
            self.xml.empty(
                "ownedEnd",
                &[
                    ("xmi:type", "uml:Property".to_string()),
                    ("xmi:id", assoc.owned_end_id.clone()),
                    ("type", assoc.owner_id.clone()),
                    ("association", assoc.id.clone()),
                ],
            );
            self.xml.close("packagedElement");
        }
        for usage in &tree.usages {
            self.xml.empty(
                "packagedElement",
                &[
                    ("xmi:type", "uml:Usage".to_string()),
                    ("xmi:id", usage.id.clone()),
                    ("client", usage.client.clone()),
                    ("supplier", usage.supplier.clone()),
                ],
            );
        }
    }

    fn write_classifier(&mut self, sym: u32, tag: &str) {
        let ctx = self.ctx;
        let class_rec = match ctx.class_record(sym) {
            Some(c) => c,
            None => return,
        };
        let id = match self.type_id(sym) {
            Some(id) => id,
            None => return,
        };
        let xmi_type = match class_rec.stereotype {
            STEREOTYPE_INTERFACE | STEREOTYPE_ANNOTATION => "uml:Interface",
            STEREOTYPE_ENUM => "uml:Enumeration",
            _ => "uml:Class",
        };
        let name = ctx
            .simple_name(sym)
            .unwrap_or_else(|| format!("<unnamed type #{}>", sym));
        let mut attrs: Vec<(&str, String)> = vec![
            ("xmi:type", xmi_type.to_string()),
            ("xmi:id", id.clone()),
            ("name", name),
            ("visibility", U::visibility_keyword(class_rec.visibility).to_string()),
        ];
        if xmi_type == "uml:Class" {
            attrs.push((
                "isAbstract",
                bool_attr(
                    class_rec.stereotype == STEREOTYPE_ABSTRACT
                        || class_rec.modifiers & SymbolModifiers::ABSTRACT != 0,
                ),
            ));
            if class_rec.stereotype == STEREOTYPE_RECORD
                || class_rec.modifiers & SymbolModifiers::FINAL != 0
            {
                attrs.push(("isLeaf", "true".to_string()));
            }
        }
        self.xml.open(tag, &attrs);
        self.source_comment(&id, sym);
        if class_rec.stereotype == STEREOTYPE_ANNOTATION || class_rec.stereotype == STEREOTYPE_RECORD
        {
            let keyword = if class_rec.stereotype == STEREOTYPE_ANNOTATION {
                "annotation type"
            } else {
                "record"
            };
            self.xml.empty(
                "ownedComment",
                &[
                    ("xmi:type", "uml:Comment".to_string()),
                    ("xmi:id", xmi_id(&[&id, "keyword"])),
                    ("body", format!("«{}»", keyword)),
                ],
            );
        }

        // Generalizations and interface realizations.
        let is_interface = xmi_type == "uml:Interface";
        let mut supertypes: BTreeSet<u32> = class_rec.generalization_syms.iter().copied().collect();
        if class_rec.extends_sym != UNKNOWN_SYM {
            supertypes.insert(class_rec.extends_sym);
        }
        supertypes.extend(class_rec.implements_syms.iter().copied());
        let implemented: HashSet<u32> = class_rec.implements_syms.iter().copied().collect();
        for &sup in &supertypes {
            if sup == sym {
                continue;
            }
            let general = match self.type_id(sup) {
                Some(g) => g,
                None => continue,
            };
            let target_is_interface = ctx
                .class_record(sup)
                .map(|c| c.stereotype == STEREOTYPE_INTERFACE)
                .unwrap_or(false);
            if !is_interface && (implemented.contains(&sup) || target_is_interface) {
                self.xml.empty(
                    "interfaceRealization",
                    &[
                        ("xmi:type", "uml:InterfaceRealization".to_string()),
                        ("xmi:id", xmi_id(&[&id, "realizes", &general])),
                        ("client", id.clone()),
                        ("supplier", general.clone()),
                        ("contract", general),
                    ],
                );
            } else {
                self.xml.empty(
                    "generalization",
                    &[
                        ("xmi:type", "uml:Generalization".to_string()),
                        ("xmi:id", xmi_id(&[&id, "generalization", &general])),
                        ("general", general),
                    ],
                );
            }
        }

        // Attributes and enumeration literals.
        for f in &class_rec.fields {
            let fname = match ctx.simple_name(f.field_sym_id) {
                Some(n) => n,
                None => continue,
            };
            let prop_id = xmi_id(&[&id, "attribute", &fname]);
            if f.field_kind == FIELD_KIND_ENUM_LITERAL && xmi_type == "uml:Enumeration" {
                self.xml.empty(
                    "ownedLiteral",
                    &[
                        ("xmi:type", "uml:EnumerationLiteral".to_string()),
                        ("xmi:id", prop_id),
                        ("name", fname),
                    ],
                );
                continue;
            }
            let target_sym = if f.element_type_sym_id != UNKNOWN_SYM
                && self.types.contains(f.element_type_sym_id)
            {
                f.element_type_sym_id
            } else {
                f.type_sym_id
            };
            let type_ref = self.type_ref(target_sym);
            let mut attrs: Vec<(&str, String)> = vec![
                ("xmi:type", "uml:Property".to_string()),
                ("xmi:id", prop_id.clone()),
                ("name", fname),
                ("visibility", U::visibility_keyword(f.visibility).to_string()),
            ];
            if f.modifiers & SymbolModifiers::STATIC != 0 {
                attrs.push(("isStatic", "true".to_string()));
            }
            if f.modifiers & SymbolModifiers::FINAL != 0 {
                attrs.push(("isReadOnly", "true".to_string()));
            }
            Self::typed_attrs(&mut attrs, &type_ref);
            let declared_target = self.types.contains(target_sym);
            let association_kind = match f.assoc_kind {
                ASSOC_ASSOCIATION => Some("none"),
                ASSOC_AGGREGATION => Some("shared"),
                ASSOC_COMPOSITION => Some("composite"),
                _ => None,
            };
            if let (Some(kind), true) = (association_kind, declared_target) {
                attrs.push(("aggregation", kind.to_string()));
                attrs.push(("association", xmi_id(&[&prop_id, "association"])));
            }
            self.xml.open("ownedAttribute", &attrs);
            self.primitive_child(&type_ref);
            self.write_multiplicity(&prop_id, f.mult_min, f.mult_max);
            self.xml.close("ownedAttribute");
        }

        // Operations.
        for m in &class_rec.methods {
            let mname = match ctx.simple_name(m.method_sym_id) {
                Some(n) => n,
                None => continue,
            };
            let op_id = xmi_id(&[&id, "operation", &mname, &m.method_sym_id.to_string()]);
            let mut attrs: Vec<(&str, String)> = vec![
                ("xmi:type", "uml:Operation".to_string()),
                ("xmi:id", op_id.clone()),
                ("name", mname),
                ("visibility", U::visibility_keyword(m.visibility).to_string()),
            ];
            if m.modifiers & SymbolModifiers::STATIC != 0 {
                attrs.push(("isStatic", "true".to_string()));
            }
            if m.modifiers & SymbolModifiers::ABSTRACT != 0 {
                attrs.push(("isAbstract", "true".to_string()));
            }
            self.xml.open("ownedOperation", &attrs);
            if m.is_constructor != 0 {
                self.xml.empty(
                    "ownedComment",
                    &[
                        ("xmi:type", "uml:Comment".to_string()),
                        ("xmi:id", xmi_id(&[&op_id, "keyword"])),
                        ("body", "«create»".to_string()),
                    ],
                );
            }
            let params_known = m.param_syms.len() == m.param_count as usize;
            for i in 0..m.param_count as usize {
                let param_id = xmi_id(&[&op_id, "param", &i.to_string()]);
                let mut pattrs: Vec<(&str, String)> = vec![
                    ("xmi:type", "uml:Parameter".to_string()),
                    ("xmi:id", param_id),
                    ("direction", "in".to_string()),
                ];
                let mut type_ref = None;
                if params_known {
                    if let Some(pname) = m.param_syms.get(i).and_then(|&p| ctx.simple_name(p)) {
                        pattrs.push(("name", pname));
                    }
                    if let Some(&t) = m.param_type_syms.get(i) {
                        type_ref = self.type_ref(t);
                    }
                }
                Self::typed_attrs(&mut pattrs, &type_ref);
                if let Some(TypeRef::Primitive(_)) = type_ref {
                    self.xml.open("ownedParameter", &pattrs);
                    self.primitive_child(&type_ref);
                    self.xml.close("ownedParameter");
                } else {
                    self.xml.empty("ownedParameter", &pattrs);
                }
            }
            if m.is_constructor == 0 {
                let ret = self.type_ref(m.return_type_sym_id);
                if ret.is_some() {
                    let mut rattrs: Vec<(&str, String)> = vec![
                        ("xmi:type", "uml:Parameter".to_string()),
                        ("xmi:id", xmi_id(&[&op_id, "return"])),
                        ("direction", "return".to_string()),
                    ];
                    Self::typed_attrs(&mut rattrs, &ret);
                    if let Some(TypeRef::Primitive(_)) = ret {
                        self.xml.open("ownedParameter", &rattrs);
                        self.primitive_child(&ret);
                        self.xml.close("ownedParameter");
                    } else {
                        self.xml.empty("ownedParameter", &rattrs);
                    }
                }
            }
            self.xml.close("ownedOperation");
        }

        // Nested classifiers.
        if let Some(inner) = self.nested.get(&sym).cloned() {
            for inner_sym in inner {
                self.write_classifier(inner_sym, "nestedClassifier");
            }
        }
        self.xml.close(tag);
    }

    fn write_multiplicity(&mut self, owner_id: &str, min: u16, max: u16) {
        if multiplicity(min, max).is_none() {
            return;
        }
        self.xml.empty(
            "lowerValue",
            &[
                ("xmi:type", "uml:LiteralInteger".to_string()),
                ("xmi:id", xmi_id(&[owner_id, "lower"])),
                ("value", min.to_string()),
            ],
        );
        let upper = if max == MULTIPLICITY_MANY {
            "*".to_string()
        } else {
            max.to_string()
        };
        self.xml.empty(
            "upperValue",
            &[
                ("xmi:type", "uml:LiteralUnlimitedNatural".to_string()),
                ("xmi:id", xmi_id(&[owner_id, "upper"])),
                ("value", upper),
            ],
        );
    }
}

pub struct XMIExporter;

impl XMIExporter {
    /// Full UML 2.5.1 class model of the analyzed sources as OMG XMI 2.5.1.
    pub fn export_model(ctx: &DiagramContext) -> String {
        let types = TypeIndex::build(ctx);
        let mut root = PackageTree::default();
        let mut nested: BTreeMap<u32, Vec<u32>> = BTreeMap::new();

        for &sym in &types.syms {
            match ctx.owner_type(sym).filter(|o| types.contains(*o)) {
                Some(outer) => nested.entry(outer).or_default().push(sym),
                None => root.at(types.package(sym)).types.push(sym),
            }
        }

        // Associations and usages live in the package of the outermost owning type.
        for &sym in &types.syms {
            let class_rec = match ctx.class_record(sym) {
                Some(c) => c,
                None => continue,
            };
            let owner_id = match types.key(sym) {
                Some(k) => xmi_id(&["type", k]),
                None => continue,
            };
            let mut outermost = sym;
            let mut guard = 0;
            while let Some(o) = ctx.owner_type(outermost).filter(|o| types.contains(*o)) {
                outermost = o;
                guard += 1;
                if guard > 64 {
                    break;
                }
            }
            let package = types.package(outermost).map(str::to_string);
            let mut associated: HashSet<u32> = HashSet::new();
            let mut associations = Vec::new();
            for f in &class_rec.fields {
                if !matches!(f.assoc_kind, ASSOC_ASSOCIATION | ASSOC_AGGREGATION | ASSOC_COMPOSITION) {
                    continue;
                }
                let target = if f.element_type_sym_id != UNKNOWN_SYM && types.contains(f.element_type_sym_id) {
                    f.element_type_sym_id
                } else {
                    f.type_sym_id
                };
                if !types.contains(target) {
                    continue;
                }
                let fname = match ctx.simple_name(f.field_sym_id) {
                    Some(n) => n,
                    None => continue,
                };
                associated.insert(target);
                let prop_id = xmi_id(&[&owner_id, "attribute", &fname]);
                let assoc_id = xmi_id(&[&prop_id, "association"]);
                associations.push(Association {
                    owned_end_id: xmi_id(&[&assoc_id, "end"]),
                    member_end_id: prop_id,
                    owner_id: owner_id.clone(),
                    id: assoc_id,
                });
            }
            let mut usages = Vec::new();
            let mut supertypes: HashSet<u32> = class_rec.generalization_syms.iter().copied().collect();
            supertypes.extend(class_rec.implements_syms.iter().copied());
            supertypes.insert(class_rec.extends_sym);
            let dependencies: BTreeSet<u32> = class_rec.dependency_syms.iter().copied().collect();
            for target in dependencies {
                if target == sym || associated.contains(&target) || supertypes.contains(&target) {
                    continue;
                }
                if let Some(k) = types.key(target) {
                    let supplier = xmi_id(&["type", k]);
                    usages.push(Usage {
                        id: xmi_id(&[&owner_id, "use", &supplier]),
                        client: owner_id.clone(),
                        supplier,
                    });
                }
            }
            let node = root.at(package.as_deref());
            node.associations.extend(associations);
            node.usages.extend(usages);
        }

        let digest = ctx.input_digest();
        let mut writer = ModelWriter {
            ctx,
            types,
            nested,
            external: BTreeMap::new(),
            xml: XmlWriter::new(),
        };
        writer.xml.open(
            "xmi:XMI",
            &[
                ("xmi:version", "20131001".to_string()),
                ("xmlns:xmi", XMI_NAMESPACE.to_string()),
                ("xmlns:uml", UML_NAMESPACE.to_string()),
            ],
        );
        writer.xml.empty(
            "xmi:Documentation",
            &[
                ("exporter", "OpenHeart".to_string()),
                ("exporterVersion", env!("CARGO_PKG_VERSION").to_string()),
            ],
        );
        let model_id = format!("_model_{}", &digest[..16]);
        writer.xml.open(
            "uml:Model",
            &[
                ("xmi:id", model_id.clone()),
                ("name", "OpenHeart model".to_string()),
            ],
        );
        writer.xml.empty(
            "ownedComment",
            &[
                ("xmi:type", "uml:Comment".to_string()),
                ("xmi:id", format!("{}_input", model_id)),
                ("body", format!("Derived by OpenHeart from input sha256:{}", digest)),
            ],
        );
        writer.write_package_contents("", &root);

        let external: Vec<(String, u32)> = writer
            .external
            .iter()
            .map(|(name, &sym)| (name.clone(), sym))
            .collect();
        if !external.is_empty() {
            writer.xml.open(
                "packagedElement",
                &[
                    ("xmi:type", "uml:Package".to_string()),
                    ("xmi:id", xmi_id(&["package", "(external)"])),
                    ("name", "(external)".to_string()),
                ],
            );
            writer.xml.empty(
                "ownedComment",
                &[
                    ("xmi:type", "uml:Comment".to_string()),
                    ("xmi:id", xmi_id(&["package", "(external)", "comment"])),
                    (
                        "body",
                        "Types referenced by the analyzed sources whose declarations were not analyzed."
                            .to_string(),
                    ),
                ],
            );
            for (name, sym) in external {
                let qualified = ctx.qualified_name(sym).unwrap_or_else(|| name.clone());
                let id = xmi_id(&["external", &name]);
                writer.xml.open(
                    "packagedElement",
                    &[
                        ("xmi:type", "uml:Class".to_string()),
                        ("xmi:id", id.clone()),
                        ("name", name),
                    ],
                );
                writer.xml.empty(
                    "ownedComment",
                    &[
                        ("xmi:type", "uml:Comment".to_string()),
                        ("xmi:id", xmi_id(&[&id, "comment"])),
                        ("body", format!("referenced as {}", qualified)),
                    ],
                );
                writer.xml.close("packagedElement");
            }
            writer.xml.close("packagedElement");
        }
        writer.xml.close("uml:Model");
        writer.xml.close("xmi:XMI");
        writer.xml.out
    }

    /// XMI 2.5.1 from class records alone. Names, members and packages need the symbol
    /// table and token corpus, so classifiers are written unnamed with their generalizations
    /// and realizations, and the model carries a comment saying so. Use [`Self::export_model`]
    /// for the complete model.
    pub fn export_class_diagram(classes: &[ClassRecord]) -> String {
        let declared: HashSet<u32> = classes.iter().map(|c| c.sym_id).collect();
        let id_of = |sym: u32| format!("_sym_{}", sym);
        let mut xml = XmlWriter::new();
        xml.open(
            "xmi:XMI",
            &[
                ("xmi:version", "20131001".to_string()),
                ("xmlns:xmi", XMI_NAMESPACE.to_string()),
                ("xmlns:uml", UML_NAMESPACE.to_string()),
            ],
        );
        xml.empty(
            "xmi:Documentation",
            &[
                ("exporter", "OpenHeart".to_string()),
                ("exporterVersion", env!("CARGO_PKG_VERSION").to_string()),
            ],
        );
        xml.open(
            "uml:Model",
            &[
                ("xmi:id", "_model_records".to_string()),
                ("name", "OpenHeart model".to_string()),
            ],
        );
        xml.empty(
            "ownedComment",
            &[
                ("xmi:type", "uml:Comment".to_string()),
                ("xmi:id", "_model_records_comment".to_string()),
                (
                    "body",
                    "Exported from class records without the symbol table: classifier names, members and packages are not available."
                        .to_string(),
                ),
            ],
        );
        let mut seen = HashSet::new();
        for class_rec in classes {
            if !seen.insert(class_rec.sym_id) {
                continue;
            }
            let id = id_of(class_rec.sym_id);
            let xmi_type = match class_rec.stereotype {
                STEREOTYPE_INTERFACE | STEREOTYPE_ANNOTATION => "uml:Interface",
                STEREOTYPE_ENUM => "uml:Enumeration",
                _ => "uml:Class",
            };
            xml.open(
                "packagedElement",
                &[
                    ("xmi:type", xmi_type.to_string()),
                    ("xmi:id", id.clone()),
                    ("visibility", U::visibility_keyword(class_rec.visibility).to_string()),
                ],
            );
            let mut supertypes: BTreeSet<u32> = class_rec.generalization_syms.iter().copied().collect();
            if class_rec.extends_sym != UNKNOWN_SYM {
                supertypes.insert(class_rec.extends_sym);
            }
            supertypes.extend(class_rec.implements_syms.iter().copied());
            for sup in supertypes {
                if sup == class_rec.sym_id || !declared.contains(&sup) {
                    continue;
                }
                let general = id_of(sup);
                if class_rec.implements_syms.contains(&sup) && xmi_type != "uml:Interface" {
                    xml.empty(
                        "interfaceRealization",
                        &[
                            ("xmi:type", "uml:InterfaceRealization".to_string()),
                            ("xmi:id", format!("{}_realizes_{}", id, sup)),
                            ("client", id.clone()),
                            ("supplier", general.clone()),
                            ("contract", general),
                        ],
                    );
                } else {
                    xml.empty(
                        "generalization",
                        &[
                            ("xmi:type", "uml:Generalization".to_string()),
                            ("xmi:id", format!("{}_generalization_{}", id, sup)),
                            ("general", general),
                        ],
                    );
                }
            }
            xml.close("packagedElement");
        }
        xml.close("uml:Model");
        xml.close("xmi:XMI");
        xml.out
    }
}
