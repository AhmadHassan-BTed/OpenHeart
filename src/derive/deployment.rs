//! Deployment diagram from build and deployment descriptors.
//!
//! Execution nodes, the artifacts deployed on them and the communication paths between
//! them exist only when a descriptor of the project model declares them (container
//! images, orchestration manifests, platform targets). Program code alone does not
//! determine where it runs, so without such descriptors the view is `NotDerivable`.
//! Artifacts that manifest a build unit carry a «manifest» relationship to its component.

use std::collections::BTreeMap;

use crate::derive::model::{
    DiagramEdge, DiagramKind, DiagramModel, DiagramNode, EdgeKind, ElementId, IdBuilder, NodeKind,
};
use crate::derive::provenance::Fact;
use crate::derive::DeriveContext;

/// The deployment diagram of the project model.
pub fn derive(ctx: &DeriveContext) -> DiagramModel {
    let kind = DiagramKind::Deployment;
    let Some(project) = ctx.project else {
        return DiagramModel::not_derivable(
            kind,
            kind.title(),
            "no project model: execution nodes come only from build or deployment descriptors",
        );
    };
    let mut nodes = project.execution_nodes();
    nodes.sort_by_key(|n| n.id);
    if nodes.is_empty() {
        return DiagramModel::not_derivable(
            kind,
            kind.title(),
            "no build or deployment descriptor of the project declares an execution node",
        );
    }
    let mut model = DiagramModel::new(kind, kind.title());
    let ids: BTreeMap<u32, ElementId> = nodes
        .iter()
        .map(|n| {
            (
                n.id,
                IdBuilder::new("execnode")
                    .text(n.kind.mnemonic())
                    .text(&n.name)
                    .text(&n.descriptor)
                    .finish(),
            )
        })
        .collect();
    for n in &nodes {
        let mut node = DiagramNode::new(ids[&n.id].clone(), NodeKind::ExecutionNode, n.name.clone())
            .with_stereotype(n.kind.mnemonic())
            .with_parent(n.parent.and_then(|p| ids.get(&p).cloned()))
            .with_property("descriptor", n.descriptor.clone())
            .with_fact(Fact::ExecutionNode(n.id));
        for (k, v) in &n.properties {
            node = node.with_property(k.clone(), v.clone());
        }
        model.add_node(node);
    }

    let units: BTreeMap<u32, (String, String)> = project
        .units()
        .into_iter()
        .map(|u| (u.id, (u.name, u.kind.mnemonic().to_string())))
        .collect();
    let mut artifacts = project.deployed_artifacts();
    artifacts.sort_by(|a, b| (a.node, &a.name).cmp(&(b.node, &b.name)));
    for a in &artifacts {
        let Some(host) = ids.get(&a.node) else {
            continue;
        };
        let aid = IdBuilder::new("artifact")
            .text(host.as_str())
            .text(&a.name)
            .finish();
        model.add_node(
            DiagramNode::new(aid.clone(), NodeKind::Artifact, a.name.clone())
                .with_stereotype("artifact")
                .with_parent(Some(host.clone()))
                .with_property("descriptor", a.descriptor.clone())
                .with_fact(Fact::ExecutionNode(a.node)),
        );
        if let Some(unit) = a.unit {
            if let Some((name, unit_kind)) = units.get(&unit) {
                let cid = IdBuilder::new("deployed_unit").text(name).text(unit_kind).finish();
                model.add_node(
                    DiagramNode::new(cid.clone(), NodeKind::Component, name.clone())
                        .with_stereotype(unit_kind.clone())
                        .with_fact(Fact::ProjectUnit(unit)),
                );
                model.add_edge(
                    DiagramEdge::new(
                        IdBuilder::new("manifest").text(aid.as_str()).text(cid.as_str()).finish(),
                        EdgeKind::Manifestation,
                        aid.clone(),
                        cid,
                    )
                    .with_stereotype("manifest")
                    .with_fact(Fact::ProjectUnit(unit)),
                );
            }
        }
    }

    let mut paths = project.communication_paths();
    paths.sort_by(|a, b| (a.from, a.to).cmp(&(b.from, b.to)));
    for p in &paths {
        let (Some(s), Some(t)) = (ids.get(&p.from), ids.get(&p.to)) else {
            continue;
        };
        let mut edge = DiagramEdge::new(
            IdBuilder::new("commpath")
                .text(s.as_str())
                .text(t.as_str())
                .text(p.protocol.as_deref().unwrap_or(""))
                .finish(),
            EdgeKind::CommunicationPath,
            s.clone(),
            t.clone(),
        )
        .with_property("descriptor", p.descriptor.clone())
        .with_fact(Fact::ExecutionNode(p.from));
        if let Some(proto) = &p.protocol {
            edge = edge.with_stereotype(proto.clone());
        }
        model.add_edge(edge);
    }
    model.finalize(ctx.limits.max_spans);
    model
}
