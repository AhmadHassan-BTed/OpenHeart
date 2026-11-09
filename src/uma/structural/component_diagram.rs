//! ComponentDiagramExtractor — components with provided and required interfaces
//! (§9.2.1).
//!
//! Two component sources are modelled:
//! - **Dependency-injection beans**: classes annotated as framework components
//!   (`@Component`, `@Service`, `@Repository`, `@Controller`, `@RestController`,
//!   `@Configuration`, JSR-330 `@Named`/`@Singleton`, EJB `@Stateless`/`@Stateful`).
//!   Provided interfaces are the interfaces the class realizes (transitively);
//!   required interfaces are interface-typed attributes and constructor
//!   parameters (the injection points).
//! - **Packages**: provided interfaces are the interfaces the package declares;
//!   required interfaces are interfaces declared elsewhere that the package's
//!   types reference.
//! Build-system modules (Maven/Gradle/JPMS) are not modelled: the token corpus
//! carries no build metadata, so no module component is invented.

use std::collections::{BTreeSet, HashMap};

use crate::uma::bodies::MemberBodies;
use crate::uma::index::UMAIndex;
use crate::uma::structural::package_diagram::PackageDiagramExtractor;
use crate::uma::types::*;

/// Framework annotations that declare a managed component.
const BEAN_ANNOTATIONS: &[&[u8]] = &[
    b"Component",
    b"Service",
    b"Repository",
    b"Controller",
    b"RestController",
    b"Configuration",
    b"Named",
    b"ManagedBean",
    b"Stateless",
    b"Stateful",
    b"Singleton",
    b"ApplicationScoped",
    b"RequestScoped",
];

pub struct ComponentDiagramExtractor;

impl ComponentDiagramExtractor {
    pub fn extract(
        index: &UMAIndex<'_>,
        bodies: &MemberBodies,
        packages: &[PackageRecord],
        class_syms: &[u32],
    ) -> Vec<ComponentRecord> {
        let mut components = Vec::new();

        for &c in class_syms {
            if index.is_interface(c) {
                continue;
            }
            let sym = match index.symbol(c) {
                Some(s) if s.first_token_id != u32::MAX => s,
                _ => continue,
            };
            let is_bean = index
                .scanner
                .annotations(sym)
                .iter()
                .any(|&a| BEAN_ANNOTATIONS.contains(&index.tca.interner.lookup_text(a)));
            if !is_bean {
                continue;
            }
            let provided: BTreeSet<u32> = index
                .ancestors(c)
                .into_iter()
                .filter(|&a| index.is_interface(a))
                .collect();
            let mut required: BTreeSet<u32> = BTreeSet::new();
            for f in index.fields(c) {
                let t = index.type_of(f);
                if index.is_interface(t) {
                    required.insert(t);
                }
            }
            for k in index.constructors(c) {
                for t in index.param_types(k) {
                    if index.is_interface(t) {
                        required.insert(t);
                    }
                }
            }
            for p in &provided {
                required.remove(p);
            }
            components.push(Self::record(
                c,
                sym.name_id,
                COMPONENT_KIND_BEAN,
                provided.into_iter().collect(),
                required.into_iter().collect(),
            ));
        }

        let mut by_package: HashMap<u32, Vec<u32>> = HashMap::new();
        for &t in class_syms {
            let p = index.package_of(t);
            if p != UNKNOWN_SYM {
                by_package.entry(p).or_default().push(t);
            }
        }
        for pkg in packages {
            let members: &[u32] = match by_package.get(&pkg.package_sym_id) {
                Some(m) if !m.is_empty() => m,
                _ => continue,
            };
            let provided: BTreeSet<u32> = members
                .iter()
                .copied()
                .filter(|&t| index.is_interface(t))
                .collect();
            let mut required: BTreeSet<u32> = BTreeSet::new();
            for &t in members {
                for r in PackageDiagramExtractor::referenced_types(index, bodies, t) {
                    if index.is_interface(r) && index.package_of(r) != pkg.package_sym_id {
                        required.insert(r);
                    }
                }
            }
            components.push(Self::record(
                pkg.package_sym_id,
                pkg.name_id,
                COMPONENT_KIND_PACKAGE,
                provided.into_iter().collect(),
                required.into_iter().collect(),
            ));
        }

        components
    }

    fn record(
        sym: u32,
        name_id: u32,
        kind: u8,
        provided: Vec<u32>,
        required: Vec<u32>,
    ) -> ComponentRecord {
        ComponentRecord {
            component_sym_id: sym,
            name_id,
            provided_interface_count: u16::try_from(provided.len()).unwrap_or(u16::MAX),
            required_interface_count: u16::try_from(required.len()).unwrap_or(u16::MAX),
            component_kind: kind,
            provided_interfaces: provided,
            required_interfaces: required,
        }
    }
}
