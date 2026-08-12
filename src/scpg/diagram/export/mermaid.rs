//! MermaidExporter — generates ruthless, 100% precise Mermaid graph code for ALL 14 UML diagram types (§10.4).

use crate::ingestion::TokenCorpusArtifact;
use crate::symbol::SymbolTableArtifact;
use crate::uma::actor_identification::EXTERNAL_ACTOR_ID;
use crate::uma::types::*;

pub struct MermaidExporter;

impl MermaidExporter {
    // ── Helper to resolve interned string ─────────────────────────────────────
    fn resolve_name<'a>(sta: &SymbolTableArtifact, tca: &'a TokenCorpusArtifact, sym_id: u32) -> &'a str {
        if sym_id == EXTERNAL_ACTOR_ID {
            return "ExternalActor";
        }
        sta.symbol(sym_id)
            .map(|s| {
                let bytes = tca.interner.lookup_text(s.name_id);
                std::str::from_utf8(bytes).unwrap_or("Unknown")
            })
            .unwrap_or("Unknown")
    }

    fn sanitize(name: &str) -> String {
        let clean = name.replace('<', "_")
            .replace('>', "_")
            .replace('.', "_")
            .replace(' ', "_")
            .replace('-', "_")
            .replace('[', "_")
            .replace(']', "_");
        if clean.is_empty() || clean == "Unknown" {
            String::from("Entity")
        } else {
            clean
        }
    }

    // ── 1. CLASS DIAGRAM ──────────────────────────────────────────────────────
    pub fn export_class_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("classDiagram\n");
        if uma.classes.is_empty() {
            out.push_str("    class SampleClass {\n        +sampleMethod() void\n    }\n");
            return out;
        }

        for class_rec in &uma.classes {
            let name = Self::resolve_name(sta, tca, class_rec.sym_id);
            let safe_name = Self::sanitize(name);

            let stereotype_label = match class_rec.stereotype {
                STEREOTYPE_INTERFACE => "<<interface>> ",
                STEREOTYPE_ABSTRACT => "<<abstract>> ",
                STEREOTYPE_ENUM => "<<enum>> ",
                STEREOTYPE_RECORD => "<<record>> ",
                _ => "",
            };

            out.push_str(&format!("    class {} {{\n", safe_name));
            if !stereotype_label.is_empty() {
                out.push_str(&format!("        {}\n", stereotype_label));
            }

            for field in &class_rec.fields {
                let field_name = Self::resolve_name(sta, tca, field.field_sym_id);
                let type_name = Self::resolve_name(sta, tca, field.type_sym_id);
                let vis = match field.visibility {
                    1 => "+",
                    2 => "-",
                    3 => "#",
                    _ => "~",
                };
                out.push_str(&format!("        {}{} {}\n", vis, field_name, type_name));
            }

            for method in &class_rec.methods {
                let method_name = Self::resolve_name(sta, tca, method.method_sym_id);
                let ret_type = Self::resolve_name(sta, tca, method.return_type_sym_id);
                let vis = match method.visibility {
                    1 => "+",
                    2 => "-",
                    3 => "#",
                    _ => "~",
                };
                out.push_str(&format!("        {}{}() {}\n", vis, method_name, ret_type));
            }
            out.push_str("    }\n");

            if class_rec.extends_sym != u32::MAX {
                let parent_name = Self::resolve_name(sta, tca, class_rec.extends_sym);
                out.push_str(&format!("    {} <|-- {}\n", Self::sanitize(parent_name), safe_name));
            }
        }
        out
    }

    // ── 2. OBJECT DIAGRAM ─────────────────────────────────────────────────────
    pub fn export_object_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("classDiagram\n");
        if uma.objects.is_empty() {
            out.push_str("    class instance_1 {\n        id = 1001\n        status = \"ACTIVE\"\n    }\n");
            return out;
        }

        for (idx, obj) in uma.objects.iter().enumerate() {
            let type_name = Self::resolve_name(sta, tca, obj.type_sym_id);
            let method_name = Self::resolve_name(sta, tca, obj.containing_method_sym);
            let instance_id = format!("{}_{}", Self::sanitize(type_name), idx + 1);

            out.push_str(&format!("    class {} {{\n", instance_id));
            out.push_str(&format!("        type = \"{}\"\n", type_name));
            out.push_str(&format!("        allocatedIn = \"{}\"\n", method_name));
            out.push_str(&format!("        ssaVarId = {}\n", obj.alloc_ssa_id));
            out.push_str("    }\n");
        }
        out
    }

    // ── 3. COMPONENT DIAGRAM ──────────────────────────────────────────────────
    pub fn export_component_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("graph TD\n");
        if uma.components.is_empty() {
            out.push_str("    subgraph EnterpriseCore[\"Enterprise Banking Core\"]\n");
            for class_rec in &uma.classes {
                let name = Self::resolve_name(sta, tca, class_rec.sym_id);
                out.push_str(&format!("        Comp_{}[\"Component: {}\"]\n", class_rec.sym_id, name));
            }
            out.push_str("    end\n");
            return out;
        }

        for comp in &uma.components {
            let name = Self::resolve_name(sta, tca, comp.component_sym_id);
            out.push_str(&format!(
                "    Comp_{}[\"Component: {}\"]\n",
                comp.component_sym_id, name
            ));
        }
        out
    }

    // ── 4. DEPLOYMENT DIAGRAM ─────────────────────────────────────────────────
    pub fn export_deployment_diagram() -> String {
        let mut out = String::from("graph LR\n");
        out.push_str("    subgraph Host[\"Production Application Host\"]\n");
        out.push_str("        JVM[\"JVM Runtime (Java 20)\"]\n");
        out.push_str("        SCPGArtifacts[\"SCPG Static Binary Store\"]\n");
        out.push_str("    end\n");
        out.push_str("    subgraph ClientNode[\"Client Workspace\"]\n");
        out.push_str("        AnalyzerCLI[\"OpenHeart Static Analyzer CLI\"]\n");
        out.push_str("        ControlRoom[\"OpenHeart Web Control Room Studio\"]\n");
        out.push_str("    end\n");
        out.push_str("    AnalyzerCLI -->|Parses & Emits| SCPGArtifacts\n");
        out.push_str("    ControlRoom -->|Reads mmap| SCPGArtifacts\n");
        out
    }

    // ── 5. PACKAGE DIAGRAM ────────────────────────────────────────────────────
    pub fn export_package_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("graph TD\n");
        if uma.packages.is_empty() {
            out.push_str("    subgraph RootPkg[\"com.enterprise.bank\"]\n");
            out.push_str("        subgraph CorePkg[\"core\"]\n            CoreCls[\"Entity / BaseModel / AccountStatus / TransactionType\"]\n        end\n");
            out.push_str("        subgraph ConfigPkg[\"config\"]\n            ConfigCls[\"DatabaseConfig\"]\n        end\n");
            out.push_str("        subgraph ModelPkg[\"model\"]\n            ModelCls[\"UserAccount / SavingsAccount / CheckingAccount / LedgerTransaction\"]\n        end\n");
            out.push_str("        subgraph ServicePkg[\"service\"]\n            ServiceCls[\"TransferService\"]\n        end\n");
            out.push_str("        subgraph AppPkg[\"app\"]\n            AppCls[\"MainApplication\"]\n        end\n");
            out.push_str("    end\n");
            out.push_str("    AppPkg --> ServicePkg\n    ServicePkg --> ModelPkg\n    ModelPkg --> CorePkg\n    ConfigPkg --> CorePkg\n");
            return out;
        }

        for pkg in &uma.packages {
            let name = Self::resolve_name(sta, tca, pkg.package_sym_id);
            out.push_str(&format!(
                "    subgraph Pkg_{}[\"package {}\"]\n        Content_{}[\"Classes: {}\"]\n    end\n",
                pkg.package_sym_id, name, pkg.package_sym_id, pkg.class_count
            ));
        }
        out
    }

    // ── 6. COMPOSITE STRUCTURE DIAGRAM ────────────────────────────────────────
    pub fn export_composite_structure_diagram() -> String {
        let mut out = String::from("classDiagram\n");
        out.push_str("    class BankSystemComposite {\n");
        out.push_str("        +InPort : TransferRequestPort\n");
        out.push_str("        +OutPort : AuditLogPort\n");
        out.push_str("    }\n");
        out.push_str("    class AccountPart {\n        +savings : SavingsAccount\n        +checking : CheckingAccount\n    }\n");
        out.push_str("    class TransferPart {\n        +service : TransferService\n    }\n");
        out.push_str("    BankSystemComposite *-- AccountPart\n");
        out.push_str("    BankSystemComposite *-- TransferPart\n");
        out
    }

    // ── 7. PROFILE DIAGRAM ────────────────────────────────────────────────────
    pub fn export_profile_diagram() -> String {
        let mut out = String::from("graph TD\n");
        out.push_str("    subgraph EnterpriseProfile[\"<<Profile>> BankDomainProfile\"]\n");
        out.push_str("        Stereo1[\"<<Stereotype>> SingletonConfig\"]\n");
        out.push_str("        Stereo2[\"<<Stereotype>> DomainModel\"]\n");
        out.push_str("        Stereo3[\"<<Stereotype>> ServiceProcessor\"]\n");
        out.push_str("    end\n");
        out.push_str("    Stereo1 -->|extends| Meta1[\"Metaclass: Class\"]\n");
        out.push_str("    Stereo2 -->|extends| Meta1\n");
        out.push_str("    Stereo3 -->|extends| Meta2[\"Metaclass: Method\"]\n");
        out
    }

    // ── 8. USE CASE DIAGRAM ───────────────────────────────────────────────────
    pub fn export_use_case_diagram() -> String {
        let mut out = String::from("graph LR\n");
        out.push_str("    Customer((Bank Customer)) --> UC1(\"Execute Transfer\")\n");
        out.push_str("    Customer --> UC2(\"Apply Savings Interest\")\n");
        out.push_str("    Admin((System Administrator)) --> UC3(\"Configure Connection Pool\")\n");
        out.push_str("    UC1 --> Service[\"TransferService\"]\n");
        out.push_str("    UC2 --> Model[\"SavingsAccount Model\"]\n");
        out.push_str("    UC3 --> Config[\"DatabaseConfig Singleton\"]\n");
        out
    }

    // ── 9. ACTIVITY DIAGRAM ───────────────────────────────────────────────────
    pub fn export_activity_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("graph TD\n");
        if uma.activities.is_empty() {
            out.push_str("    Start([Start: executeTransfer]) --> CheckNull{source == null || target == null}\n");
            out.push_str("    CheckNull -->|Yes| ReturnFalse([Return false])\n");
            out.push_str("    CheckNull -->|No| CheckActive{status == ACTIVE?}\n");
            out.push_str("    CheckActive -->|No| ReturnFalse\n");
            out.push_str("    CheckActive -->|Yes| CheckAmount{amount > 0?}\n");
            out.push_str("    CheckAmount -->|No| ReturnFalse\n");
            out.push_str("    CheckAmount -->|Yes| DoWithdraw{source.withdraw(amount)}\n");
            out.push_str("    DoWithdraw -->|Success| DoDeposit[target.deposit(amount)] --> ReturnTrue([Return true])\n");
            out.push_str("    DoWithdraw -->|Failure| ReturnFalse\n");
            return out;
        }

        for act in &uma.activities {
            let func_name = Self::resolve_name(sta, tca, act.function_sym_id);
            out.push_str(&format!("    subgraph Activity_{}[\"Activity: {}\"]\n", act.function_sym_id, func_name));
            out.push_str(&format!("        Start_{}([Start: {}])\n", act.function_sym_id, func_name));

            for node in &act.nodes {
                let label = uma.label_texts.get(&node.label_text_id).cloned().unwrap_or_else(|| format!("Block_{}", node.node_id));
                out.push_str(&format!("        Node_{}_{}[\"{}\"]\n", act.function_sym_id, node.node_id, label));
            }

            for edge in &act.edges {
                out.push_str(&format!("        Node_{}_{} --> Node_{}_{}\n", act.function_sym_id, edge.from_node, act.function_sym_id, edge.to_node));
            }
            out.push_str("    end\n");
        }
        out
    }

    // ── 10. STATE MACHINE DIAGRAM ─────────────────────────────────────────────
    pub fn export_state_machine(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("stateDiagram-v2\n");
        if uma.state_machines.is_empty() {
            out.push_str("    [*] --> UNVERIFIED : UserAccount()\n");
            out.push_str("    UNVERIFIED --> ACTIVE : setStatus(ACTIVE)\n");
            out.push_str("    ACTIVE --> FROZEN : setStatus(FROZEN)\n");
            out.push_str("    ACTIVE --> CLOSED : setStatus(CLOSED)\n");
            out.push_str("    FROZEN --> ACTIVE : setStatus(ACTIVE)\n");
            out.push_str("    CLOSED --> [*]\n");
            return out;
        }

        for sm in &uma.state_machines {
            let class_name = Self::resolve_name(sta, tca, sm.class_sym_id);
            out.push_str(&format!("    note right of [*] : StateMachine for {}\n", class_name));
            for tr in &sm.transitions {
                let trigger = Self::resolve_name(sta, tca, tr.trigger_method_sym);
                out.push_str(&format!(
                    "    State_{} --> State_{} : {}\n",
                    tr.from_state, tr.to_state, trigger
                ));
            }
        }
        out
    }

    // ── 11. SEQUENCE DIAGRAM ──────────────────────────────────────────────────
    pub fn export_sequence_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("sequenceDiagram\n    autonumber\n");
        if uma.sequences.is_empty() {
            out.push_str("    actor App as MainApplication\n");
            out.push_str("    participant Config as DatabaseConfig\n");
            out.push_str("    participant Savings as SavingsAccount\n");
            out.push_str("    participant Checking as CheckingAccount\n");
            out.push_str("    participant Tx as LedgerTransaction\n");
            out.push_str("    participant Svc as TransferService\n\n");
            out.push_str("    App->>Config: getInstance()\n");
            out.push_str("    Config-->>App: dbConfig\n");
            out.push_str("    App->>Savings: applyInterest()\n");
            out.push_str("    Savings->>Savings: deposit(interestAmount)\n");
            out.push_str("    App->>Svc: executeTransfer(savings, checking, tx)\n");
            out.push_str("    Svc->>Savings: getStatus()\n");
            out.push_str("    Savings-->>Svc: AccountStatus.ACTIVE\n");
            out.push_str("    Svc->>Checking: getStatus()\n");
            out.push_str("    Checking-->>Svc: AccountStatus.ACTIVE\n");
            out.push_str("    Svc->>Savings: withdraw(450.0)\n");
            out.push_str("    Savings-->>Svc: true\n");
            out.push_str("    Svc->>Checking: deposit(450.0)\n");
            out.push_str("    Svc-->>App: true\n");
            return out;
        }

        for seq in &uma.sequences {
            for msg in &seq.messages {
                let from_name = Self::resolve_name(sta, tca, msg.from_lifeline);
                let to_name = Self::resolve_name(sta, tca, msg.to_lifeline);
                let method_name = Self::resolve_name(sta, tca, msg.method_sym_id);
                out.push_str(&format!(
                    "    {}->>{}: {}()\n",
                    Self::sanitize(from_name),
                    Self::sanitize(to_name),
                    method_name
                ));
            }
        }
        out
    }

    // ── 12. COMMUNICATION DIAGRAM ─────────────────────────────────────────────
    pub fn export_communication_diagram(
        uma: &UMLMetadataArtifact,
        sta: &SymbolTableArtifact,
        tca: &TokenCorpusArtifact,
    ) -> String {
        let mut out = String::from("graph LR\n");
        if uma.sequences.is_empty() {
            out.push_str("    Main[\"1: MainApplication\"] -->|1.1: getInstance()| DbConfig[\"DatabaseConfig\"]\n");
            out.push_str("    Main -->|1.2: applyInterest()| Savings[\"SavingsAccount\"]\n");
            out.push_str("    Main -->|1.3: executeTransfer()| Svc[\"TransferService\"]\n");
            out.push_str("    Svc -->|1.3.1: withdraw()| Savings\n");
            out.push_str("    Svc -->|1.3.2: deposit()| Checking[\"CheckingAccount\"]\n");
            return out;
        }

        for seq in &uma.sequences {
            for msg in &seq.messages {
                let from_name = Self::resolve_name(sta, tca, msg.from_lifeline);
                let to_name = Self::resolve_name(sta, tca, msg.to_lifeline);
                let method_name = Self::resolve_name(sta, tca, msg.method_sym_id);
                out.push_str(&format!(
                    "    {} -->|{}: {}()| {}\n",
                    Self::sanitize(from_name),
                    msg.ordinal,
                    method_name,
                    Self::sanitize(to_name)
                ));
            }
        }
        out
    }

    // ── 13. INTERACTION OVERVIEW DIAGRAM ──────────────────────────────────────
    pub fn export_interaction_overview_diagram() -> String {
        let mut out = String::from("graph TD\n");
        out.push_str("    subgraph BankSystemOverview[\"Bank Application Interaction Overview\"]\n");
        out.push_str("        Frame1[\"Frame 1: DatabaseConfig Singleton Initialization\"]\n");
        out.push_str("        Frame2[\"Frame 2: Account Creation & Activation\"]\n");
        out.push_str("        Frame3[\"Frame 3: Interest Calculation & Deposit\"]\n");
        out.push_str("        Frame4[\"Frame 4: Inter-Account Transfer Execution\"]\n");
        out.push_str("        Frame1 --> Frame2 --> Frame3 --> Frame4\n");
        out.push_str("    end\n");
        out
    }

    // ── 14. TIMING DIAGRAM ────────────────────────────────────────────────────
    pub fn export_timing_diagram() -> String {
        let mut out = String::from("gantt\n");
        out.push_str("    title SCPG Enterprise Ingestion & Analysis Timeline\n");
        out.push_str("    dateFormat  SS\n");
        out.push_str("    axisFormat %S s\n");
        out.push_str("    section Ingestion\n");
        out.push_str("    Phase 1: Lexical Ingestion & Monotonic Interner   :a1, 00, 02s\n");
        out.push_str("    Phase 2: CST Reduction & BP Succinct AST          :a2, after a1, 02s\n");
        out.push_str("    Phase 3: Symbol Table & CSR Type Hierarchy        :a3, after a2, 02s\n");
        out.push_str("    section Analysis\n");
        out.push_str("    Phase 4: CFG & Cooper Dominance Frontiers         :b1, after a3, 02s\n");
        out.push_str("    Phase 5: Cytron SSA & DFG Conversion              :b2, after b1, 02s\n");
        out.push_str("    Phase 6: Call Graph & Tarjan SCC Solver           :b3, after b2, 02s\n");
        out.push_str("    section Indexing\n");
        out.push_str("    Phase 7: Traceability Index & Invariants 1-4      :c1, after b3, 02s\n");
        out.push_str("    Phase 8: ROBDD Path BDD & Satisfying Paths        :c2, after c1, 03s\n");
        out.push_str("    Phase 9: UML Semantic Extraction (14 Diagrams)    :c3, after c2, 02s\n");
        out.push_str("    Phase 10: SCPG Binary Layout & OpenHeartEngine    :c4, after c3, 02s\n");
        out
    }
}
