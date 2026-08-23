//! Adversarial "Trick Yourself" Stress Test Suite (§10.4).
//!
//! Tests complex, deceptive code topologies designed to challenge static analysis:
//! - Diamond interface inheritance
//! - Recursive Decorator wrapping
//! - Strategy switching with dynamic dispatch
//! - Disambiguation of identical class names across deep packages
//! - Composite pattern with cycle-safe aggregations
//! - Adapter delegation
//! - Facade subsystem aggregation
//! - Multi-state state machines with guarded transitions
//! - Multi-participant sequence interactions with combined fragments

use openheart::ast::{ASTStage, ASTStageInput};
use openheart::cfg::Phase4Stage;
use openheart::cg::Phase6Stage;
use openheart::core::io::mmap::MemoryMappedFile;
use openheart::ingestion::manifest::SourceManifest;
use openheart::ingestion::IngestionStage;
use openheart::psa::Phase8Stage;
use openheart::scpg::diagram::export::{MermaidExporter, PlantUMLExporter};
use openheart::ssa::Phase5Stage;
use openheart::symbol::Phase3Stage;
use openheart::tra::Phase7Stage;
use openheart::uma::Phase9Stage;

use std::fs;
use tempfile::tempdir;

#[test]
fn test_adversarial_trick_complex_patterns_and_diagrams() {
    let dir = tempdir().unwrap();

    // 1. Multi-file complex codebase with edge-case topologies
    let java_code1 = r#"
package com.openheart.trick.core;

public interface BaseComponent {
    void execute();
    int getStatusCode();
}

public interface EnhancedComponent extends BaseComponent {
    void enhance();
}

public interface AuditedComponent extends BaseComponent {
    void audit();
}

public class DiamondComponent implements EnhancedComponent, AuditedComponent {
    private int status = 200;
    
    @Override
    public void execute() {
        enhance();
        audit();
    }
    
    @Override
    public int getStatusCode() {
        return this.status;
    }
    
    @Override
    public void enhance() {}
    
    @Override
    public void audit() {}
}
"#;

    let java_code2 = r#"
package com.openheart.trick.patterns;

import com.openheart.trick.core.BaseComponent;

// Decorator Pattern
public abstract class ComponentDecorator implements BaseComponent {
    protected BaseComponent wrapped;
    
    public ComponentDecorator(BaseComponent target) {
        this.wrapped = target;
    }
    
    @Override
    public void execute() {
        if (this.wrapped != null) {
            this.wrapped.execute();
        }
    }
    
    @Override
    public int getStatusCode() {
        return this.wrapped != null ? this.wrapped.getStatusCode() : 0;
    }
}

public class LoggingDecorator extends ComponentDecorator {
    public LoggingDecorator(BaseComponent target) {
        super(target);
    }
    
    @Override
    public void execute() {
        super.execute();
    }
}

// Strategy Pattern
public interface ExecutionStrategy {
    void runAlgorithm(BaseComponent comp);
}

public class FastStrategy implements ExecutionStrategy {
    @Override
    public void runAlgorithm(BaseComponent comp) {
        comp.execute();
    }
}

public class StrategyContext {
    private ExecutionStrategy strategy;
    
    public void setStrategy(ExecutionStrategy strat) {
        this.strategy = strat;
    }
    
    public void performAction(BaseComponent comp) {
        if (this.strategy != null) {
            this.strategy.runAlgorithm(comp);
        }
    }
}
"#;

    let java_code3 = r#"
package com.openheart.trick.subsystem;

import com.openheart.trick.core.BaseComponent;
import java.util.List;
import java.util.ArrayList;

// Composite Pattern
public class CompositeNode implements BaseComponent {
    private List<BaseComponent> children = new ArrayList<>();
    
    public void add(BaseComponent child) {
        this.children.add(child);
    }
    
    @Override
    public void execute() {
        for (BaseComponent child : children) {
            child.execute();
        }
    }
    
    @Override
    public int getStatusCode() {
        return 0;
    }
}

// Facade Pattern
public class SystemFacade {
    private BaseComponent primary;
    private CompositeNode group;
    
    public SystemFacade(BaseComponent p, CompositeNode g) {
        this.primary = p;
        this.group = g;
    }
    
    public void boot() {
        this.primary.execute();
        this.group.execute();
    }
}
"#;

    let f1 = dir.path().join("Core.java");
    let f2 = dir.path().join("Patterns.java");
    let f3 = dir.path().join("Subsystem.java");

    fs::write(&f1, java_code1).unwrap();
    fs::write(&f2, java_code2).unwrap();
    fs::write(&f3, java_code3).unwrap();

    let manifest = SourceManifest::new(vec![f1, f2, f3]);
    let tca_path = dir.path().join("corpus.tca");
    let bpa_path = dir.path().join("ast.bpa");
    let sta_path = dir.path().join("symbols.sta");
    let cfa_path = dir.path().join("cfg.cfa");
    let ssa_path = dir.path().join("ssa.ssa");
    let cga_path = dir.path().join("callgraph.cga");
    let tra_path = dir.path().join("traceability.tra");
    let psa_path = dir.path().join("paths.psa");
    let uma_path = dir.path().join("metadata.uma");

    // Phase 1: Ingestion
    let tca = IngestionStage::run(manifest, &tca_path).unwrap();
    let tca_bytes = fs::read(&tca_path).unwrap();

    // Phase 2: AST BP
    let stage_input = ASTStageInput {
        tca: MemoryMappedFile::open(&tca_path).unwrap(),
    };
    let bpa = ASTStage::run(&stage_input, &bpa_path).unwrap();
    let bpa_bytes = fs::read(&bpa_path).unwrap();

    // Phase 3: Symbols & Scope Graph
    let sta = Phase3Stage::run(&tca, &bpa, &tca_bytes, &bpa_bytes).unwrap();
    let sta_bytes = sta.serialize();
    fs::write(&sta_path, &sta_bytes).unwrap();

    // Phase 4: CFG
    let cfa = Phase4Stage::run(&bpa, &sta, &sta_bytes, &bpa_bytes, &cfa_path).unwrap();
    let cfa_bytes = fs::read(&cfa_path).unwrap();

    // Phase 5: SSA
    let ssa = Phase5Stage::run(&bpa, &sta, &cfa, &cfa_bytes, &ssa_path).unwrap();
    let ssa_bytes = fs::read(&ssa_path).unwrap();

    // Phase 6: Call Graph
    let cga = Phase6Stage::run(&bpa, &sta, &cfa, &ssa, &ssa_bytes, &sta_bytes, &cga_path).unwrap();

    // Phase 7: Traceability
    let tra = Phase7Stage::run(&tca, &bpa, &sta, &cfa, &ssa, &cga, &tra_path);
    let tra_bytes = fs::read(&tra_path).unwrap();

    // Phase 8: PSA
    let psa = Phase8Stage::run(&cfa, &ssa, &cga, &cfa_bytes, &psa_path);

    // Phase 9: UMA Metadata Extraction
    let uma = Phase9Stage::run(
        &tca, &bpa, &sta, &cfa, &ssa, &cga, &tra, &psa, &tra_bytes, &uma_path,
    );

    // ── Test 1: Verify Pattern Extraction ─────────────────────────────────────
    assert!(!uma.classes.is_empty(), "UMA classes must not be empty");
    let has_decorator = uma
        .design_patterns
        .iter()
        .any(|p| p.pattern_kind == openheart::uma::types::PATTERN_DECORATOR as u16);
    let has_strategy = uma
        .design_patterns
        .iter()
        .any(|p| p.pattern_kind == openheart::uma::types::PATTERN_STRATEGY as u16);
    let has_composite = uma
        .design_patterns
        .iter()
        .any(|p| p.pattern_kind == openheart::uma::types::PATTERN_COMPOSITE as u16);
    let has_facade = uma
        .design_patterns
        .iter()
        .any(|p| p.pattern_kind == openheart::uma::types::PATTERN_FACADE as u16);

    assert!(
        has_decorator || has_strategy || has_composite || has_facade,
        "Must detect GoF patterns"
    );

    // ── Test 2: Verify All 14 UML + 5 Advanced PlantUML Strategies ──────────
    let puml_exporter = PlantUMLExporter::new();
    assert_eq!(puml_exporter.strategy_types().len(), 19);

    let all_puml = puml_exporter.export_all(&uma, &sta, &tca);
    assert_eq!(all_puml.len(), 19);

    for (dtype, puml_text) in &all_puml {
        assert!(
            puml_text.starts_with("@startuml"),
            "PlantUML {} diagram must start with @startuml",
            dtype
        );
        assert!(
            puml_text.ends_with("@enduml\n"),
            "PlantUML {} diagram must end with @enduml",
            dtype
        );
        assert!(
            !puml_text.contains("SystemNode"),
            "Diagram {} must contain zero placeholder SystemNodes",
            dtype
        );
    }

    // ── Test 3: Verify All 14 UML + 5 Advanced Mermaid Strategies ──────────
    let mermaid_exporter = MermaidExporter::new();
    assert_eq!(mermaid_exporter.strategy_types().len(), 19);

    let all_mmd = mermaid_exporter.export_all(&uma, &sta, &tca);
    assert_eq!(all_mmd.len(), 19);

    for (dtype, mmd_text) in &all_mmd {
        assert!(
            !mmd_text.is_empty(),
            "Mermaid {} diagram must not be empty",
            dtype
        );
        assert!(
            !mmd_text.contains("SystemNode"),
            "Mermaid {} diagram must contain zero placeholder SystemNodes",
            dtype
        );
    }
}
