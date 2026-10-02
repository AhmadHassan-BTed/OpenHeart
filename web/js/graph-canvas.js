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
    this.undoStack = [];
    this.redoStack = [];
    this.maxHistory = 50;
    this._hudStatusTimer = null;
  }

  init() {
    const container = document.getElementById(this.containerId);
    if (!container) return;
    this.bindContainerDomListeners(container);
    this.bindHistoryControls();
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

  async renderGraph(graphType, customElements = null, preservePositions = false) {
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

    // Compute exact collision-free coordinates across all 3 tiers (unless restoring saved layout)
    let safeElements;
    if (preservePositions) {
      safeElements = elements;
    } else {
      const layoutElements = computeDeterministicLayout(elements, graphType);
      const nodeIds = new Set(layoutElements.filter(e => e.data && !e.data.source).map(e => e.data.id));
      safeElements = layoutElements.filter(e => {
        if (e.data && e.data.source) {
          return nodeIds.has(e.data.source) && nodeIds.has(e.data.target);
        }
        return true;
      });
    }

    this.cy = cytoscape({
      container: container,
      elements: safeElements,
      boxSelectionEnabled: false,
      autounselectify: false,
      userZoomingEnabled: false,
      userPanningEnabled: true,
      autoungrabify: this.isPanLocked,
      minZoom: 0.04,
      maxZoom: 5.0,
      pixelRatio: 'auto',
      textureOnViewport: false,
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
    if (!preservePositions) {
      this.cy.fit(undefined, 60);
      this.undoStack = [];
      this.redoStack = [];
      this.updateUndoRedoButtons();
    }

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
    if (!this.cy || !fileName) return;
    const baseName = fileName.replace(/\\/g, '/').split('/').pop();
    const node = this.cy.nodes().filter(n => {
      const f = n.data('file');
      const id = n.data('id');
      return f === fileName ||
             (baseName && f && (f.endsWith('/' + baseName) || f === baseName)) ||
             id === fileName ||
             id === baseName;
    })[0];

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

  getNodeDataByFile(fileName) {
    if (!this.cy || !fileName) return null;
    const baseName = fileName.replace(/\\/g, '/').split('/').pop();
    const node = this.cy.nodes().filter(n => {
      const f = n.data('file');
      return f === fileName || (baseName && f && (f.endsWith('/' + baseName) || f === baseName));
    })[0];
    return node ? node.data() : null;
  }

  getNodeDataById(nodeId) {
    if (!this.cy || !nodeId) return null;
    const node = this.cy.getElementById(nodeId);
    return (node && node.length > 0) ? node.data() : null;
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

    // ── 1. Figma-Grade Smooth Navigation Engine (60fps/120fps RAF Inertia & Easing) ──
    const navPhysics = {
      targetPanDx: 0,
      targetPanDy: 0,
      targetZoom: null,
      zoomAnchor: null,
      panRafId: null,
      zoomRafId: null
    };

    const startPanLoop = () => {
      if (navPhysics.panRafId) return;
      const step = () => {
        if (!this.cy) {
          navPhysics.panRafId = null;
          return;
        }

        const remainingX = navPhysics.targetPanDx;
        const remainingY = navPhysics.targetPanDy;

        if (Math.abs(remainingX) < 0.25 && Math.abs(remainingY) < 0.25) {
          if (remainingX !== 0 || remainingY !== 0) {
            this.cy.panBy({ x: remainingX, y: remainingY });
          }
          navPhysics.targetPanDx = 0;
          navPhysics.targetPanDy = 0;
          navPhysics.panRafId = null;
          return;
        }

        // Exponential ease-out friction (smooth, organic glide)
        const moveX = remainingX * 0.28;
        const moveY = remainingY * 0.28;
        this.cy.panBy({ x: moveX, y: moveY });
        navPhysics.targetPanDx -= moveX;
        navPhysics.targetPanDy -= moveY;

        navPhysics.panRafId = requestAnimationFrame(step);
      };
      navPhysics.panRafId = requestAnimationFrame(step);
    };

    const startZoomLoop = () => {
      if (navPhysics.zoomRafId) return;
      const step = () => {
        if (!this.cy || navPhysics.targetZoom === null || !navPhysics.zoomAnchor) {
          navPhysics.zoomRafId = null;
          return;
        }

        const currentZoom = this.cy.zoom();
        const diff = navPhysics.targetZoom - currentZoom;

        if (Math.abs(diff) < 0.001) {
          this.cy.zoom({
            level: navPhysics.targetZoom,
            renderedPosition: navPhysics.zoomAnchor
          });
          navPhysics.targetZoom = null;
          navPhysics.zoomAnchor = null;
          navPhysics.zoomRafId = null;
          return;
        }

        // Smooth ease-out zoom convergence (snappy 80-100ms)
        const nextZoom = currentZoom + diff * 0.45;
        this.cy.zoom({
          level: nextZoom,
          renderedPosition: navPhysics.zoomAnchor
        });

        navPhysics.zoomRafId = requestAnimationFrame(step);
      };
      navPhysics.zoomRafId = requestAnimationFrame(step);
    };

    // ── Prevent Windows Menu Bar / Chrome Shortcut Capture on Alt Key ──
    let isAltHeld = false;

    window.addEventListener('keydown', (e) => {
      if (e.key === 'Alt' || e.code === 'AltLeft' || e.code === 'AltRight') {
        const activeTag = document.activeElement ? document.activeElement.tagName : '';
        if (!['INPUT', 'TEXTAREA'].includes(activeTag) && !document.activeElement?.isContentEditable) {
          e.preventDefault();
          isAltHeld = true;
        }
      }
    }, { capture: true });

    window.addEventListener('keyup', (e) => {
      if (e.key === 'Alt' || e.code === 'AltLeft' || e.code === 'AltRight') {
        const activeTag = document.activeElement ? document.activeElement.tagName : '';
        if (!['INPUT', 'TEXTAREA'].includes(activeTag) && !document.activeElement?.isContentEditable) {
          e.preventDefault();
          isAltHeld = false;
        }
      }
    }, { capture: true });

    window.addEventListener('blur', () => {
      isAltHeld = false;
    });

    // Attach wheel listener to full viewport area (canvas + overlay card)
    const viewportTarget = container.closest('.canvas-card') || container;

    viewportTarget.addEventListener('wheel', (e) => {
      e.preventDefault();
      if (!this.cy) return;

      const rect = container.getBoundingClientRect();
      const pointerPos = {
        x: Math.max(0, Math.min(rect.width, e.clientX - rect.left)),
        y: Math.max(0, Math.min(rect.height, e.clientY - rect.top))
      };

      const userSensitivity = this.panSensitivity !== undefined ? this.panSensitivity : 0.10;
      const speed = userSensitivity * 10.0;

      // Extract raw delta across all OS drivers (some remap Alt+wheel to deltaX or wheelDelta)
      let rawDelta = 0;
      if (Math.abs(e.deltaY) >= Math.abs(e.deltaX) && e.deltaY !== 0) {
        rawDelta = e.deltaY;
      } else if (e.deltaX !== 0) {
        rawDelta = e.deltaX;
      } else if (e.detail) {
        rawDelta = e.detail * 40;
      } else if (e.wheelDelta) {
        rawDelta = -e.wheelDelta;
      }

      if (rawDelta === 0) return;

      const isZoomGesture = e.ctrlKey || e.metaKey || e.altKey || isAltHeld;

      // ── Condition A: ZOOM (Ctrl, Cmd, Alt, or Trackpad Pinch) ──
      if (isZoomGesture) {
        navPhysics.targetPanDx = 0;
        navPhysics.targetPanDy = 0;

        const isTrackpadPinch = Math.abs(rawDelta) < 25 && (e.ctrlKey || e.metaKey);

        if (isTrackpadPinch) {
          // Continuous trackpad pinch: instantaneous 1:1 proportional zoom
          const currentZoom = this.cy.zoom();
          const zoomFactor = Math.exp(-rawDelta * 0.008);
          const newZoom = Math.min(5.0, Math.max(0.04, currentZoom * zoomFactor));
          this.cy.zoom({
            level: newZoom,
            renderedPosition: pointerPos
          });
          navPhysics.targetZoom = null;
        } else {
          // Discrete mouse wheel notch: smooth 10% zoom with organic ease-out glide
          const baseZoom = (navPhysics.targetZoom !== null && !isNaN(navPhysics.targetZoom))
            ? navPhysics.targetZoom
            : this.cy.zoom();
          const zoomMultiplier = rawDelta < 0 ? 1.10 : (1 / 1.10);
          navPhysics.targetZoom = Math.min(5.0, Math.max(0.04, baseZoom * zoomMultiplier));
          navPhysics.zoomAnchor = pointerPos;
          startZoomLoop();
        }
        return;
      }

      // ── Condition B: PANNING (Standard Wheel or Shift + Wheel) ──
      const isDiscreteWheel = e.deltaMode !== 0 || Math.abs(rawDelta) >= 40;

      let dx = 0;
      let dy = 0;

      if (e.shiftKey) {
        // Shift + Wheel -> Horizontal Pan
        dx = -Math.sign(rawDelta) * (isDiscreteWheel ? 54 : Math.abs(rawDelta)) * speed;
        dy = 0;
      } else {
        if (isDiscreteWheel) {
          dx = -Math.sign(e.deltaX || 0) * (e.deltaX ? 54 : 0) * speed;
          dy = -Math.sign(rawDelta) * 54 * speed;
        } else {
          // Smooth continuous trackpad 2-finger scroll
          dx = -e.deltaX * speed;
          dy = -rawDelta * speed;
        }
      }

      if (isDiscreteWheel) {
        navPhysics.targetPanDx += dx;
        navPhysics.targetPanDy += dy;
        startPanLoop();
      } else {
        this.cy.panBy({ x: dx, y: dy });
      }
    }, { passive: false });

    // ── 2 & 3. Unified Pan Drag Engine: Middle-Click, Spacebar Hand Tool, and Pan Lock Active Drag ──
    let isMiddleDragging = false;
    let isSpacePressed = false;
    let isLeftHandDragging = false;
    let leftHandStartPos = { x: 0, y: 0 };
    let hasLeftHandMoved = false;

    let dragRafId = null;
    let pendingDragDx = 0;
    let pendingDragDy = 0;

    const dispatchPendingDrag = () => {
      if (dragRafId) return;
      dragRafId = requestAnimationFrame(() => {
        dragRafId = null;
        if (this.cy && (pendingDragDx !== 0 || pendingDragDy !== 0)) {
          this.cy.panBy({ x: pendingDragDx, y: pendingDragDy });
          pendingDragDx = 0;
          pendingDragDy = 0;
        }
      });
    };

    container.addEventListener('mousedown', (e) => {
      // Middle click pan (button 1)
      if (e.button === 1) {
        e.preventDefault();
        e.stopPropagation();
        isMiddleDragging = true;
        leftHandStartPos = { x: e.clientX, y: e.clientY };
        container.classList.add('cursor-grabbing');
        navPhysics.targetPanDx = 0;
        navPhysics.targetPanDy = 0;
        return;
      }

      // Left click pan when either Pan Lock is active OR Spacebar is held
      // Operates anywhere across canvas, diagrams, box models, packages, and edges
      if (e.button === 0 && (this.isPanLocked || isSpacePressed)) {
        e.preventDefault();
        e.stopPropagation();
        isLeftHandDragging = true;
        leftHandStartPos = { x: e.clientX, y: e.clientY };
        hasLeftHandMoved = false;
        container.classList.remove('cursor-grab');
        container.classList.add('cursor-grabbing');
        navPhysics.targetPanDx = 0;
        navPhysics.targetPanDy = 0;
        return;
      }
    }, { capture: true });

    window.addEventListener('mousemove', (e) => {
      if (isMiddleDragging && this.cy) {
        pendingDragDx += e.clientX - leftHandStartPos.x;
        pendingDragDy += e.clientY - leftHandStartPos.y;
        leftHandStartPos = { x: e.clientX, y: e.clientY };
        dispatchPendingDrag();
        return;
      }

      if (isLeftHandDragging && this.cy) {
        const dx = e.clientX - leftHandStartPos.x;
        const dy = e.clientY - leftHandStartPos.y;
        if (Math.hypot(dx, dy) > 2) {
          hasLeftHandMoved = true;
        }
        pendingDragDx += dx;
        pendingDragDy += dy;
        leftHandStartPos = { x: e.clientX, y: e.clientY };
        dispatchPendingDrag();
        return;
      }
    });

    window.addEventListener('mouseup', (e) => {
      if (e.button === 1 && isMiddleDragging) {
        isMiddleDragging = false;
        container.classList.remove('cursor-grabbing');
        if (this.isPanLocked || isSpacePressed) {
          container.classList.add('cursor-grab');
        }
        return;
      }

      if (e.button === 0 && isLeftHandDragging) {
        isLeftHandDragging = false;
        container.classList.remove('cursor-grabbing');
        if (this.isPanLocked || isSpacePressed) {
          container.classList.add('cursor-grab');
        }

        // If user tapped without dragging, trigger selection/inspection on node under cursor
        if (!hasLeftHandMoved && this.cy) {
          const rect = container.getBoundingClientRect();
          const renderedPos = {
            x: e.clientX - rect.left,
            y: e.clientY - rect.top
          };
          const targetNode = this.getNodeAtRenderedPos(renderedPos);
          if (targetNode) {
            if (targetNode.data('isPackage')) {
              this.togglePackageCollapse(targetNode);
            } else {
              this.selectedNode = targetNode.data();
              this.cy.nodes().unselect();
              targetNode.select();
              if (this.onNodeSelectedCallback) {
                this.onNodeSelectedCallback(this.selectedNode);
              }
            }
          }
        }
        return;
      }
    }, { capture: true });

    // ── 3. Figma Hand Tool (Spacebar Key Listeners) ──
    window.addEventListener('keydown', (e) => {
      if (e.code === 'Space' && !e.repeat) {
        const activeTag = document.activeElement ? document.activeElement.tagName : '';
        if (['INPUT', 'TEXTAREA'].includes(activeTag) || document.activeElement?.isContentEditable) {
          return;
        }
        e.preventDefault();
        isSpacePressed = true;
        container.classList.add('cursor-grab');
        if (this.cy) {
          this.cy.autoungrabify(true);
        }
      }
    });

    window.addEventListener('keyup', (e) => {
      if (e.code === 'Space') {
        isSpacePressed = false;
        isLeftHandDragging = false;
        if (!this.isPanLocked) {
          container.classList.remove('cursor-grab', 'cursor-grabbing');
          if (this.cy) {
            this.cy.autoungrabify(false);
          }
        }
      }
    });

    window.addEventListener('blur', () => {
      isSpacePressed = false;
      isLeftHandDragging = false;
      isMiddleDragging = false;
      if (!this.isPanLocked) {
        container.classList.remove('cursor-grab', 'cursor-grabbing');
        if (this.cy) {
          this.cy.autoungrabify(false);
        }
      }
    });

    // ── 4. Figma Keyboard Shortcuts (Ctrl + / -, Ctrl 0, Shift 1, Arrow Keys) ──
    window.addEventListener('keydown', (e) => {
      const activeTag = document.activeElement ? document.activeElement.tagName : '';
      if (['INPUT', 'TEXTAREA'].includes(activeTag) || document.activeElement?.isContentEditable) {
        return;
      }

      const isCtrl = e.ctrlKey || e.metaKey;
      const isZ = e.key === 'z' || e.key === 'Z' || e.code === 'KeyZ';
      const isY = e.key === 'y' || e.key === 'Y' || e.code === 'KeyY';

      // ── Figma Undo: Ctrl + Z / Cmd + Z (without Shift) ──
      if (isCtrl && !e.shiftKey && isZ) {
        e.preventDefault();
        this.undo();
        return;
      }

      // ── Figma Redo: Ctrl + Shift + Z / Cmd + Shift + Z OR Ctrl + Y / Cmd + Y ──
      if ((isCtrl && e.shiftKey && isZ) || (isCtrl && isY)) {
        e.preventDefault();
        this.redo();
        return;
      }

      // ── Delete / Backspace: Remove selected node with full undo/redo ──
      if ((e.key === 'Delete' || e.key === 'Backspace') && (this.selectedNode || (this.cy && this.cy.nodes(':selected').length > 0))) {
        e.preventDefault();
        this.deleteSelectedNode();
        return;
      }

      // Ctrl/Cmd + = / + : Zoom In
      if (isCtrl && (e.key === '=' || e.key === '+')) {
        e.preventDefault();
        this.zoomIn();
        return;
      }

      // Ctrl/Cmd + - : Zoom Out
      if ((e.ctrlKey || e.metaKey) && (e.key === '-' || e.key === '_')) {
        e.preventDefault();
        this.zoomOut();
        return;
      }

      // Ctrl/Cmd + 0 : Reset Zoom to 100%
      if ((e.ctrlKey || e.metaKey) && e.key === '0') {
        e.preventDefault();
        if (this.cy) {
          this.cy.animate({
            zoom: {
              level: 1.0,
              renderedPosition: { x: this.cy.width() / 2, y: this.cy.height() / 2 }
            },
            duration: 200
          });
        }
        return;
      }

      // Shift + 1 OR Ctrl + 1 : Zoom to Fit (Fit all diagram elements into screen)
      if (((e.shiftKey || e.ctrlKey || e.metaKey) && e.key === '1') || e.key === '!') {
        e.preventDefault();
        this.resetView();
        return;
      }

      // Arrow Keys: Nudge/pan canvas when not inspecting or typing
      if (['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(e.key) && !this.selectedNode) {
        e.preventDefault();
        const dist = e.shiftKey ? 120 : 40;
        const dx = e.key === 'ArrowLeft' ? dist : (e.key === 'ArrowRight' ? -dist : 0);
        const dy = e.key === 'ArrowUp' ? dist : (e.key === 'ArrowDown' ? -dist : 0);
        if (this.cy) {
          this.cy.panBy({ x: dx, y: dy });
        }
      }
    });

    // ── 5. Multi-Touch Support (2-Finger Pinch & Pan for Touchscreens) ──
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

        const scale = 1 + (currentDist / touchStartDist - 1) * 0.4;
        const newZoom = Math.min(5.0, Math.max(0.04, touchStartZoom * scale));
        const rect = container.getBoundingClientRect();

        this.cy.zoom({
          level: newZoom,
          renderedPosition: {
            x: currentCenter.x - rect.left,
            y: currentCenter.y - rect.top
          }
        });

        const userSensitivity = this.panSensitivity !== undefined ? this.panSensitivity : 0.10;
        const speed = userSensitivity * 10.0;
        const deltaX = (currentCenter.x - touchStartCenter.x) * speed;
        const deltaY = (currentCenter.y - touchStartCenter.y) * speed;
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
        container.classList.add('pan-locked', 'cursor-grab');
      } else {
        container.classList.remove('pan-locked', 'cursor-grab', 'cursor-grabbing');
      }
    }
    return this.isPanLocked;
  }

  togglePanLock() {
    return this.setPanLock(!this.isPanLocked);
  }

  getNodeAtRenderedPos(renderedPos) {
    if (!this.cy || !renderedPos) return null;
    const pan = this.cy.pan();
    const zoom = this.cy.zoom();
    const modelX = (renderedPos.x - pan.x) / zoom;
    const modelY = (renderedPos.y - pan.y) / zoom;

    const hitNodes = this.cy.nodes().filter(node => {
      if (node.style('display') === 'none') return false;
      const bb = node.boundingBox();
      return modelX >= bb.x1 && modelX <= bb.x2 && modelY >= bb.y1 && modelY <= bb.y2;
    });

    if (hitNodes.length === 0) return null;
    // Prefer innermost leaf nodes (classes/interfaces) over parent compound packages
    const leafNodes = hitNodes.filter(n => !n.isParent());
    return leafNodes.length > 0 ? leafNodes[leafNodes.length - 1] : hitNodes[hitNodes.length - 1];
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
      $schema: "https://openheart.dev/schema/v1/diagram-state.json",
      generator: "OpenHeart Studio 2026",
      version: "1.0.0",
      format: "OpenHeart-Diagram-IR",
      diagram_type: this.currentGraphType,
      codebase_name: codebaseName,
      timestamp: new Date().toISOString(),
      pan: { ...this.cy.pan() },
      zoom: this.cy.zoom(),
      pan_locked: this.isPanLocked,
      active_filters: this.hiddenEdgeKinds ? Array.from(this.hiddenEdgeKinds) : [],
      elements: elements
    };
  }

  async loadDiagramState(savedState) {
    if (!savedState || !savedState.elements || !Array.isArray(savedState.elements)) {
      throw new Error("Invalid OpenHeart diagram state format.");
    }
    const targetGraphType = savedState.diagram_type || this.currentGraphType || 'class';
    this.currentGraphType = targetGraphType;
    
    // Check if elements have custom positions
    const hasCustomPositions = savedState.elements.some(e => e.position && (e.position.x !== undefined || e.position.y !== undefined));

    await this.renderGraph(targetGraphType, savedState.elements, hasCustomPositions);

    if (savedState.zoom && savedState.pan && this.cy) {
      this.cy.viewport({
        zoom: savedState.zoom,
        pan: savedState.pan
      });
    }

    if (savedState.pan_locked !== undefined) {
      this.setPanLock(savedState.pan_locked);
    }

    if (Array.isArray(savedState.active_filters)) {
      this.hiddenEdgeKinds = new Set(savedState.active_filters);
      if (this.onLayersUpdateCallback) {
        this.onLayersUpdateCallback(this.getActiveEdgeKinds());
      }
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

  /**
   * Normalizes a field definition (from object or raw string) into a structured AST property object.
   */
  parseFieldDefinition(f) {
    if (!f) return null;

    if (typeof f === 'object') {
      const name = f.name || 'field';
      const type = f.type || f.type_name || 'unknown';
      let visibility = f.visibility || 'private';
      if (visibility === '+') visibility = 'public';
      else if (visibility === '-') visibility = 'private';
      else if (visibility === '#') visibility = 'protected';
      else if (visibility === '~') visibility = 'package';

      const isStatic = Boolean(f.is_static || f.isStatic);
      const isFinal = Boolean(f.is_final || f.isFinal || f.is_readonly || f.readonly);
      const isCollection = Boolean(f.is_collection || f.isCollection || /List|Set|Map|Collection|Iterable|Array|Vec|\[\]/i.test(type));
      const defaultValue = f.default_value || f.defaultValue || null;
      const signature = f.signature || `${visibility} ${isStatic ? 'static ' : ''}${isFinal ? 'final ' : ''}${name}: ${type}${defaultValue ? ` = ${defaultValue}` : ''}`.trim();

      return {
        name,
        type,
        visibility,
        is_static: isStatic,
        is_final: isFinal,
        is_collection: isCollection,
        default_value: defaultValue,
        signature
      };
    }

    if (typeof f === 'string') {
      const raw = f.trim();
      if (!raw) return null;

      const isStatic = /\{static\}|\bstatic\b/i.test(raw);
      const isFinal = /\{final\}|\{readonly\}|\bfinal\b|\bconst\b|\breadonly\b/i.test(raw);

      // Strip annotations like {static}, {abstract} first to correctly inspect visibility prefix
      const stripped = raw.replace(/\{[^}]+\}\s*/g, '').trim();

      let visibility = 'private';
      if (stripped.startsWith('+') || /\bpublic\b/i.test(stripped)) visibility = 'public';
      else if (stripped.startsWith('#') || /\bprotected\b/i.test(stripped)) visibility = 'protected';
      else if (stripped.startsWith('~') || /\bpackage\b/i.test(stripped)) visibility = 'package';
      else if (stripped.startsWith('-') || /\bprivate\b/i.test(stripped)) visibility = 'private';

      let clean = stripped
        .replace(/^[+\-#~]\s*/, '')
        .replace(/\b(public|protected|private|static|final|readonly|const|val|var|let|mut)\b/gi, '')
        .replace(/;$/, '')
        .trim();

      let defaultValue = null;
      if (clean.includes('=')) {
        const parts = clean.split('=');
        clean = parts[0].trim();
        defaultValue = parts.slice(1).join('=').trim();
      }

      let name = 'field';
      let type = 'unknown';

      if (clean.includes(':')) {
        const parts = clean.split(':');
        name = parts[0].trim().replace(/[^a-zA-Z0-9_$]/g, '');
        type = parts[1].trim();
      } else if (clean.includes(' ')) {
        const tokens = clean.split(/\s+/).filter(Boolean);
        if (tokens.length >= 2) {
          name = tokens[tokens.length - 1].replace(/[^a-zA-Z0-9_$]/g, '');
          type = tokens.slice(0, -1).join(' ');
        } else if (tokens.length === 1) {
          name = tokens[0].replace(/[^a-zA-Z0-9_$]/g, '');
          type = 'any';
        }
      } else {
        name = clean.replace(/[^a-zA-Z0-9_$]/g, '') || 'field';
        type = 'any';
      }

      const isCollection = /List|Set|Map|Collection|Iterable|Array|Vec|\[\]|<[^>]+>/i.test(type);
      const signature = `${visibility} ${isStatic ? 'static ' : ''}${isFinal ? 'final ' : ''}${name}: ${type}${defaultValue ? ` = ${defaultValue}` : ''}`.trim();

      return {
        name,
        type,
        visibility,
        is_static: isStatic,
        is_final: isFinal,
        is_collection: isCollection,
        default_value: defaultValue,
        signature
      };
    }

    return null;
  }

  /**
   * Normalizes a method definition (from object or raw string) into a structured AST method object.
   */
  parseMethodDefinition(m) {
    if (!m) return null;

    if (typeof m === 'object') {
      const name = m.name || 'method';
      const returnType = m.return_type || m.returnType || m.type_name || 'void';
      let visibility = m.visibility || 'public';
      if (visibility === '+') visibility = 'public';
      else if (visibility === '-') visibility = 'private';
      else if (visibility === '#') visibility = 'protected';
      else if (visibility === '~') visibility = 'package';

      const isStatic = Boolean(m.is_static || m.isStatic);
      const isAbstract = Boolean(m.is_abstract || m.isAbstract);
      const isAsync = Boolean(m.is_async || m.isAsync);

      let parameters = [];
      if (Array.isArray(m.parameters)) {
        parameters = m.parameters.map(p => {
          if (typeof p === 'object' && p !== null) {
            return { name: p.name || 'arg', type: p.type || p.type_name || 'any' };
          }
          return { name: String(p), type: 'any' };
        });
      }

      const paramSig = parameters.map(p => `${p.name}: ${p.type}`).join(', ');
      const signature = m.signature || `${visibility} ${isStatic ? 'static ' : ''}${isAbstract ? 'abstract ' : ''}${isAsync ? 'async ' : ''}${name}(${paramSig}): ${returnType}`.trim();

      return {
        name,
        return_type: returnType,
        visibility,
        is_static: isStatic,
        is_abstract: isAbstract,
        is_async: isAsync,
        parameters,
        signature
      };
    }

    if (typeof m === 'string') {
      const raw = m.trim();
      if (!raw) return null;

      const isStatic = /\{static\}|\bstatic\b/i.test(raw);
      const isAbstract = /\{abstract\}|\babstract\b/i.test(raw);
      const isAsync = /\basync\b/i.test(raw);

      // Strip annotations like {static}, {abstract} first to correctly inspect visibility prefix
      const stripped = raw.replace(/\{[^}]+\}\s*/g, '').trim();

      let visibility = 'public';
      if (stripped.startsWith('-') || /\bprivate\b/i.test(stripped)) visibility = 'private';
      else if (stripped.startsWith('#') || /\bprotected\b/i.test(stripped)) visibility = 'protected';
      else if (stripped.startsWith('~') || /\bpackage\b/i.test(stripped)) visibility = 'package';
      else if (stripped.startsWith('+') || /\bpublic\b/i.test(stripped)) visibility = 'public';

      let clean = stripped
        .replace(/^[+\-#~]\s*/, '')
        .replace(/\b(public|protected|private|static|abstract|async|fun|fn|def|function)\b/gi, '')
        .replace(/;$/, '')
        .trim();

      const paramMatch = clean.match(/\(([^)]*)\)/);
      const paramsRaw = paramMatch ? paramMatch[1].trim() : '';
      const parameters = [];

      if (paramsRaw) {
        const parts = paramsRaw.split(',');
        for (const p of parts) {
          const trimmed = p.trim();
          if (!trimmed) continue;
          if (trimmed.includes(':')) {
            const [pName, pType] = trimmed.split(':').map(s => s.trim());
            parameters.push({ name: pName.replace(/[^a-zA-Z0-9_$]/g, '') || 'arg', type: pType || 'any' });
          } else {
            const tokens = trimmed.replace(/\bfinal\b|\bval\b|\bvar\b/g, '').trim().split(/\s+/);
            if (tokens.length >= 2) {
              const pType = tokens.slice(0, -1).join(' ');
              const pName = tokens[tokens.length - 1].replace(/[^a-zA-Z0-9_$]/g, '');
              parameters.push({ name: pName || 'arg', type: pType });
            } else if (tokens.length === 1) {
              parameters.push({ name: tokens[0].replace(/[^a-zA-Z0-9_$]/g, '') || 'arg', type: 'any' });
            }
          }
        }
      }

      let returnType = 'void';
      let name = 'method';

      const afterParenMatch = clean.match(/\)\s*:\s*([A-Za-z0-9_<>[\]?]+)/);
      if (afterParenMatch) {
        returnType = afterParenMatch[1].trim();
      }

      const beforeParen = clean.split('(')[0].trim();
      const nameTokens = beforeParen.split(/\s+/).filter(Boolean);
      if (nameTokens.length >= 2) {
        name = nameTokens[nameTokens.length - 1].replace(/[^a-zA-Z0-9_$]/g, '');
        if (returnType === 'void') {
          returnType = nameTokens.slice(0, -1).join(' ');
        }
      } else if (nameTokens.length === 1) {
        name = nameTokens[0].replace(/[^a-zA-Z0-9_$]/g, '');
      }

      const paramSig = parameters.map(p => `${p.name}: ${p.type}`).join(', ');
      const signature = `${visibility} ${isStatic ? 'static ' : ''}${isAbstract ? 'abstract ' : ''}${isAsync ? 'async ' : ''}${name}(${paramSig}): ${returnType}`.trim();

      return {
        name,
        return_type: returnType,
        visibility,
        is_static: isStatic,
        is_abstract: isAbstract,
        is_async: isAsync,
        parameters,
        signature
      };
    }

    return null;
  }

  /**
   * Generates natural language semantic explanation for an architectural relationship.
   */
  describeRelationSemantics(srcName, tgtName, kind, arrow, label) {
    const k = (kind || '').toLowerCase();
    const lblStr = label ? ` ('${label}')` : '';

    if (k === 'generalization' || arrow === '--|>' || arrow === '<|--') {
      return `${srcName} inherits from and specializes ${tgtName} (Generalization / Extends)`;
    }
    if (k === 'realization' || arrow === '..|>' || arrow === '<|..') {
      return `${srcName} realizes and implements contract of ${tgtName} (Realization / Implements)`;
    }
    if (k === 'composition' || arrow.includes('*--') || arrow.includes('--*')) {
      return `${srcName} strongly composes ${tgtName} with strict lifecycle ownership (Composition)`;
    }
    if (k === 'aggregation' || arrow.includes('o--') || arrow.includes('--o')) {
      return `${srcName} aggregates and references ${tgtName} (Aggregation)`;
    }
    if (k === 'dependency' || arrow.includes('..>') || arrow.includes('<..')) {
      return `${srcName} depends on and consumes ${tgtName}${lblStr} (Dependency)`;
    }
    if (k === 'association' || arrow.includes('-->') || arrow.includes('<--')) {
      return `${srcName} is associated with and calls ${tgtName}${lblStr} (Association)`;
    }
    if (k === 'message') {
      return `${srcName} sends message '${label || 'invokes'}' to ${tgtName} (Sequence Message)`;
    }
    if (k === 'transition') {
      return `${srcName} transitions to ${tgtName}${label ? ` on event '${label}'` : ''} (State Transition)`;
    }
    if (k === 'control_flow') {
      return `${srcName} directs control flow to ${tgtName}${label ? ` condition '${label}'` : ''} (Control Flow)`;
    }
    if (k === 'data_flow') {
      return `${srcName} transmits data to ${tgtName}${lblStr} (Data Flow)`;
    }
    return `${srcName} connects to ${tgtName}${lblStr} (${kind})`;
  }

  /**
   * Exports pure semantic AST code structure without any coordinates, dimensions, SVG data URIs, or styling.
   * Tailored specifically for AI / LLM reasoning, code generation, and architectural analysis.
   */
  getCodeStructureForAi() {
    if (!this.cy) return null;

    const codebaseName = sessionStorage.getItem('openheart_custom_name') || 'OpenHeart';
    const nodes = this.cy.nodes();
    const edges = this.cy.edges();

    const nodeById = new Map();
    const packageNodes = [];
    const typeNodes = [];

    nodes.forEach(node => {
      const d = node.data() || {};
      nodeById.set(node.id(), d);

      const isPkg = Boolean(d.isPackage || d.isLeafPackage || d.kind === 'package' || node.isParent());
      if (isPkg) {
        packageNodes.push({ id: node.id(), data: d });
      } else {
        typeNodes.push({ id: node.id(), data: d });
      }
    });

    const packageMembersMap = new Map();
    packageNodes.forEach(p => {
      const rawName = p.data.rawName || p.data.name || p.id;
      packageMembersMap.set(p.id, {
        id: p.id,
        name: rawName,
        is_domain_tier: Boolean(p.data.isDomainTier),
        members: []
      });
    });

    const relationships = [];
    const outgoingByNode = new Map();
    const incomingByNode = new Map();

    edges.forEach((edge, idx) => {
      const ed = edge.data() || {};
      const srcId = edge.source().id();
      const tgtId = edge.target().id();
      const srcData = nodeById.get(srcId) || {};
      const tgtData = nodeById.get(tgtId) || {};

      const srcName = srcData.name || srcData.label || srcId;
      const tgtName = tgtData.name || tgtData.label || tgtId;
      const umlKind = ed.uml_kind || ed.kind || 'association';
      const arrow = ed.arrow || '-->';
      const label = (ed.label || '').trim();

      const semantics = this.describeRelationSemantics(srcName, tgtName, umlKind, arrow, label);

      const relObj = {
        id: ed.id || `rel_${idx}_${srcId}_${tgtId}`,
        source: srcName,
        source_id: srcId,
        target: tgtName,
        target_id: tgtId,
        relation_type: umlKind,
        arrow: arrow,
        label: label || undefined,
        semantics: semantics
      };

      relationships.push(relObj);

      if (!outgoingByNode.has(srcId)) outgoingByNode.set(srcId, []);
      outgoingByNode.get(srcId).push({
        target: tgtName,
        relation_type: umlKind,
        semantics: semantics,
        label: label || undefined
      });

      if (!incomingByNode.has(tgtId)) incomingByNode.set(tgtId, []);
      incomingByNode.get(tgtId).push({
        source: srcName,
        relation_type: umlKind,
        semantics: semantics,
        label: label || undefined
      });
    });

    const types = [];
    let totalProperties = 0;
    let totalMethods = 0;
    let classCount = 0;
    let interfaceCount = 0;
    let enumCount = 0;
    let abstractCount = 0;

    typeNodes.forEach(t => {
      const d = t.data;
      const typeId = t.id;
      const typeName = d.name || d.label || typeId;
      const kind = (d.kind || 'class').toLowerCase();
      const stereotype = d.stereotype || `<<${kind}>>`;

      if (kind === 'interface') interfaceCount++;
      else if (kind === 'enum') enumCount++;
      else if (kind === 'abstract' || stereotype.includes('abstract')) abstractCount++;
      else classCount++;

      let packageName = d.package || null;
      if (!packageName && d.parent) {
        const parentPkg = packageMembersMap.get(d.parent);
        if (parentPkg) {
          packageName = parentPkg.name;
          if (!parentPkg.members.includes(typeName)) parentPkg.members.push(typeName);
        }
      } else if (packageName) {
        let existing = null;
        for (const [_, pkg] of packageMembersMap) {
          if (pkg.name === packageName) {
            existing = pkg;
            break;
          }
        }
        if (!existing) {
          existing = { id: packageName, name: packageName, members: [] };
          packageMembersMap.set(packageName, existing);
        }
        if (!existing.members.includes(typeName)) existing.members.push(typeName);
      }

      const extendsList = [];
      const implementsList = [];

      if (d.extends) {
        if (Array.isArray(d.extends)) extendsList.push(...d.extends);
        else extendsList.push(d.extends);
      }
      if (d.implements) {
        if (Array.isArray(d.implements)) implementsList.push(...d.implements);
        else implementsList.push(d.implements);
      }

      const outgoing = outgoingByNode.get(typeId) || [];
      outgoing.forEach(rel => {
        if (rel.relation_type === 'generalization' && !extendsList.includes(rel.target)) {
          extendsList.push(rel.target);
        } else if (rel.relation_type === 'realization' && !implementsList.includes(rel.target)) {
          implementsList.push(rel.target);
        }
      });

      const rawFields = Array.isArray(d.fields) ? d.fields : [];
      const properties = rawFields
        .map(f => this.parseFieldDefinition(f))
        .filter(Boolean);
      totalProperties += properties.length;

      const rawMethods = Array.isArray(d.methods) ? d.methods : [];
      const methods = rawMethods
        .map(m => this.parseMethodDefinition(m))
        .filter(Boolean);
      totalMethods += methods.length;

      types.push({
        id: typeId,
        name: typeName,
        kind: kind,
        stereotype: stereotype,
        package: packageName || undefined,
        file_path: d.file || `${typeName}.java`,
        extends: extendsList.length > 0 ? (extendsList.length === 1 ? extendsList[0] : extendsList) : undefined,
        implements: implementsList.length > 0 ? implementsList : undefined,
        properties: properties,
        methods: methods,
        outgoing_relations: outgoing,
        incoming_relations: incomingByNode.get(typeId) || []
      });
    });

    const packages = Array.from(packageMembersMap.values()).map(p => ({
      name: p.name,
      is_domain_tier: p.is_domain_tier || undefined,
      member_count: p.members.length,
      members: p.members
    }));

    return {
      $schema: "https://openheart.dev/schema/v1/code-structure.ai.json",
      generator: "OpenHeart Studio 2026 - AI Code Structure Exporter",
      project_name: codebaseName,
      diagram_type: this.currentGraphType,
      export_timestamp: new Date().toISOString(),
      description: "Semantic codebase architecture AST, complete type contracts, and relationship matrix for AI/LLM contextual understanding, reasoning, and code generation. Layout coordinates and UI formatting are excluded.",
      summary: {
        total_packages: packages.length,
        total_types: types.length,
        total_classes: classCount,
        total_interfaces: interfaceCount,
        total_abstract_classes: abstractCount,
        total_enums: enumCount,
        total_relationships: relationships.length,
        total_properties: totalProperties,
        total_methods: totalMethods
      },
      packages: packages,
      types: types,
      relationships: relationships
    };
  }

  /**
   * Generates a prompt-ready Markdown document formatted for pasting directly into LLMs (ChatGPT, Claude, Gemini, etc.).
   */
  getCodeStructureMarkdownForAi() {
    const data = this.getCodeStructureForAi();
    if (!data) return '';

    const lines = [];
    lines.push(`# Codebase Architecture & Semantic Model: ${data.project_name}`);
    lines.push(`> Exported from OpenHeart Studio for AI & LLM Contextual Reasoning (Type Contracts, AST, & Relationship Matrix)`);
    lines.push('');
    lines.push(`## Architectural Overview`);
    lines.push(`- **Project Name:** ${data.project_name}`);
    lines.push(`- **Diagram Projection:** ${data.diagram_type.toUpperCase()}`);
    lines.push(`- **Total Packages:** ${data.summary.total_packages}`);
    lines.push(`- **Total Types:** ${data.summary.total_types} (${data.summary.total_classes} Classes, ${data.summary.total_interfaces} Interfaces, ${data.summary.total_abstract_classes} Abstract Classes, ${data.summary.total_enums} Enums)`);
    lines.push(`- **Total Properties / Fields:** ${data.summary.total_properties}`);
    lines.push(`- **Total Functions / Methods:** ${data.summary.total_methods}`);
    lines.push(`- **Total Architectural Couplings:** ${data.summary.total_relationships}`);
    lines.push('');

    if (data.packages.length > 0) {
      lines.push(`## Packages & Namespaces`);
      data.packages.forEach(pkg => {
        const membersStr = pkg.members.length > 0 ? pkg.members.join(', ') : 'None';
        lines.push(`- **\`${pkg.name}\`** (${pkg.member_count} types): ${membersStr}`);
      });
      lines.push('');
    }

    lines.push(`## Type Definitions & Declarations`);
    lines.push('');

    data.types.forEach(t => {
      const kindHeader = `${t.kind || 'class'} ${t.name}`;
      lines.push(`### \`${kindHeader}\``);
      if (t.package) lines.push(`- **Package:** \`${t.package}\``);
      if (t.file_path) lines.push(`- **Source File:** \`${t.file_path}\``);
      if (t.extends) {
        const extStr = Array.isArray(t.extends) ? t.extends.join(', ') : t.extends;
        lines.push(`- **Extends / Base:** \`${extStr}\``);
      }
      if (t.implements && t.implements.length > 0) {
        lines.push(`- **Implements:** \`${t.implements.join(', ')}\``);
      }

      if (t.properties.length > 0) {
        lines.push('');
        lines.push(`#### Properties & Fields (${t.properties.length})`);
        t.properties.forEach(p => {
          const modStr = [
            p.visibility,
            p.is_static ? 'static' : '',
            p.is_final ? 'final' : '',
            p.is_collection ? 'collection' : ''
          ].filter(Boolean).join(' ');
          const valStr = p.default_value ? ` = ${p.default_value}` : '';
          lines.push(`- \`${modStr ? modStr + ' ' : ''}${p.name}: ${p.type}${valStr}\``);
        });
      }

      if (t.methods.length > 0) {
        lines.push('');
        lines.push(`#### Methods & Functions (${t.methods.length})`);
        t.methods.forEach(m => {
          const modStr = [
            m.visibility,
            m.is_static ? 'static' : '',
            m.is_abstract ? 'abstract' : '',
            m.is_async ? 'async' : ''
          ].filter(Boolean).join(' ');
          const paramStr = m.parameters.map(p => `${p.name}: ${p.type}`).join(', ');
          lines.push(`- \`${modStr ? modStr + ' ' : ''}${m.name}(${paramStr}): ${m.return_type}\``);
        });
      }

      if (t.outgoing_relations.length > 0 || t.incoming_relations.length > 0) {
        lines.push('');
        lines.push(`#### Coupling & Relations`);
        t.outgoing_relations.forEach(r => {
          lines.push(`- Outgoing: ${r.semantics}`);
        });
        t.incoming_relations.forEach(r => {
          lines.push(`- Incoming: ${r.semantics}`);
        });
      }

      lines.push('');
      lines.push('---');
      lines.push('');
    });

    if (data.relationships.length > 0) {
      lines.push(`## Relationship Coupling Matrix`);
      data.relationships.forEach(r => {
        lines.push(`- **\`${r.source}\`** ${r.arrow} **\`${r.target}\`** : ${r.semantics}`);
      });
      lines.push('');
    }

    return lines.join('\n');
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

    // ── Node Reposition Tracking for Figma-Style Undo / Redo ──
    const dragStartPositions = new Map();

    this.cy.on('grab', 'node', () => {
      dragStartPositions.clear();
      this.cy.nodes().forEach(n => {
        dragStartPositions.set(n.id(), { ...n.position() });
      });
    });

    this.cy.on('free', 'node', () => {
      if (dragStartPositions.size === 0) return;
      const movedNodes = [];
      dragStartPositions.forEach((oldPos, id) => {
        const node = this.cy.getElementById(id);
        if (node && node.length > 0) {
          const currentPos = node.position();
          if (Math.hypot(currentPos.x - oldPos.x, currentPos.y - oldPos.y) > 1.0) {
            movedNodes.push({
              id,
              oldPos: { x: oldPos.x, y: oldPos.y },
              newPos: { x: currentPos.x, y: currentPos.y }
            });
          }
        }
      });
      dragStartPositions.clear();

      if (movedNodes.length > 0) {
        const targetLabel = movedNodes.length === 1
          ? (this.cy.getElementById(movedNodes[0].id).data('label') || movedNodes[0].id)
          : `${movedNodes.length} nodes`;

        this.pushUndoAction({
          type: 'move',
          description: `Move ${targetLabel}`,
          undo: () => {
            const canAnimate = movedNodes.length <= 15;
            movedNodes.forEach(({ id, oldPos }) => {
              const n = this.cy.getElementById(id);
              if (n && n.length > 0) {
                if (canAnimate) {
                  n.animate({ position: oldPos }, { duration: 180 });
                } else {
                  n.position(oldPos);
                }
              }
            });
            if (this.minimap) this.minimap.onViewportChange();
          },
          redo: () => {
            const canAnimate = movedNodes.length <= 15;
            movedNodes.forEach(({ id, newPos }) => {
              const n = this.cy.getElementById(id);
              if (n && n.length > 0) {
                if (canAnimate) {
                  n.animate({ position: newPos }, { duration: 180 });
                } else {
                  n.position(newPos);
                }
              }
            });
            if (this.minimap) this.minimap.onViewportChange();
          }
        });
      }
    });
  }

  togglePackageCollapse(pkgNode, recordUndo = true) {
    if (!pkgNode || (typeof pkgNode.id !== 'function' && !pkgNode.length)) return;
    const actualNode = pkgNode.length !== undefined ? pkgNode[0] : pkgNode;
    const pkgId = actualNode.id();
    const children = this.cy.nodes(`[parent = "${pkgId}"]`);
    const isCollapsed = this.collapsedPackages.has(pkgId);
    const rawName = actualNode.data('rawName') || pkgId.replace(/^pkg_/, '').replace(/_/g, '.');
    const shortName = rawName.split('.').pop();
    const isDomainTier = actualNode.data('isDomainTier');

    this.cy.batch(() => {
      if (isCollapsed) {
        // Expand (Open)
        this.collapsedPackages.delete(pkgId);
        actualNode.removeClass('package-collapsed');
        actualNode.data('width', actualNode.data('origWidth') || 650);
        actualNode.data('height', actualNode.data('origHeight') || 400);
        actualNode.data('label', isDomainTier ? `[−] DOMAIN LAYER: ${rawName.toUpperCase()}` : `[−] package [${shortName}]`);
        children.style('display', 'element');
        children.connectedEdges().style('display', 'element');
      } else {
        // Collapse (Close)
        this.collapsedPackages.add(pkgId);
        actualNode.addClass('package-collapsed');
        actualNode.data('label', isDomainTier ? `[+] DOMAIN LAYER: ${rawName.toUpperCase()} (${children.length} subpackages)` : `[+] package [${shortName}] (${children.length} classes)`);
        children.style('display', 'none');
        children.connectedEdges().style('display', 'none');
      }
    });

    if (recordUndo) {
      this.pushUndoAction({
        type: 'package-collapse',
        description: isCollapsed ? `Expand ${shortName}` : `Collapse ${shortName}`,
        undo: () => {
          const p = this.cy.getElementById(pkgId);
          if (p && p.length > 0) this.togglePackageCollapse(p, false);
        },
        redo: () => {
          const p = this.cy.getElementById(pkgId);
          if (p && p.length > 0) this.togglePackageCollapse(p, false);
        }
      });
    }
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

  // ── 6. Figma-Grade Undo / Redo & Manipulation History Engine ──
  bindHistoryControls() {
    const btnUndo = document.getElementById('btn-canvas-undo');
    const btnRedo = document.getElementById('btn-canvas-redo');

    if (btnUndo) {
      btnUndo.addEventListener('click', (e) => {
        e.preventDefault();
        this.undo();
      });
    }

    if (btnRedo) {
      btnRedo.addEventListener('click', (e) => {
        e.preventDefault();
        this.redo();
      });
    }

    this.updateUndoRedoButtons();
  }

  pushUndoAction(action) {
    this.undoStack.push(action);
    if (this.undoStack.length > this.maxHistory) {
      this.undoStack.shift();
    }
    this.redoStack = [];
    this.updateUndoRedoButtons();
    if (action.description && action.type !== 'move') {
      this.showCanvasNotification(action.description);
    }
  }

  undo() {
    if (this.undoStack.length === 0) {
      this.showCanvasNotification('Nothing to undo');
      return;
    }
    const action = this.undoStack.pop();
    try {
      action.undo();
      this.redoStack.push(action);
      this.showCanvasNotification(`Undo: ${action.description || 'Action'}`);
    } catch (err) {
      console.error('Error during undo:', err);
    }
    this.updateUndoRedoButtons();
  }

  redo() {
    if (this.redoStack.length === 0) {
      this.showCanvasNotification('Nothing to redo');
      return;
    }
    const action = this.redoStack.pop();
    try {
      action.redo();
      this.undoStack.push(action);
      this.showCanvasNotification(`Redo: ${action.description || 'Action'}`);
    } catch (err) {
      console.error('Error during redo:', err);
    }
    this.updateUndoRedoButtons();
  }

  updateUndoRedoButtons() {
    const btnUndo = document.getElementById('btn-canvas-undo');
    const btnRedo = document.getElementById('btn-canvas-redo');

    if (btnUndo) {
      if (this.undoStack.length > 0) {
        btnUndo.classList.remove('disabled');
        btnUndo.removeAttribute('disabled');
        const lastAction = this.undoStack[this.undoStack.length - 1];
        btnUndo.setAttribute('title', `Undo ${lastAction.description || ''} (Ctrl+Z)`);
      } else {
        btnUndo.classList.add('disabled');
        btnUndo.setAttribute('disabled', 'true');
        btnUndo.setAttribute('title', 'Undo (Ctrl+Z)');
      }
    }

    if (btnRedo) {
      if (this.redoStack.length > 0) {
        btnRedo.classList.remove('disabled');
        btnRedo.removeAttribute('disabled');
        const nextAction = this.redoStack[this.redoStack.length - 1];
        btnRedo.setAttribute('title', `Redo ${nextAction.description || ''} (Ctrl+Y or Ctrl+Shift+Z)`);
      } else {
        btnRedo.classList.add('disabled');
        btnRedo.setAttribute('disabled', 'true');
        btnRedo.setAttribute('title', 'Redo (Ctrl+Y or Ctrl+Shift+Z)');
      }
    }
  }

  showCanvasNotification(msg) {
    const statusText = document.getElementById('hud-status-text');
    const hudStatus = document.getElementById('hud-status');
    if (statusText) {
      statusText.textContent = msg;
      if (hudStatus) {
        hudStatus.classList.remove('status-pulse');
        void hudStatus.offsetWidth;
        hudStatus.classList.add('status-pulse');
      }
      clearTimeout(this._hudStatusTimer);
      this._hudStatusTimer = setTimeout(() => {
        if (statusText) {
          statusText.textContent = 'Two-finger scroll to Pan · Two-finger pinch to Zoom · Hover to trace path';
        }
      }, 2500);
    }
  }

  deleteSelectedNode() {
    if (!this.cy) return;
    const selectedNodes = this.cy.nodes(':selected');
    const targetNodes = selectedNodes.length > 0
      ? selectedNodes
      : (this.selectedNode ? this.cy.getElementById(this.selectedNode.id) : null);

    if (!targetNodes || targetNodes.length === 0) return;

    const validTargets = targetNodes.filter(n => n.id() !== 'interactive-canvas');
    if (validTargets.length === 0) return;

    const toDelete = validTargets.union(validTargets.connectedEdges());
    const elesJson = toDelete.jsons();
    const label = validTargets.length === 1
      ? (validTargets[0].data('label') || validTargets[0].id())
      : `${validTargets.length} nodes`;

    toDelete.remove();
    this.selectedNode = null;
    if (this.minimap) this.minimap.onViewportChange();

    this.pushUndoAction({
      type: 'delete',
      description: `Delete ${label}`,
      undo: () => {
        const added = this.cy.add(elesJson);
        if (this.minimap) this.minimap.onViewportChange();
        added.nodes().select();
      },
      redo: () => {
        elesJson.forEach(item => {
          const el = this.cy.getElementById(item.data.id);
          if (el && el.length > 0) el.remove();
        });
        this.selectedNode = null;
        if (this.minimap) this.minimap.onViewportChange();
      }
    });

    this.showCanvasNotification(`Deleted ${label}`);
  }
}
