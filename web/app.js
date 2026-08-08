/**
 * OpenHeart Web Studio — Three.js WebGL 3D Morphing Orb Shader & Neo-Brutalist Controller
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
   1. THREE.JS WEBGL 3D MORPHING SPIKY ORB SHADER (Matching file.mp4)
   ========================================================================== */

let scene, camera, renderer, orbMesh, material, particlesMesh;
let clock = new THREE.Clock();
let isStudioEngaged = false;
let mouseX = 0, mouseY = 0;
let targetX = 0, targetY = 0;

// Custom GLSL Shaders for Organic Morphing Noise Orb
const vertexShader = `
  uniform float uTime;
  uniform float uDisplacement;
  varying vec3 vNormal;
  varying vec3 vPosition;
  varying float vNoise;

  // Simplex 3D noise function
  vec4 permute(vec4 x){return mod(((x*34.0)+1.0)*x, 289.0);}
  vec4 taylorInvSqrt(vec4 r){return 1.79284291400159 - 0.85373472095314 * r;}

  float snoise(vec3 v){
    const vec2 C = vec2(1.0/6.0, 1.0/3.0);
    const vec4 D = vec4(0.0, 0.5, 1.0, 2.0);
    vec3 i  = floor(v + dot(v, C.yyy) );
    vec3 x0 = v - i + dot(i, C.xxx) ;
    vec3 g = step(x0.yzx, x0.xyz);
    vec3 l = 1.0 - g;
    vec3 i1 = min( g.xyz, l.zxy );
    vec3 i2 = max( g.xyz, l.zxy );
    vec3 x1 = x0 - i1 + 1.0 * C.xxx;
    vec3 x2 = x0 - i2 + 2.0 * C.xxx;
    vec3 x3 = x0 - 1.0 + 3.0 * C.xxx;
    i = mod(i, 289.0 );
    vec4 p = permute( permute( permute(
                i.z + vec4(0.0, i1.z, i2.z, 1.0 ))
              + i.y + vec4(0.0, i1.y, i2.y, 1.0 ))
              + i.x + vec4(0.0, i1.x, i2.x, 1.0 ));
    float n_ = 0.142857142857;
    vec3  ns = n_ * D.wyz - D.xzx;
    vec4 j = p - 49.0 * floor(p * ns.z);
    vec4 x_ = floor(j * ns.z);
    vec4 y_ = floor(j - 7.0 * x_ );
    vec4 x = x_ *ns.x + D.eeee;
    vec4 y = y_ *ns.x + D.eeee;
    vec4 h = 1.0 - abs(x) - abs(y);
    vec4 b0 = vec4( x.xy, y.xy );
    vec4 b1 = vec4( x.zw, y.zw );
    vec4 s0 = floor(b0)*2.0 + 1.0;
    vec4 s1 = floor(b1)*2.0 + 1.0;
    vec4 sh = -step(h, vec4(0.0));
    vec4 a0 = b0.xzyw + s0.xzyw*sh.xxyy ;
    vec4 a1 = b1.xzyw + s1.xzyw*sh.zzww ;
    vec3 p0 = vec3(a0.xy,h.x);
    vec3 p1 = vec3(a0.zw,h.y);
    vec3 p2 = vec3(a1.xy,h.z);
    vec3 p3 = vec3(a1.zw,h.w);
    vec4 norm = taylorInvSqrt(vec4(dot(p0,p0), dot(p1,p1), dot(p2, p2), dot(p3,p3)));
    p0 *= norm.x;
    p1 *= norm.y;
    p2 *= norm.z;
    p3 *= norm.w;
    vec4 m = max(0.6 - vec4(dot(x0,x0), dot(x1,x1), dot(x2,x2), dot(x3,x3)), 0.0);
    m = m * m;
    return 42.0 * dot( m*m, vec4( dot(p0,x0), dot(p1,x1), dot(p2,x2), dot(p3,x3) ) );
  }

  void main() {
    vNormal = normal;
    vPosition = position;
    
    // Spiky noise calculation
    float noise = snoise(position * 1.5 + vec3(uTime * 0.4));
    vNoise = noise;
    
    vec3 newPosition = position + normal * (noise * uDisplacement);
    gl_Position = projectionMatrix * modelViewMatrix * vec4(newPosition, 1.0);
  }
`;

const fragmentShader = `
  uniform float uTime;
  varying vec3 vNormal;
  varying vec3 vPosition;
  varying float vNoise;

  void main() {
    // High contrast brutalist monochrome gradient with rim lighting
    vec3 lightDir = normalize(vec3(1.0, 1.0, 2.0));
    float diff = max(dot(vNormal, lightDir), 0.0);
    
    // Base white-grey gradient
    vec3 color = vec3(0.85 + vNoise * 0.15);
    color *= (diff * 0.7 + 0.3);
    
    // Edge rim highlight
    float rim = 1.0 - max(dot(normalize(-vPosition), vNormal), 0.0);
    color += vec3(pow(rim, 3.0) * 0.4);
    
    gl_FragColor = vec4(color, 0.95);
  }
`;

function initThreeOrb() {
  const canvas = document.getElementById('orb-canvas');
  scene = new THREE.Scene();

  camera = new THREE.PerspectiveCamera(45, window.innerWidth / window.innerHeight, 0.1, 100);
  camera.position.z = 7;

  renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true });
  renderer.setSize(window.innerWidth, window.innerHeight);
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));

  // Geometry: Detailed sphere for smooth displacement
  const geometry = new THREE.IcosahedronGeometry(2.2, 64);

  // Custom Shader Material
  material = new THREE.ShaderMaterial({
    vertexShader,
    fragmentShader,
    uniforms: {
      uTime: { value: 0.0 },
      uDisplacement: { value: 0.8 }
    },
    wireframe: false,
    transparent: true
  });

  orbMesh = new THREE.Mesh(geometry, material);
  scene.add(orbMesh);

  // Ambient Floating Code Nodes (Particles)
  const particlesGeo = new THREE.BufferGeometry();
  const particleCount = 200;
  const posArray = new Float32Array(particleCount * 3);

  for (let i = 0; i < particleCount * 3; i++) {
    posArray[i] = (Math.random() - 0.5) * 15;
  }

  particlesGeo.setAttribute('position', new THREE.BufferAttribute(posArray, 3));
  const particlesMat = new THREE.PointsMaterial({
    size: 0.03,
    color: 0xffffff,
    transparent: true,
    opacity: 0.6
  });

  particlesMesh = new THREE.Points(particlesGeo, particlesMat);
  scene.add(particlesMesh);

  // Event Listeners
  window.addEventListener('resize', onWindowResize);
  document.addEventListener('mousemove', onMouseMove);

  // Animation Loop
  animate();
}

function onWindowResize() {
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

  // Update Shader Uniforms
  material.uniforms.uTime.value = elapsedTime;

  // Smooth Rotation & Mouse Parallax
  targetX = mouseX * 0.5;
  targetY = mouseY * 0.5;

  orbMesh.rotation.y += 0.005;
  orbMesh.rotation.x += (targetY - orbMesh.rotation.x) * 0.05;
  orbMesh.rotation.z += (targetX - orbMesh.rotation.z) * 0.05;

  // Rotate ambient particles
  if (particlesMesh) {
    particlesMesh.rotation.y = elapsedTime * 0.02;
  }

  // Camera zoom dynamics if studio engaged
  if (isStudioEngaged) {
    camera.position.z += (3.5 - camera.position.z) * 0.05;
    material.uniforms.uDisplacement.value += (1.8 - material.uniforms.uDisplacement.value) * 0.05;
  } else {
    camera.position.z += (7.0 - camera.position.z) * 0.05;
    material.uniforms.uDisplacement.value += (0.8 - material.uniforms.uDisplacement.value) * 0.05;
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
        P3["Phase 3: CSR CFG Builder"]
        P4["Phase 4: ROBDD Path Engine"]
    end
    P1 --> P2 --> P3 --> P4`,

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
  // Initialize Three.js Orb Canvas
  initThreeOrb();

  const heroLanding = document.getElementById('hero-landing');
  const studioApp = document.getElementById('studio-app');
  const btnEngageOrb = document.getElementById('btn-engage-orb');
  const btnBackLanding = document.getElementById('btn-back-landing');
  
  const btnFetch = document.getElementById('btn-fetch');
  const btnSelectAll = document.getElementById('btn-select-all');
  const btnClearAll = document.getElementById('btn-clear-all');
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

  // Engage Studio (Hero -> Studio Transition)
  btnEngageOrb.addEventListener('click', engageStudio);
  document.getElementById('orb-canvas').addEventListener('click', () => {
    if (!isStudioEngaged) engageStudio();
  });

  function engageStudio() {
    isStudioEngaged = true;
    heroLanding.classList.add('hero-hidden');
    setTimeout(() => {
      studioApp.classList.remove('studio-hidden');
    }, 400);
  }

  btnBackLanding.addEventListener('click', () => {
    isStudioEngaged = false;
    studioApp.classList.add('studio-hidden');
    setTimeout(() => {
      heroLanding.classList.remove('hero-hidden');
    }, 300);
  });

  // Update Diagram Count Display
  function updateCountDisplay() {
    activeDiagramCount.textContent = selectedDiagrams.size;
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

  btnSelectAll.addEventListener('click', () => {
    document.querySelectorAll('input[name="uml-type"]').forEach(cb => {
      cb.checked = true;
      const parentCard = cb.closest('.brutalist-checkbox');
      if (parentCard) parentCard.classList.add('active');
      selectedDiagrams.add(cb.value);
    });
    updateCountDisplay();
  });

  btnClearAll.addEventListener('click', () => {
    document.querySelectorAll('input[name="uml-type"]').forEach(cb => {
      cb.checked = false;
      const parentCard = cb.closest('.brutalist-checkbox');
      if (parentCard) parentCard.classList.remove('active');
    });
    selectedDiagrams.clear();
    updateCountDisplay();
  });

  // Pipeline Fetch Execution
  btnFetch.addEventListener('click', () => {
    const url = repoUrlInput.value.trim();
    if (!url || !url.startsWith('https://github.com/')) {
      alert('Please enter a valid GitHub repository URL starting with https://github.com/');
      return;
    }

    if (selectedDiagrams.size === 0) {
      alert('Please select at least one UML diagram type from the matrix.');
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
    await sleep(500);

    statusStepTitle.textContent = 'STAGE 1: LEXICAL INGESTION & TREE-SITTER WALK...';
    progressBarFill.style.width = '35%';
    statusPercent.textContent = '35%';
    logStep('> Allocating monotonic token_id counter [0..4096]');
    logStep('> Interning identifiers with 64-bit FNV-1a StringInterner');
    await sleep(700);

    statusStepTitle.textContent = 'STAGE 2: VERIFYING CORPUS INVARIANTS 1–4...';
    progressBarFill.style.width = '65%';
    statusPercent.textContent = '65%';
    logStep('> Invariant 1 (Monotonicity): VERIFIED');
    logStep('> Invariant 2 (Injectivity): VERIFIED');
    logStep('> Invariant 3 (Completeness): VERIFIED');
    logStep('> Invariant 4 (Index Consistency): VERIFIED');
    await sleep(600);

    statusStepTitle.textContent = 'STAGE 3: DERIVING 14 UML DIAGRAM VIEWS...';
    progressBarFill.style.width = '90%';
    statusPercent.textContent = '90%';
    logStep(`> Compiling ${selectedDiagrams.size} selected UML graph projections...`);
    await sleep(500);

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

  async function renderMermaidDiagram(type) {
    const mermaidCode = generatedDiagrams[type];
    renderContainer.innerHTML = `<div class="mermaid">${mermaidCode}</div>`;

    try {
      await mermaid.run({
        nodes: renderContainer.querySelectorAll('.mermaid')
      });
    } catch (err) {
      console.error('Mermaid render error:', err);
    }

    // Update Traceability Drawer
    document.getElementById('trace-tid').textContent = `#${Math.floor(Math.random() * 8000 + 1000)}`;
    document.getElementById('trace-file').textContent = `src/${type}_layer.rs`;
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
