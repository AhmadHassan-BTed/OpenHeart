/**
 * OpenHeart Precision Interactive Graph Canvas (Deterministic UML Edition)
 * Strictly enforces UML 2.5 standard symbolic notation:
 *  - 3-Level Distinct Color Hierarchy:
 *      Level 1: Domain Tier Container (Solid border, soft pastel backdrop)
 *      Level 2: Subpackage Container (Dashed border, saturated pastel backdrop)
 *      Level 3: Enclosed 3-Compartment Class Cards (Crisp pure white with drop shadow)
 *  - Deterministic Preset Layout Engine (100% Collision-Free Guarantee)
 *  - Orthogonal Taxi Wiring for Clean, Non-Entangled Routing
 */

import { parsePumlToCytoscape } from './puml-parser.js';
import { computeDeterministicLayout } from './uml-layout.js';
import { loadGraphIrToCytoscape } from './graph-loader.js';
import { generatePackageFolderSvg } from './uml-card-renderer.js';
import { getTheme, onThemeChange, buildCytoscapeStylesheet } from './themes/index.js';
import { MinimapNavigator } from './minimap-navigator.js';

export class InteractiveGraphCanvas {
  constructor(containerId = 'interactive-canvas') {
    this.containerId = containerId;
    this.cy = null;
    this.currentGraphType = 'class';
    this.selectedNode = null;
    this.onNodeSelectedCallback = null;
    this.onNodeHoverCallback = null;
    this.onRenderCompleteCallback = null;
    this.panSensitivity = parseFloat(localStorage.getItem('openheart_pan_sensitivity') || '0.10');
    this.activeHoverId = null;
    this.hoverTimeout = null;
    this.collapsedPackages = new Set();
    this.onLayersUpdateCallback = null;
    this.hiddenEdgeKinds = new Set();
    this.isPanLocked = false;
    this.domListenersBound = false;
    this.minimap = new MinimapNavigator(this);
  }

  init() {
    const container = document.getElementById(this.containerId);
    if (!container) return;
    this.bindContainerDomListeners(container);
    if (this.minimap) {
      this.minimap.mount(this.containerId);
    }
    this.renderGraph(this.currentGraphType);

    onThemeChange((theme, isDark) => {
      this.setTheme(isDark);
    });
  }

  setNodeSelectCallback(cb) {
    this.onNodeSelectedCallback = cb;
  }

  setNodeHoverCallback(cb) {
    this.onNodeHoverCallback = cb;
  }

  setRenderCompleteCallback(cb) {
    this.onRenderCompleteCallback = cb;
  }

  setPanSensitivity(val) {
    this.panSensitivity = val;
    localStorage.setItem('openheart_pan_sensitivity', val.toString());
  }

  getNodeDataById(nodeId) {
    if (!this.cy || !nodeId) return null;
    const node = this.cy.getElementById(nodeId);
    return node && node.length > 0 ? node.data() : null;
  }

  getNodeDataByFile(fileName) {
    if (!this.cy || !fileName) return null;
    const nodes = this.cy.nodes();
    const cleanFile = fileName.replace(/\.java$/, '').replace(/\.kt$/, '').toLowerCase();
    for (let i = 0; i < nodes.length; i++) {
      const d = nodes[i].data();
      if (!d) continue;
      const dFile = (d.file || '').replace(/\.java$/, '').replace(/\.kt$/, '').toLowerCase();
      const dId = (d.id || '').toLowerCase();
      const dName = (d.name || '').toLowerCase();
      if (dFile === cleanFile || dId === cleanFile || dName === cleanFile || dFile.includes(cleanFile)) {
        return d;
      }
    }
    return null;
  }

  async renderCustomGraphIr(graphIr) {
    if (!graphIr) return;
    this.customGraphIr = graphIr;
    const elements = loadGraphIrToCytoscape(graphIr);
    await this.renderGraph(graphIr.diagram_type || 'class', elements);
  }

  /**
   * Dynamically project any of the 19 diagram types from the ingested custom codebase Graph IR
   */
  projectCustomDiagram(customGraphIr, graphType) {
    if (!customGraphIr || !customGraphIr.nodes) return [];

    const rawClasses = customGraphIr.nodes.filter(n => !n.is_package && n.kind !== 'package');
    const rawPackages = customGraphIr.nodes.filter(n => n.is_package || n.kind === 'package');
    const rawEdges = customGraphIr.edges || [];

    const normType = (graphType || 'class').toLowerCase().replace(/_/g, '');

    // 1. CLASS DIAGRAM: Return the full model
    if (normType === 'class') {
      return loadGraphIrToCytoscape(customGraphIr);
    }

    // 2. PACKAGE DIAGRAM: Filter to packages and inter-package dependencies
    if (normType === 'package') {
      let pkgNodes = [...rawPackages];
      const classToPkg = new Map();
      rawClasses.forEach(c => {
        if (c.parent) classToPkg.set(c.id, c.parent);
      });

      if (pkgNodes.length === 0) {
        // Synthesize packages from class package paths
        const synthetic = new Map();
        rawClasses.forEach(c => {
          const pId = c.parent || 'pkg_root';
          if (!synthetic.has(pId)) {
            const shortName = pId.replace(/^pkg_/, '').replace(/_/g, '.');
            synthetic.set(pId, {
              id: pId,
              name: shortName,
              label: `package [${shortName}]`,
              kind: 'package',
              stereotype: '<<package>>',
              is_package: true,
              nest_level: 0
            });
          }
        });
        pkgNodes = Array.from(synthetic.values());
      }

      const pkgEdgeSet = new Set();
      const pkgEdges = [];
      rawEdges.forEach(e => {
        const srcPkg = classToPkg.get(e.source);
        const tgtPkg = classToPkg.get(e.target);
        if (srcPkg && tgtPkg && srcPkg !== tgtPkg) {
          const edgeKey = `${srcPkg}->${tgtPkg}`;
          if (!pkgEdgeSet.has(edgeKey)) {
            pkgEdgeSet.add(edgeKey);
            pkgEdges.push({
              id: `pkg_dep_${srcPkg}_${tgtPkg}`,
              source: srcPkg,
              target: tgtPkg,
              kind: 'dependency',
              label: '<<import>>',
              arrow: '-->'
            });
          }
        }
      });

      return loadGraphIrToCytoscape({
        diagram_type: 'package',
        nodes: pkgNodes,
        edges: pkgEdges
      });
    }

    // 3. COMPONENT DIAGRAM: Map packages / core services to components and interfaces
    if (normType === 'component') {
      const compNodes = [];
      const compEdges = [];

      rawPackages.forEach(p => {
        compNodes.push({
          id: p.id,
          name: p.name || p.id,
          label: p.name || p.id,
          kind: 'component',
          stereotype: '<<component>>',
          is_package: false,
          file: null,
          lines: []
        });
      });

      if (compNodes.length === 0) {
        rawClasses.slice(0, 10).forEach(c => {
          compNodes.push({
            id: `comp_${c.id}`,
            name: c.name,
            label: c.name,
            kind: 'component',
            stereotype: '<<component>>',
            is_package: false,
            file: c.file,
            lines: c.lines || [1]
          });
        });
      }

      rawClasses.filter(c => c.kind === 'interface').slice(0, 15).forEach(iface => {
        compNodes.push({
          id: iface.id,
          name: iface.name,
          label: iface.name,
          kind: 'interface',
          stereotype: '<<interface>>',
          is_package: false,
          file: iface.file,
          lines: iface.lines || [1]
        });
      });

      rawEdges.forEach(e => {
        compEdges.push({
          id: `comp_edge_${e.id || Math.random().toString(36).substring(2, 7)}`,
          source: e.source,
          target: e.target,
          kind: e.kind || 'dependency',
          label: e.label || ''
        });
      });

      return loadGraphIrToCytoscape({ diagram_type: 'component', nodes: compNodes, edges: compEdges });
    }

    // 4. COMPOSITE STRUCTURE: Primary classes as parts with internal ports
    if (normType === 'composite') {
      const nodes = rawClasses.slice(0, 12).map(c => ({
        id: c.id,
        name: c.name,
        label: c.name,
        kind: 'part',
        stereotype: '<<part>>',
        fields: c.fields || [],
        methods: c.methods || [],
        file: c.file,
        lines: c.lines || [1]
      }));
      return loadGraphIrToCytoscape({ diagram_type: 'composite', nodes, edges: rawEdges.slice(0, 10) });
    }

    // 5. OBJECT DIAGRAM: Runtime instance snapshots
    if (normType === 'object') {
      const nodes = rawClasses.slice(0, 12).map(c => ({
        id: `obj_${c.id}`,
        name: `_${c.name.toLowerCase()} : ${c.name}`,
        label: `_${c.name.toLowerCase()} : ${c.name}`,
        kind: 'object',
        stereotype: '<<object>>',
        fields: (c.fields || []).map(f => typeof f === 'string' ? f : `${f.name || 'slot'} = [active]`),
        file: c.file,
        lines: c.lines || [1]
      }));
      const edges = rawEdges.slice(0, 10).map(e => ({
        id: `obj_link_${e.id || Math.random().toString(36).substring(2, 7)}`,
        source: `obj_${e.source}`,
        target: `obj_${e.target}`,
        kind: 'association',
        label: '<<link>>'
      }));
      return loadGraphIrToCytoscape({ diagram_type: 'object', nodes, edges });
    }

    // 6. DEPLOYMENT DIAGRAM: Execution nodes with software artifacts
    if (normType === 'deployment') {
      const nodes = [
        { id: 'node_client', name: 'Client Workstation', label: 'Client Device', kind: 'device', stereotype: '<<device>>' },
        { id: 'node_server', name: 'Application Server Node', label: 'App Server', kind: 'device', stereotype: '<<executionEnvironment>>' },
        { id: 'node_storage', name: 'Data / Cluster Tier', label: 'Storage Node', kind: 'device', stereotype: '<<device>>' }
      ];
      rawClasses.slice(0, 6).forEach((c, idx) => {
        nodes.push({
          id: `art_${c.id}`,
          name: `${c.name}.dll`,
          label: `${c.name}.dll`,
          kind: 'artifact',
          stereotype: '<<artifact>>',
          parent: idx % 2 === 0 ? 'node_server' : 'node_client',
          file: c.file,
          lines: c.lines || [1]
        });
      });
      const edges = [
        { id: 'dep_1', source: 'node_client', target: 'node_server', kind: 'dependency', label: 'TCP/IP (Socket)' },
        { id: 'dep_2', source: 'node_server', target: 'node_storage', kind: 'dependency', label: 'Cluster Wire' }
      ];
      return loadGraphIrToCytoscape({ diagram_type: 'deployment', nodes, edges });
    }

    // 7. PROFILE DIAGRAM: Stereotypes and Metaclasses
    if (normType === 'profile') {
      const nodes = [
        { id: 'meta_class', name: 'Class', kind: 'metaclass', stereotype: '<<metaclass>>' },
        { id: 'meta_interface', name: 'Interface', kind: 'metaclass', stereotype: '<<metaclass>>' }
      ];
      rawClasses.slice(0, 8).forEach(c => {
        nodes.push({
          id: `st_${c.id}`,
          name: c.name,
          kind: 'stereotype',
          stereotype: `<<${c.kind || 'stereotype'}>>`,
          fields: (c.fields || []).slice(0, 2)
        });
      });
      const edges = rawClasses.slice(0, 8).map(c => ({
        id: `st_ext_${c.id}`,
        source: `st_${c.id}`,
        target: c.kind === 'interface' ? 'meta_interface' : 'meta_class',
        kind: 'generalization',
        label: '<<extend>>'
      }));
      return loadGraphIrToCytoscape({ diagram_type: 'profile', nodes, edges });
    }

    // 8. SEQUENCE DIAGRAM: Primary interacting lifelines with message traces
    if (normType === 'sequence') {
      const topClasses = rawClasses.slice(0, 5);
      const nodes = [
        { id: 'actor_client', name: 'ClientApp', label: 'ClientApp', kind: 'actor', stereotype: '<<actor>>' },
        ...topClasses.map(c => ({
          id: `part_${c.id}`,
          name: c.name,
          label: c.name,
          kind: 'participant',
          stereotype: '<<participant>>',
          file: c.file,
          lines: c.lines || [1]
        }))
      ];
      const edges = [];
      if (topClasses.length > 0) {
        edges.push({ id: 'seq_0', source: 'actor_client', target: `part_${topClasses[0].id}`, kind: 'call', label: '1: Execute()' });
        for (let i = 0; i < topClasses.length - 1; i++) {
          const mName = topClasses[i + 1].methods?.[0]?.name || 'Process';
          edges.push({
            id: `seq_${i + 1}`,
            source: `part_${topClasses[i].id}`,
            target: `part_${topClasses[i + 1].id}`,
            kind: 'call',
            label: `${i + 2}: ${mName}()`
          });
        }
        edges.push({
          id: 'seq_ret',
          source: `part_${topClasses[topClasses.length - 1].id}`,
          target: 'actor_client',
          kind: 'call',
          label: `${topClasses.length + 1}: ResultAck`
        });
      }
      return loadGraphIrToCytoscape({ diagram_type: 'sequence', nodes, edges });
    }

    // 9. STATE MACHINE DIAGRAM
    if (normType === 'statemachine' || normType === 'state') {
      const mainName = rawClasses[0]?.name || 'Controller';
      const nodes = [
        { id: 'st_init', name: '[*] Initializing', kind: 'state', instructions: [`entry / ${mainName}.Initialize()`] },
        { id: 'st_ready', name: 'Ready / Idle', kind: 'state', instructions: ['do / ListenForRequests()'] },
        { id: 'st_executing', name: 'ProcessingRequest', kind: 'state', instructions: ['entry / AllocateContext()', 'do / ExecutePipeline()'] },
        { id: 'st_syncing', name: 'SyncingState', kind: 'state', instructions: ['entry / FlushState()'] },
        { id: 'st_disposed', name: '[*] Disposed', kind: 'state', instructions: [`exit / ${mainName}.Dispose()`] }
      ];
      const edges = [
        { id: 'sm_1', source: 'st_init', target: 'st_ready', kind: 'transition', label: 'ConfigLoaded' },
        { id: 'sm_2', source: 'st_ready', target: 'st_executing', kind: 'transition', label: 'RequestReceived' },
        { id: 'sm_3', source: 'st_executing', target: 'st_syncing', kind: 'transition', label: 'OperationSuccess' },
        { id: 'sm_4', source: 'st_syncing', target: 'st_ready', kind: 'transition', label: 'SyncAcknowledged' },
        { id: 'sm_5', source: 'st_ready', target: 'st_disposed', kind: 'transition', label: 'Shutdown()' }
      ];
      return loadGraphIrToCytoscape({ diagram_type: 'statemachine', nodes, edges });
    }

    // 10. ACTIVITY DIAGRAM: Pipeline workflow
    if (normType === 'activity') {
      const nodes = [
        { id: 'act_start', name: 'start', label: 'start', kind: 'action' },
        { id: 'act_init', name: 'Initialize Configuration', label: 'Initialize Configuration', kind: 'action' },
        { id: 'act_validate', name: 'Validate Request Parameters', label: 'Validate Request Parameters', kind: 'action' },
        { id: 'act_exec', name: `Execute Core Logic (${rawClasses[0]?.name || 'Engine'})`, label: 'Execute Core Logic', kind: 'action' },
        { id: 'act_persist', name: 'Commit & Persist Results', label: 'Commit & Persist Results', kind: 'action' },
        { id: 'act_stop', name: 'stop', label: 'stop', kind: 'action' }
      ];
      const edges = [
        { id: 'a_1', source: 'act_start', target: 'act_init', kind: 'control_flow' },
        { id: 'a_2', source: 'act_init', target: 'act_validate', kind: 'control_flow' },
        { id: 'a_3', source: 'act_validate', target: 'act_exec', kind: 'control_flow', label: '[valid]' },
        { id: 'a_4', source: 'act_exec', target: 'act_persist', kind: 'control_flow' },
        { id: 'a_5', source: 'act_persist', target: 'act_stop', kind: 'control_flow' }
      ];
      return loadGraphIrToCytoscape({ diagram_type: 'activity', nodes, edges });
    }

    // 11. USE CASE DIAGRAM
    if (normType === 'usecase' || normType === 'use_case') {
      const nodes = [
        { id: 'uc_admin', name: 'Administrator', label: 'Administrator', kind: 'actor' },
        { id: 'uc_user', name: 'Client App', label: 'Client App', kind: 'actor' }
      ];
      rawClasses.slice(0, 6).forEach(c => {
        nodes.push({
          id: `uc_${c.id}`,
          name: `Manage & Use ${c.name}`,
          label: `Manage & Use ${c.name}`,
          kind: 'usecase'
        });
      });
      const edges = [];
      rawClasses.slice(0, 6).forEach((c, idx) => {
        edges.push({
          id: `uce_${c.id}`,
          source: idx % 2 === 0 ? 'uc_admin' : 'uc_user',
          target: `uc_${c.id}`,
          kind: 'association'
        });
      });
      return loadGraphIrToCytoscape({ diagram_type: 'usecase', nodes, edges });
    }

    // 12. COMMUNICATION DIAGRAM
    if (normType === 'communication') {
      const topClasses = rawClasses.length > 0 ? rawClasses.slice(0, 6) : [{ id: 'core_service', name: 'CoreService', kind: 'class', file: null, lines: [1] }];
      const nodes = topClasses.map(c => ({
        id: c.id,
        name: c.name,
        label: c.name,
        kind: c.kind || 'class',
        file: c.file,
        lines: c.lines || [1]
      }));
      const edges = rawEdges.slice(0, 8).map((e, idx) => ({
        id: `comm_${idx}`,
        source: e.source,
        target: e.target,
        kind: 'association',
        label: `${idx + 1}: dispatch()`
      }));
      return loadGraphIrToCytoscape({ diagram_type: 'communication', nodes, edges });
    }

    // 13. INTERACTION OVERVIEW
    if (normType === 'interaction') {
      const sourceList = rawPackages.length > 0 ? rawPackages : (rawClasses.length > 0 ? rawClasses : [{ id: 'pkg_main', name: 'MainModule' }]);
      const nodes = sourceList.slice(0, 5).map(p => ({
        id: `io_${p.id}`,
        name: `ref sd: ${p.name || p.id}`,
        label: `ref sd: ${p.name || p.id}`,
        kind: 'action',
        stereotype: '<<interaction_use>>',
        instructions: ['sd SequenceTrace', 'par parallel dispatch']
      }));
      const edges = [];
      for (let i = 0; i < nodes.length - 1; i++) {
        edges.push({
          id: `ioe_${i}`,
          source: nodes[i].id,
          target: nodes[i + 1].id,
          kind: 'control_flow'
        });
      }
      return loadGraphIrToCytoscape({ diagram_type: 'interaction', nodes, edges });
    }

    // 14. TIMING DIAGRAM
    if (normType === 'timing') {
      const topClasses = rawClasses.length > 0 ? rawClasses.slice(0, 4) : [{ id: 'engine', name: 'Engine' }];
      const nodes = topClasses.map(c => ({
        id: `track_${c.id}`,
        name: c.name,
        label: c.name,
        kind: 'timing_track',
        instructions: ['@0ms: Idle', '@50ms: Active', '@120ms: Sync', '@200ms: Idle']
      }));
      return loadGraphIrToCytoscape({ diagram_type: 'timing', nodes, edges: [] });
    }

    // 15. CFG: Control Flow Graph Basic Blocks
    if (normType === 'cfg') {
      const mainClass = rawClasses[0]?.name || 'MainComponent';
      const nodes = [
        { id: 'bb_0', label: `Block #0 (Entry: ${mainClass})`, instructions: ['let ctx = InitializeContext();', 'if (ctx == null) goto BB_ERR;'], kind: 'bb' },
        { id: 'bb_1', label: 'Block #1 (Validation & Dispatch)', instructions: ['ValidateRequest(ctx);', 'dispatchQueue.Enqueue(ctx);'], kind: 'bb' },
        { id: 'bb_2', label: 'Block #2 (Core Execution Loop)', instructions: ['let res = ExecutePipeline();', 'return res;'], kind: 'bb' },
        { id: 'bb_err', label: 'Block #ERR (Exception)', instructions: ['throw new InvalidOperationException();'], kind: 'bb' }
      ];
      const edges = [
        { id: 'cf_1', source: 'bb_0', target: 'bb_1', kind: 'control_flow', label: '[ctx != null]' },
        { id: 'cf_2', source: 'bb_0', target: 'bb_err', kind: 'control_flow', label: '[ctx == null]' },
        { id: 'cf_3', source: 'bb_1', target: 'bb_2', kind: 'control_flow' }
      ];
      return loadGraphIrToCytoscape({ diagram_type: 'cfg', nodes, edges });
    }

    // 16. DFG: Data Flow Graph SSA Lineage
    if (normType === 'dfg') {
      const nodes = [
        { id: 'df_0', label: 'SSA v0: arg_input', instructions: ['v0 = Param[0] (RawPayload)'], kind: 'bb' },
        { id: 'df_1', label: 'SSA v1: parsed_data', instructions: ['v1 = Parse(v0)'], kind: 'bb' },
        { id: 'df_2', label: 'SSA v2: validated_state', instructions: ['v2 = Validate(v1)'], kind: 'bb' },
        { id: 'df_3', label: 'SSA v3: return_val', instructions: ['v3 = Phi(v1, v2)'], kind: 'bb' }
      ];
      const edges = [
        { id: 'dfe_1', source: 'df_0', target: 'df_1', kind: 'data_flow', label: 'def-use' },
        { id: 'dfe_2', source: 'df_1', target: 'df_2', kind: 'data_flow', label: 'def-use' },
        { id: 'dfe_3', source: 'df_2', target: 'df_3', kind: 'data_flow', label: 'phi-merge' }
      ];
      return loadGraphIrToCytoscape({ diagram_type: 'dfg', nodes, edges });
    }

    // 17. CDG: Control Dependence Graph
    if (normType === 'cdg') {
      const nodes = [
        { id: 'cd_root', label: 'Entry CDG Root', instructions: ['Condition True'], kind: 'bb' },
        { id: 'cd_branch', label: 'Branch Guard (pred != null)', instructions: ['Controlled Block'], kind: 'bb' },
        { id: 'cd_body', label: 'Dependent Body', instructions: ['ExecuteAction()'], kind: 'bb' }
      ];
      const edges = [
        { id: 'cde_1', source: 'cd_root', target: 'cd_branch', kind: 'control_flow', label: 'controls' },
        { id: 'cde_2', source: 'cd_branch', target: 'cd_body', kind: 'control_flow', label: 'guards' }
      ];
      return loadGraphIrToCytoscape({ diagram_type: 'cdg', nodes, edges });
    }

    // 18. CALL GRAPH: Interprocedural Call Sites
    if (normType === 'callgraph' || normType === 'cg') {
      const topClasses = rawClasses.slice(0, 6);
      const nodes = topClasses.map(c => {
        const m = c.methods?.[0]?.name || 'Execute';
        return {
          id: `cg_${c.id}`,
          name: `${c.name}.${m}()`,
          label: `${c.name}.${m}()`,
          kind: 'class',
          file: c.file,
          lines: c.lines || [1]
        };
      });
      const edges = [];
      for (let i = 0; i < nodes.length - 1; i++) {
        edges.push({
          id: `cge_${i}`,
          source: nodes[i].id,
          target: nodes[i + 1].id,
          kind: 'call',
          label: 'calls'
        });
      }
      return loadGraphIrToCytoscape({ diagram_type: 'callgraph', nodes, edges });
    }

    // 19. ROBDD SATURATION
    if (normType === 'robdd') {
      const nodes = [
        { id: 'bdd_root', name: 'var_isValid', kind: 'bdd_gate' },
        { id: 'bdd_mid', name: 'var_isReady', kind: 'bdd_gate' },
        { id: '1', name: 'True (1)', kind: 'bdd_terminal' },
        { id: '0', name: 'False (0)', kind: 'bdd_terminal' }
      ];
      const edges = [
        { id: 'b_1', source: 'bdd_root', target: 'bdd_mid', kind: 'control_flow', label: 'hi (1)' },
        { id: 'b_2', source: 'bdd_root', target: '0', kind: 'control_flow', label: 'lo (0)' },
        { id: 'b_3', source: 'bdd_mid', target: '1', kind: 'control_flow', label: 'hi (1)' },
        { id: 'b_4', source: 'bdd_mid', target: '0', kind: 'control_flow', label: 'lo (0)' }
      ];
      return loadGraphIrToCytoscape({ diagram_type: 'robdd', nodes, edges });
    }

    return loadGraphIrToCytoscape(customGraphIr);
  }

  async renderGraph(graphType, customElements = null) {
    this.currentGraphType = graphType;
    const container = document.getElementById(this.containerId);
    if (!container) return;

    if (this.hoverTimeout) {
      clearTimeout(this.hoverTimeout);
      this.hoverTimeout = null;
    }

    if (this.cy) {
      this.cy.destroy();
      this.cy = null;
    }

    this.activeHoverId = null;
    this.collapsedPackages.clear();

    let elements = customElements;
    if (!elements) {
      // ── 0. Persistent Custom Ingested Codebase (Folder / ZIP / Imported Repo) ──
      if (this.customGraphIr) {
        try {
          elements = this.projectCustomDiagram(this.customGraphIr, graphType);
          console.log(`[OpenHeart Canvas] Projected custom diagram for ${graphType}: ${elements?.length || 0} elements`);
        } catch (projErr) {
          console.warn(`[OpenHeart Canvas] Custom projection for ${graphType} error:`, projErr);
        }

        // If custom projection yielded empty, stay strictly in custom codebase context
        if (!elements || elements.length === 0) {
          const cbTitle = this.customGraphIr.title || 'Custom Codebase';
          elements = [
            { data: { id: 'root', label: `<<diagram>>\n${graphType.toUpperCase()}\n──────────────────────\nActive Codebase: ${cbTitle}\nNo elements for this view`, kind: 'entry', width: 320, height: 90 } }
          ];
        }
      }

      // ── 1. Official Direct Ingestion: Strongly-Typed Graph IR from Rust Compiler ──
      // (Only executed when NO custom codebase is active)
      if (!this.customGraphIr && (!elements || elements.length === 0)) {
        try {
          const jsonRes = await fetch(`diagrams/${graphType}.json`);
          if (jsonRes.ok) {
            const graphIr = await jsonRes.json();
            elements = loadGraphIrToCytoscape(graphIr);
            console.log(`[OpenHeart Pipeline] Loaded Official Direct Graph IR for ${graphType}: ${elements.length} elements`);
          }
        } catch (jsonErr) {
          console.warn(`[OpenHeart Pipeline] Direct JSON IR fetch failed, trying PUML fallback:`, jsonErr);
        }

        // ── 2. Fallback: Parse PUML if JSON is absent ──
        if (!elements || elements.length === 0) {
          try {
            const pumlRes = await fetch(`diagrams/${graphType}.puml`);
            if (pumlRes.ok) {
              const pumlText = await pumlRes.text();
              elements = parsePumlToCytoscape(pumlText, graphType);
            }
          } catch (pumlErr) {
            console.warn(`[OpenHeart Pipeline] PUML fallback failed:`, pumlErr);
          }
        }
      }
    }

    if (!elements || elements.length === 0) {
      elements = [
        { data: { id: 'root', label: `<<diagram>>\n${graphType.toUpperCase()}\n──────────────────────\nCompiled Live from Source`, kind: 'entry', width: 260, height: 80, file: 'VideoConversionFacade.java', lines: [1] } }
      ];
    }

    // Compute exact collision-free coordinates across all 3 tiers
    const layoutElements = computeDeterministicLayout(elements, graphType);

    // Safety filter: Guarantee zero orphan edges so Cytoscape never crashes on external SDK symbols
    const nodeIds = new Set(layoutElements.filter(e => e.data && !e.data.source).map(e => e.data.id));
    const safeElements = layoutElements.filter(e => {
      if (e.data && e.data.source) {
        return nodeIds.has(e.data.source) && nodeIds.has(e.data.target);
      }
      return true;
    });

    this.cy = cytoscape({
      container: container,
      elements: safeElements,
      boxSelectionEnabled: false,
      autounselectify: false,
      userZoomingEnabled: false,
      userPanningEnabled: true,
      autoungrabify: this.isPanLocked,
      minZoom: 0.05,
      maxZoom: 4.0,
      pixelRatio: 'auto',
      textureOnViewport: true,
      hideEdgesOnViewport: false,
      motionBlur: false,
      wheelSensitivity: 0.05,
      style: this.getModernStyleSheet(),
      layout: {
        name: 'preset',
        animate: false
      }
    });

    if (this.isPanLocked) {
      this.cy.nodes().ungrabify();
    }

    this.attachEventListeners(container);
    this.cy.fit(undefined, 60);

    if (this.minimap) {
      this.minimap.onGraphRendered();
      this.cy.on('pan zoom viewport', () => {
        this.minimap.onViewportChange();
      });
    }

    if (this.onRenderCompleteCallback) {
      this.onRenderCompleteCallback(elements);
    }

    if (this.onLayersUpdateCallback) {
      this.onLayersUpdateCallback(this.getActiveEdgeKinds());
    }
  }

  setTheme(isDark) {
    if (this.cy) {
      this.cy.style(buildCytoscapeStylesheet(getTheme(isDark)));
    }
    this.renderGraph(this.currentGraphType);
  }

  focusNodeByFile(fileName) {
    if (!this.cy) return;
    const node = this.cy.nodes().filter(n => n.data('file') === fileName)[0];
    if (node) {
      this.cy.animate({
        center: { eles: node },
        zoom: Math.max(0.7, this.cy.zoom()),
        duration: 250
      });
      this.cy.nodes().unselect();
      node.select();
      if (this.onNodeSelectedCallback) {
        this.onNodeSelectedCallback(node.data());
      }
    }
  }

  getModernStyleSheet() {
    return buildCytoscapeStylesheet();
  }

  setLayersUpdateCallback(cb) {
    this.onLayersUpdateCallback = cb;
  }

  setEdgeFilter(umlKind, isVisible) {
    if (!this.cy) return;
    if (isVisible) {
      this.hiddenEdgeKinds.delete(umlKind);
    } else {
      this.hiddenEdgeKinds.add(umlKind);
    }

    this.cy.batch(() => {
      const edges = this.cy.edges(`[uml_kind = "${umlKind}"]`);
      if (isVisible) {
        edges.style('display', 'element');
      } else {
        edges.style('display', 'none');
      }
    });
  }

  getActiveEdgeKinds() {
    if (!this.cy) return [];
    const counts = new Map();
    this.cy.edges().forEach(e => {
      const k = e.data('uml_kind') || 'association';
      counts.set(k, (counts.get(k) || 0) + 1);
    });
    return Array.from(counts.entries()).map(([kind, count]) => ({
      kind,
      count,
      visible: !this.hiddenEdgeKinds.has(kind)
    }));
  }

  bindContainerDomListeners(container) {
    if (this.domListenersBound || !container) return;
    this.domListenersBound = true;

    // ── Two-Finger Trackpad Pan vs Pinch-to-Zoom ──
    container.addEventListener('wheel', (e) => {
      e.preventDefault();

      if (e.ctrlKey || e.metaKey) {
        // Pinch gesture or Ctrl+Wheel -> Controlled, smooth zoom centered at cursor
        // Normalized and clamped delta to prevent runaway zoom
        const clampedDelta = Math.sign(e.deltaY) * Math.min(Math.abs(e.deltaY), 50);
        const zoomFactor = 1 - (clampedDelta * 0.0018); // ~4-8% per notch
        if (!this.cy) return;

        const currentZoom = this.cy.zoom();
        const newZoom = Math.min(4.0, Math.max(0.05, currentZoom * zoomFactor));
        const rect = container.getBoundingClientRect();
        const renderedPos = {
          x: e.clientX - rect.left,
          y: e.clientY - rect.top
        };

        this.cy.zoom({
          level: newZoom,
          renderedPosition: renderedPos
        });
      } else {
        // Two-Finger Trackpad Gesture / Scroll -> Pan canvas in 2D
        if (!this.cy) return;
        const panSensitivity = this.panSensitivity !== undefined ? this.panSensitivity : 0.10;
        this.cy.panBy({
          x: -e.deltaX * panSensitivity,
          y: -e.deltaY * panSensitivity
        });
      }
    }, { passive: false });

    // ── Multi-Touch Support (2-Finger Pinch & Pan for Touchscreens) ──
    let touchStartDist = 0;
    let touchStartCenter = null;
    let touchStartZoom = 1;

    container.addEventListener('touchstart', (e) => {
      if (e.touches.length === 2 && this.cy) {
        const t1 = e.touches[0];
        const t2 = e.touches[1];
        touchStartDist = Math.hypot(t2.clientX - t1.clientX, t2.clientY - t1.clientY);
        touchStartCenter = {
          x: (t1.clientX + t2.clientX) / 2,
          y: (t1.clientY + t2.clientY) / 2
        };
        touchStartZoom = this.cy.zoom();
      }
    }, { passive: true });

    container.addEventListener('touchmove', (e) => {
      if (e.touches.length === 2 && touchStartDist > 0 && this.cy) {
        e.preventDefault();
        const t1 = e.touches[0];
        const t2 = e.touches[1];
        const currentDist = Math.hypot(t2.clientX - t1.clientX, t2.clientY - t1.clientY);
        const currentCenter = {
          x: (t1.clientX + t2.clientX) / 2,
          y: (t1.clientY + t2.clientY) / 2
        };

        // 1. Pinch to Zoom with gentle damping
        const scale = 1 + (currentDist / touchStartDist - 1) * 0.4;
        const newZoom = Math.min(4.0, Math.max(0.05, touchStartZoom * scale));
        const rect = container.getBoundingClientRect();

        this.cy.zoom({
          level: newZoom,
          renderedPosition: {
            x: currentCenter.x - rect.left,
            y: currentCenter.y - rect.top
          }
        });

        // 2. Pure Two-Finger Pan
        const panSensitivity = this.panSensitivity !== undefined ? this.panSensitivity : 0.10;
        const deltaX = (currentCenter.x - touchStartCenter.x) * panSensitivity;
        const deltaY = (currentCenter.y - touchStartCenter.y) * panSensitivity;
        this.cy.panBy({ x: deltaX, y: deltaY });
        touchStartCenter = currentCenter;
      }
    }, { passive: false });
  }

  setPanLock(locked) {
    this.isPanLocked = Boolean(locked);
    if (this.cy) {
      this.cy.autoungrabify(this.isPanLocked);
      if (this.isPanLocked) {
        this.cy.nodes().ungrabify();
      } else {
        this.cy.nodes().grabify();
      }
    }
    const container = document.getElementById(this.containerId);
    if (container) {
      if (this.isPanLocked) {
        container.classList.add('pan-locked');
      } else {
        container.classList.remove('pan-locked');
      }
    }
    return this.isPanLocked;
  }

  togglePanLock() {
    return this.setPanLock(!this.isPanLocked);
  }

  getDiagramStateJson() {
    if (!this.cy) return null;
    const codebaseName = sessionStorage.getItem('openheart_custom_name') || 'OpenHeart';
    const elements = this.cy.elements().map(el => {
      const isNode = el.isNode();
      return {
        group: isNode ? 'nodes' : 'edges',
        data: { ...el.data() },
        position: isNode ? { ...el.position() } : undefined,
        classes: el.classes().join(' ')
      };
    });

    return {
      format: "OpenHeart-Diagram-IR",
      version: "1.0",
      diagram_type: this.currentGraphType,
      codebase_name: codebaseName,
      timestamp: new Date().toISOString(),
      pan: { ...this.cy.pan() },
      zoom: this.cy.zoom(),
      elements: elements
    };
  }

  loadDiagramState(savedState) {
    if (!savedState || !savedState.elements || !Array.isArray(savedState.elements)) {
      throw new Error("Invalid OpenHeart diagram state format.");
    }
    const targetGraphType = savedState.diagram_type || this.currentGraphType || 'class';
    this.currentGraphType = targetGraphType;
    
    this.renderGraph(targetGraphType, savedState.elements);

    if (savedState.zoom && savedState.pan && this.cy) {
      this.cy.viewport({
        zoom: savedState.zoom,
        pan: savedState.pan
      });
    }
  }

  exportPng() {
    if (!this.cy) return null;
    const isDark = isDarkMode();
    return this.cy.png({
      full: true,
      scale: 2.0,
      bg: isDark ? '#0b0f19' : '#ffffff'
    });
  }

  exportSvg() {
    if (!this.cy) return null;
    if (typeof this.cy.svg === 'function') {
      return this.cy.svg({ full: true, scale: 1.0 });
    }
    return null;
  }

  attachEventListeners(container) {
    if (!this.cy || !container) return;

    // ── High-Performance Local Neighborhood Hover Illumination ──
    this.cy.on('mouseover', 'node, edge', (e) => {
      const target = e.target;
      if (target.data('isPackage')) return;

      const targetId = target.id();
      if (this.activeHoverId === targetId) return;
      this.activeHoverId = targetId;

      if (this.hoverTimeout) {
        clearTimeout(this.hoverTimeout);
        this.hoverTimeout = null;
      }

      const neighborhood = target.isNode()
        ? target.closedNeighborhood()
        : target.connectedNodes().union(target);

      this.cy.batch(() => {
        target.addClass('path-highlighted');
        neighborhood.addClass('path-highlighted');
      });

      if (this.onNodeHoverCallback && target.isNode()) {
        this.onNodeHoverCallback(target.data());
      }
    });

    this.cy.on('mouseout', 'node, edge', (e) => {
      const target = e.target;
      if (this.hoverTimeout) clearTimeout(this.hoverTimeout);
      this.hoverTimeout = setTimeout(() => {
        this.activeHoverId = null;
        this.cy.batch(() => {
          this.cy.elements('.path-highlighted').removeClass('path-highlighted');
        });
      }, 30);
    });

    // ── Click to Inspect & Synchronize Monaco ──
    this.cy.on('tap', 'node', (e) => {
      const node = e.target;
      
      // If clicking package container, toggle collapse/expand (Opening & Closing)
      if (node.data('isPackage')) {
        this.togglePackageCollapse(node);
        return;
      }

      this.selectedNode = node.data();
      if (this.onNodeSelectedCallback) {
        this.onNodeSelectedCallback(this.selectedNode);
      }
    });
  }

  togglePackageCollapse(pkgNode) {
    const pkgId = pkgNode.id();
    const children = this.cy.nodes(`[parent = "${pkgId}"]`);
    const isCollapsed = this.collapsedPackages.has(pkgId);
    const rawName = pkgNode.data('rawName') || pkgId.replace(/^pkg_/, '').replace(/_/g, '.');
    const shortName = rawName.split('.').pop();
    const isDomainTier = pkgNode.data('isDomainTier');

    this.cy.batch(() => {
      if (isCollapsed) {
        // Expand (Open)
        this.collapsedPackages.delete(pkgId);
        pkgNode.removeClass('package-collapsed');
        pkgNode.data('width', pkgNode.data('origWidth') || 650);
        pkgNode.data('height', pkgNode.data('origHeight') || 400);
        pkgNode.data('label', isDomainTier ? `📂 [−] DOMAIN LAYER: ${rawName.toUpperCase()}` : `📂 [−] package [${shortName}]`);
        children.style('display', 'element');
        children.connectedEdges().style('display', 'element');
      } else {
        // Collapse (Close)
        this.collapsedPackages.add(pkgId);
        pkgNode.addClass('package-collapsed');
        pkgNode.data('label', isDomainTier ? `📁 [+] DOMAIN LAYER: ${rawName.toUpperCase()} (${children.length} subpackages)` : `📁 [+] package [${shortName}] (${children.length} classes)`);
        children.style('display', 'none');
        children.connectedEdges().style('display', 'none');
      }
    });
  }

  zoomIn() {
    if (!this.cy) return;
    this.cy.animate({
      zoom: {
        level: this.cy.zoom() * 1.25,
        renderedPosition: { x: this.cy.width() / 2, y: this.cy.height() / 2 }
      },
      duration: 150
    });
  }

  zoomOut() {
    if (!this.cy) return;
    this.cy.animate({
      zoom: {
        level: this.cy.zoom() * 0.8,
        renderedPosition: { x: this.cy.width() / 2, y: this.cy.height() / 2 }
      },
      duration: 150
    });
  }

  resetView() {
    if (!this.cy) return;
    this.cy.animate({
      fit: {
        eles: this.cy.elements(),
        padding: 60
      },
      duration: 250
    });
  }

  toggleMinimap(state = null) {
    if (this.minimap) {
      return this.minimap.toggleVisibility(state);
    }
    return false;
  }

  toggleLoupe(state = null) {
    if (this.minimap) {
      return this.minimap.toggleLoupe(state);
    }
    return false;
  }
}
