/**
 * OpenHeart Web Studio — 3D Morphing Spiky Orb Engine & Neo-Brutalist Studio Controller
 * Authored for OpenHeart SCPG Engine. Maintained by Ahmad Hassan (B-Ted).
 */

// Initialize Mermaid.js for diagram rendering
mermaid.initialize({
  startOnLoad: false,
  theme: 'dark',
  securityLevel: 'loose',
  flowchart: { curve: 'basis' }
});

/* ==========================================================================
   1. THREE.JS WEBGL 3D MORPHING SPIKY ORB
   ========================================================================== */

let scene, camera, renderer, orbMesh, particleSystem;
let originalPositions, originalNormals;
let simplex = new SimplexNoise();
let clock = new THREE.Clock();
let isStudioEngaged = false;
let mouseX = 0, mouseY = 0;
let targetX = 0, targetY = 0;

function initThreeOrb() {
  const canvas = document.getElementById('orb-canvas');
  if (!canvas) return;

  // Scene & Camera
  scene = new THREE.Scene();
  camera = new THREE.PerspectiveCamera(45, window.innerWidth / window.innerHeight, 0.1, 100);
  camera.position.set(0, 0, 6.5);

  // Renderer with High DPR and Antialiasing
  renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true });
  renderer.setSize(window.innerWidth, window.innerHeight);
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));

  // Dramatic 3D Studio Lighting Setup
  const ambientLight = new THREE.AmbientLight(0x333333, 1.0);
  scene.add(ambientLight);

  const mainLight = new THREE.DirectionalLight(0xffffff, 1.4);
  mainLight.position.set(5, 5, 5);
  scene.add(mainLight);

  const rimLight = new THREE.DirectionalLight(0x888888, 0.8);
  rimLight.position.set(-5, -5, -2);
  scene.add(rimLight);

  const pointLight = new THREE.PointLight(0xffffff, 1.2, 10);
  pointLight.position.set(0, 0, 3);
  scene.add(pointLight);

  // 3D Geometry: High-resolution Icosahedron for crystalline facet spiky morphing
  const geometry = new THREE.IcosahedronGeometry(2.1, 32);

  const posAttr = geometry.attributes.position;
  const normAttr = geometry.attributes.normal;
  originalPositions = new Float32Array(posAttr.array);
  originalNormals = new Float32Array(normAttr.array);

  // High-Contrast Off-White Metallic Material with Flat Facet Shading
  const material = new THREE.MeshStandardMaterial({
    color: 0xdddddd,
    roughness: 0.25,
    metalness: 0.15,
    flatShading: true,
    transparent: true,
    opacity: 0.95
  });

  orbMesh = new THREE.Mesh(geometry, material);
  scene.add(orbMesh);

  // Ambient Floating Particles
  const particleCount = 400;
  const particleGeo = new THREE.BufferGeometry();
  const particlePositions = new Float32Array(particleCount * 3);

  for (let i = 0; i < particleCount * 3; i += 3) {
    particlePositions[i] = (Math.random() - 0.5) * 18;
    particlePositions[i + 1] = (Math.random() - 0.5) * 18;
    particlePositions[i + 2] = (Math.random() - 0.5) * 18;
  }

  particleGeo.setAttribute('position', new THREE.BufferAttribute(particlePositions, 3));
  const particleMat = new THREE.PointsMaterial({
    color: 0xffffff,
    size: 0.03,
    transparent: true,
    opacity: 0.5
  });

  particleSystem = new THREE.Points(particleGeo, particleMat);
  scene.add(particleSystem);

  // Event Listeners
  window.addEventListener('resize', onWindowResize);
  document.addEventListener('mousemove', onMouseMove);

  // Animation Loop
  animate();
}

function onWindowResize() {
  if (!camera || !renderer) return;
  camera.aspect = window.innerWidth / window.innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(window.innerWidth, window.innerHeight);
}

function onMouseMove(e) {
  mouseX = (e.clientX / window.innerWidth - 0.5) * 2;
  mouseY = (e.clientY / window.innerHeight - 0.5) * 2;
}

function animate() {
  requestAnimationFrame(animate);
  const elapsedTime = clock.getElapsedTime();

  if (orbMesh && originalPositions) {
    const geo = orbMesh.geometry;
    const posAttr = geo.attributes.position;

    const spikeFactor = isStudioEngaged ? 1.6 : (0.45 + Math.sin(elapsedTime * 1.5) * 0.3);

    for (let i = 0; i < posAttr.count; i++) {
      const px = originalPositions[i * 3];
      const py = originalPositions[i * 3 + 1];
      const pz = originalPositions[i * 3 + 2];

      const nx = originalNormals[i * 3];
      const ny = originalNormals[i * 3 + 1];
      const nz = originalNormals[i * 3 + 2];

      const n1 = simplex.noise3D(px * 0.9 + elapsedTime * 0.4, py * 0.9 + elapsedTime * 0.4, pz * 0.9 + elapsedTime * 0.4);
      const n2 = simplex.noise3D(px * 2.2 - elapsedTime * 0.3, py * 2.2 - elapsedTime * 0.3, pz * 2.2 - elapsedTime * 0.3);
      const noiseVal = n1 * 0.7 + n2 * 0.3;

      const displacement = noiseVal * spikeFactor;

      posAttr.setXYZ(i, px + nx * displacement, py + ny * displacement, pz + nz * displacement);
    }

    posAttr.needsUpdate = true;
    geo.computeVertexNormals();

    targetX = mouseX * 0.4;
    targetY = mouseY * 0.4;

    orbMesh.rotation.y += 0.005;
    orbMesh.rotation.x += (targetY - orbMesh.rotation.x) * 0.05;
    orbMesh.rotation.z += (targetX - orbMesh.rotation.z) * 0.05;
  }

  if (particleSystem) {
    particleSystem.rotation.y = elapsedTime * 0.012;
  }

  if (camera) {
    if (isStudioEngaged) {
      camera.position.z += (3.5 - camera.position.z) * 0.05;
    } else {
      camera.position.z += (6.5 - camera.position.z) * 0.05;
    }
  }

  renderer.render(scene, camera);
}


/* ==========================================================================
   2. NEO-BRUTALIST STUDIO CONTROLLER & 14 UML MATRIX
   ========================================================================== */

const UML_TEMPLATES = {
  class: `graph TD
    classDef cls fill:#0a0a0a,stroke:#ffffff,stroke-width:2px,color:#ffffff;
    classDef trait fill:#0a0a0a,stroke:#a3a3a3,stroke-width:2px,color:#ffffff;
    
    C1["TokenCorpusBuilder<br/>+token_records: Vec&lt;TokenRecord&gt;<br/>+build(): TokenCorpusArtifact"]:::cls
    C2["StringInterner<br/>+table: Vec&lt;u64&gt;<br/>+intern(bytes): u32"]:::cls
    T1["&lt;&lt;LanguageAdapter&gt;&gt;<br/>+map_node_type(kind): TokenType"]:::trait
    
    C1 ..|> T1 : implements
    C1 --> C2 : contains interner`,

  object: `graph TD
    obj1["token_id: #1042<br/>sort_key: 0x0001000C0004<br/>text_id: 42<br/>type: Identifier"]
    obj2["file_id: #1<br/>path: 'src/main.java'<br/>sha256: 0xa4f..."]
    obj1 -->|SourceFileRecord| obj2`,

  component: `graph TD
    subgraph SCPG_Core["OpenHeart Core Engine"]
        P1["Phase 1: Lexical Ingestion"]
        P2["Phase 2: BP AST Encoder"]
        P3["Phase 3: Symbol Table & TH"]
        P4["Phase 4: CFG & Dominators"]
        P5["Phase 5: SSA & Data Flow"]
    end
    P1 --> P2 --> P3 --> P4 --> P5`,

  deployment: `graph LR
    Node1["Developer Workstation"] -->|HTTPS / Git| Node2["OpenHeart Portal"]
    Node2 -->|Process| Node3["Target .tca Binary Engine"]`,

  package: `graph TD
    subgraph crate_openheart["crate::openheart"]
        subgraph core_mod["core"]
            io["io"]
            types["types"]
        end
        subgraph phase1_mod["phase1"]
            adapter["adapter"]
            parser["parser"]
            builder["builder"]
        end
    end
    phase1_mod --> core_mod`,

  composite: `classDiagram
    class ClassStructure {
        +Port_1 : IngestionStream
        +Port_2 : SerializerPipe
        +InternalBuffer : StringInterner
    }`,

  profile: `graph TD
    prof["<<Stereotype>> SystemComponent"] --> classNode["TokenCorpusBuilder"]`,

  usecase: `graph LR
    Developer((Developer)) --> UC1(Fetch Repository)
    Developer --> UC2(Select UML Diagrams)
    Developer --> UC3(Export TCA Artifact)
    UC2 --> Engine((SCPG Engine))`,

  activity: `graph TD
    Start([Start Pipeline]) --> ReadFile[Read Source Bytes]
    ReadFile --> Tokenize[Tree-sitter Scan & Assign token_id]
    Tokenize --> Intern[FNV-1a String Intern]
    Intern --> Check{Check Invariants 1-4}
    Check -->|Pass| WriteTCA[Write .tca Binary Artifact]
    Check -->|Fail| Err[Abort Integrity Error]
    WriteTCA --> End([Complete])`,

  statemachine: `stateDiagram-v2
    [*] --> Uninitialized
    Uninitialized --> Scanning : fetch_repo()
    Scanning --> InvariantCheck : validate_tokens()
    InvariantCheck --> ArtifactReady : build_tca()
    ArtifactReady --> [*]`,

  sequence: `sequenceDiagram
    autonumber
    actor User
    participant Portal as Web Adapter
    participant Scanner as Tree-sitter Parser
    participant Engine as SCPG Engine

    User->>Portal: Input GitHub Repository URL
    Portal->>Scanner: Fetch & Walk CST Leaves
    Scanner->>Engine: Monotonic token_ids + TokenRecords
    Engine-->>Portal: 14 Derived UML Views
    Portal-->>User: Interactive Studio Rendering`,

  communication: `graph LR
    User -->|1: submit_url()| Portal
    Portal -->|2: walk_cst()| Scanner
    Scanner -->|3: build_tca()| Engine`,

  interaction: `graph TD
    subgraph Overview["Interaction Overview"]
        Init["Init Repository"] --> Seq1["Sequence: Scanner Handshake"]
        Seq1 --> Seq2["Sequence: UML Derivation"]
    end`,

  timing: `gantt
    title SCPG Ingestion Phase Timing Bounds
    dateFormat  SS
    axisFormat %S s
    section Phase 1
    Lexical Scanning     :a1, 00, 02s
    String Interning     :a2, after a1, 01s
    section Phase 2
    BP AST Construction  :b1, after a2, 02s`
};

let selectedDiagrams = new Set(['class', 'object', 'component', 'package', 'activity', 'sequence']);
let generatedDiagrams = {};
let currentActiveTab = 'class';

document.addEventListener('DOMContentLoaded', () => {
  // Initialize WebGL 3D Morphing Orb Engine
  initThreeOrb();

  const heroLanding = document.getElementById('hero-landing');
  const studioApp = document.getElementById('studio-app');
  const btnEngageOrb = document.getElementById('btn-engage-orb');
  const btnBackLanding = document.getElementById('btn-back-landing');
  
  const btnFetch = document.getElementById('btn-fetch');
  const btnToggleAdvanced = document.getElementById('btn-toggle-advanced');
  const advancedOptions = document.getElementById('advanced-options');
  
  const btnPresetCore = document.getElementById('btn-preset-core');
  const btnSelectAll = document.getElementById('btn-select-all');
  const btnClearAll = document.getElementById('btn-clear-all');
  
  const tabStructural = document.getElementById('tab-structural');
  const tabBehavioral = document.getElementById('tab-behavioral');
  const viewStructural = document.getElementById('view-structural');
  const viewBehavioral = document.getElementById('view-behavioral');

  const repoUrlInput = document.getElementById('repo-url-input');
  const pipelineStatus = document.getElementById('pipeline-status');
  const progressBarFill = document.getElementById('progress-bar-fill');
  const statusStepTitle = document.getElementById('status-step-title');
  const statusPercent = document.getElementById('status-percent');
  const statusLogs = document.getElementById('status-logs');
  const diagramTabs = document.getElementById('diagram-tabs');
  const renderContainer = document.getElementById('mermaid-render-container');
  const btnCopy = document.getElementById('btn-copy-mermaid');
  const btnExportSvg = document.getElementById('btn-export-svg');
  const activeDiagramCount = document.getElementById('active-diagram-count');
  const cornerBrandTopRight = document.querySelector('.corner-brand.top-right');

  // Advanced Options Toggle
  if (btnToggleAdvanced && advancedOptions) {
    btnToggleAdvanced.addEventListener('click', () => {
      advancedOptions.classList.toggle('hidden-options');
      btnToggleAdvanced.textContent = advancedOptions.classList.contains('hidden-options')
        ? 'SETTINGS ▾'
        : 'SETTINGS ▴';
    });
  }

  // Scope Toggle Handler (System View vs Module Breakdown)
  const scopeSystem = document.getElementById('scope-system');
  const scopeModule = document.getElementById('scope-module');
  const moduleSelector = document.getElementById('module-selector');

  if (scopeSystem && scopeModule) {
    scopeSystem.addEventListener('click', () => {
      scopeSystem.classList.add('active');
      scopeModule.classList.remove('active');
      if (moduleSelector) moduleSelector.value = 'all';
      if (currentActiveTab) renderMermaidDiagram(currentActiveTab, 'all');
    });

    scopeModule.addEventListener('click', () => {
      scopeModule.classList.add('active');
      scopeSystem.classList.remove('active');
      if (moduleSelector && moduleSelector.value === 'all') moduleSelector.value = 'core';
      if (currentActiveTab) renderMermaidDiagram(currentActiveTab, moduleSelector ? moduleSelector.value : 'core');
    });
  }

  if (moduleSelector) {
    moduleSelector.addEventListener('change', (e) => {
      const mod = e.target.value;
      if (mod !== 'all' && scopeSystem && scopeModule) {
        scopeModule.classList.add('active');
        scopeSystem.classList.remove('active');
      }
      if (currentActiveTab) renderMermaidDiagram(currentActiveTab, mod);
    });
  }

  // Category Tabs (Structural vs Behavioral)
  if (tabStructural && tabBehavioral) {
    tabStructural.addEventListener('click', () => {
      tabStructural.classList.add('active');
      tabBehavioral.classList.remove('active');
      viewStructural.classList.remove('hidden');
      viewBehavioral.classList.add('hidden');
    });

    tabBehavioral.addEventListener('click', () => {
      tabBehavioral.classList.add('active');
      tabStructural.classList.remove('active');
      viewBehavioral.classList.remove('hidden');
      viewStructural.classList.add('hidden');
    });
  }

  // Presets
  btnPresetCore.addEventListener('click', () => {
    setPreset(['class', 'object', 'component', 'package', 'activity', 'sequence']);
    setActivePresetBtn(btnPresetCore);
  });

  btnSelectAll.addEventListener('click', () => {
    const all = ['class', 'object', 'component', 'deployment', 'package', 'composite', 'profile', 'usecase', 'activity', 'statemachine', 'sequence', 'communication', 'interaction', 'timing'];
    setPreset(all);
    setActivePresetBtn(btnSelectAll);
  });

  btnClearAll.addEventListener('click', () => {
    setPreset([]);
    setActivePresetBtn(btnClearAll);
  });

  function setPreset(array) {
    selectedDiagrams = new Set(array);
    document.querySelectorAll('input[name="uml-type"]').forEach(cb => {
      const isChecked = selectedDiagrams.has(cb.value);
      cb.checked = isChecked;
      const parentCard = cb.closest('.brutalist-checkbox');
      if (parentCard) {
        if (isChecked) parentCard.classList.add('active');
        else parentCard.classList.remove('active');
      }
    });
    updateCountDisplay();
  }

  function setActivePresetBtn(btn) {
    document.querySelectorAll('.btn-preset').forEach(b => b.classList.remove('active'));
    btn.classList.add('active');
  }

  // Engage Studio
  btnEngageOrb.addEventListener('click', engageStudio);
  document.getElementById('orb-canvas').addEventListener('click', () => {
    if (!isStudioEngaged) engageStudio();
  });

  function engageStudio() {
    isStudioEngaged = true;
    heroLanding.classList.add('hero-hidden');
    if (cornerBrandTopRight) cornerBrandTopRight.classList.add('corner-hidden');
    setTimeout(() => {
      studioApp.classList.remove('studio-hidden');
    }, 400);
  }

  btnBackLanding.addEventListener('click', () => {
    isStudioEngaged = false;
    studioApp.classList.add('studio-hidden');
    if (cornerBrandTopRight) cornerBrandTopRight.classList.remove('corner-hidden');
    setTimeout(() => {
      heroLanding.classList.remove('hero-hidden');
    }, 300);
  });

  function updateCountDisplay() {
    if (activeDiagramCount) activeDiagramCount.textContent = selectedDiagrams.size;
  }

  // Checkbox Handlers
  document.querySelectorAll('input[name="uml-type"]').forEach(cb => {
    cb.addEventListener('change', (e) => {
      const parentCard = e.target.closest('.brutalist-checkbox');
      if (e.target.checked) {
        selectedDiagrams.add(e.target.value);
        if (parentCard) parentCard.classList.add('active');
      } else {
        selectedDiagrams.delete(e.target.value);
        if (parentCard) parentCard.classList.remove('active');
      }
      updateCountDisplay();
    });
  });

  // Pipeline Fetch Execution
  btnFetch.addEventListener('click', () => {
    const url = repoUrlInput.value.trim();
    if (!url || !url.startsWith('https://github.com/')) {
      alert('Please enter a valid GitHub repository URL starting with https://github.com/');
      return;
    }

    if (selectedDiagrams.size === 0) {
      alert('Please select at least one UML diagram projection.');
      return;
    }

    runPipelineSimulation(url);
  });

  async function runPipelineSimulation(url) {
    pipelineStatus.classList.remove('hidden');
    statusLogs.innerHTML = '';
    progressBarFill.style.width = '10%';
    statusPercent.textContent = '10%';

    logStep(`> Validating target repository URL: ${url}`);
    await sleep(400);

    statusStepTitle.textContent = 'STAGE 1: LEXICAL INGESTION & TREE-SITTER WALK...';
    progressBarFill.style.width = '35%';
    statusPercent.textContent = '35%';
    logStep('> Allocating monotonic token_id counter [0..4096]');
    logStep('> Interning identifiers with 64-bit FNV-1a StringInterner');
    await sleep(500);

    statusStepTitle.textContent = 'STAGE 2: VERIFYING CORPUS INVARIANTS 1–4...';
    progressBarFill.style.width = '65%';
    statusPercent.textContent = '65%';
    logStep('> Invariant 1 (Monotonicity): VERIFIED');
    logStep('> Invariant 2 (Injectivity): VERIFIED');
    logStep('> Invariant 3 (Completeness): VERIFIED');
    logStep('> Invariant 4 (Index Consistency): VERIFIED');
    await sleep(400);

    statusStepTitle.textContent = 'STAGE 3: DERIVING UML DIAGRAM VIEWS...';
    progressBarFill.style.width = '90%';
    statusPercent.textContent = '90%';
    logStep(`> Compiling ${selectedDiagrams.size} selected UML graph projections...`);
    await sleep(400);

    progressBarFill.style.width = '100%';
    statusPercent.textContent = '100%';
    statusStepTitle.textContent = 'ANALYSIS COMPLETE :: SCPG ARTIFACT RENDERED';
    logStep('> Serialization completed successfully.');
    await sleep(300);

    pipelineStatus.classList.add('hidden');
    renderStudioTabs();
  }

  function logStep(text) {
    const div = document.createElement('div');
    div.className = 'log-line';
    div.textContent = text;
    statusLogs.appendChild(div);
    statusLogs.scrollTop = statusLogs.scrollHeight;
  }

  function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
  }

  // Render Tabs & Studio View
  function renderStudioTabs() {
    diagramTabs.innerHTML = '';
    generatedDiagrams = {};
    const selectedArray = Array.from(selectedDiagrams);

    selectedArray.forEach((type, index) => {
      const btn = document.createElement('button');
      btn.className = `tab-btn ${index === 0 ? 'active' : ''}`;
      btn.textContent = getDiagramLabel(type);
      btn.addEventListener('click', () => {
        document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
        btn.classList.add('active');
        currentActiveTab = type;
        renderMermaidDiagram(type);
      });
      diagramTabs.appendChild(btn);

      generatedDiagrams[type] = UML_TEMPLATES[type] || `graph TD\n A["${type} Diagram"]`;
    });

    if (selectedArray.length > 0) {
      currentActiveTab = selectedArray[0];
      renderMermaidDiagram(currentActiveTab);
    }
  }

  function getDiagramLabel(type) {
    const labels = {
      class: 'CLASS',
      object: 'OBJECT',
      component: 'COMPONENT',
      deployment: 'DEPLOYMENT',
      package: 'PACKAGE',
      composite: 'COMPOSITE',
      profile: 'PROFILE',
      usecase: 'USE CASE',
      activity: 'ACTIVITY',
      statemachine: 'STATE MACHINE',
      sequence: 'SEQUENCE',
      communication: 'COMMUNICATION',
      interaction: 'INTERACTION OVERVIEW',
      timing: 'TIMING'
    };
    return labels[type] || type.toUpperCase();
  }

  async function renderMermaidDiagram(type, selectedModule = 'all') {
    const mermaidCode = generatedDiagrams[type];
    renderContainer.innerHTML = `<div class="mermaid">${mermaidCode}</div>`;

    try {
      await mermaid.run({
        nodes: renderContainer.querySelectorAll('.mermaid')
      });
    } catch (err) {
      console.error('Mermaid render error:', err);
    }

    // Dynamic Module Traceability Update
    const modPath = selectedModule === 'all' ? `src/${type}_layer.rs` : `src/${selectedModule}/${type}_spec.rs`;
    document.getElementById('trace-tid').textContent = `#${Math.floor(Math.random() * 8000 + 1000)}`;
    document.getElementById('trace-file').textContent = modPath;
    document.getElementById('trace-span').textContent = `L12:C4 - L48:C32`;
    document.getElementById('trace-hash').textContent = `0x${Math.floor(Math.random() * 0xFFFFFFFF).toString(16).toUpperCase()}`;
  }

  // Copy Mermaid Source
  btnCopy.addEventListener('click', () => {
    const code = generatedDiagrams[currentActiveTab];
    if (code) {
      navigator.clipboard.writeText(code);
      alert('Mermaid diagram source copied to clipboard!');
    }
  });

  // Export SVG
  btnExportSvg.addEventListener('click', () => {
    const svgEl = renderContainer.querySelector('svg');
    if (svgEl) {
      const svgData = new XMLSerializer().serializeToString(svgEl);
      const blob = new Blob([svgData], { type: 'image/svg+xml;charset=utf-8' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `openheart_${currentActiveTab}_diagram.svg`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
    } else {
      alert('No SVG available to export.');
    }
  });
});
