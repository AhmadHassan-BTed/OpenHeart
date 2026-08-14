/**
 * OpenHeart Web Studio — 3D Morphing Spiky Orb Engine
 * Restored 1:1 original Three.js & Canvas2D WebGL Engine from commit 6a631ba.
 */

import { Logger } from './logger.js';

export class ThreeOrbEngine {
  constructor() {
    this.scene = null;
    this.camera = null;
    this.renderer = null;
    this.orbMesh = null;
    this.particleSystem = null;
    this.simplex = null;
    this.clock = null;
    this.originalPositions = null;
    this.originalNormals = null;
    this.isStudioEngaged = false;
    this.mouseX = 0;
    this.mouseY = 0;
    this.targetX = 0;
    this.targetY = 0;
  }

  init(canvasId = 'orb-canvas') {
    const canvas = document.getElementById(canvasId);
    if (!canvas) return;

    if (typeof THREE === 'undefined') {
      Logger.warn('[ORB] Three.js not loaded. Falling back to 2D Canvas Orb.');
      this.initCanvas2DOrb(canvas);
      return;
    }

    try {
      this.simplex = typeof SimplexNoise !== 'undefined' ? new SimplexNoise() : { noise3D: () => 0 };
      this.clock = new THREE.Clock();

      this.scene = new THREE.Scene();
      this.camera = new THREE.PerspectiveCamera(45, window.innerWidth / window.innerHeight, 0.1, 100);
      this.camera.position.set(0, 0, 6.5);

      this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true });
      this.renderer.setSize(window.innerWidth, window.innerHeight);
      this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));

      const ambientLight = new THREE.AmbientLight(0x333333, 1.0);
      this.scene.add(ambientLight);

      const mainLight = new THREE.DirectionalLight(0xffffff, 1.4);
      mainLight.position.set(5, 5, 5);
      this.scene.add(mainLight);

      const rimLight = new THREE.DirectionalLight(0x888888, 0.8);
      rimLight.position.set(-5, -5, -2);
      this.scene.add(rimLight);

      const pointLight = new THREE.PointLight(0xffffff, 1.2, 10);
      pointLight.position.set(0, 0, 3);
      this.scene.add(pointLight);

      const geometry = new THREE.IcosahedronGeometry(2.1, 32);
      const posAttr = geometry.attributes.position;
      const normAttr = geometry.attributes.normal;
      this.originalPositions = new Float32Array(posAttr.array);
      this.originalNormals = new Float32Array(normAttr.array);

      const material = new THREE.MeshStandardMaterial({
        color: 0xdddddd,
        roughness: 0.25,
        metalness: 0.15,
        flatShading: true,
        transparent: true,
        opacity: 0.95
      });
      this.orbMesh = new THREE.Mesh(geometry, material);
      this.scene.add(this.orbMesh);

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

      this.particleSystem = new THREE.Points(particleGeo, particleMat);
      this.scene.add(this.particleSystem);

      window.addEventListener('resize', () => this.onWindowResize());
      document.addEventListener('mousemove', (e) => this.onDocumentMouseMove(e));

      this.animate();
      Logger.log('[ORB] 3D WebGL Morphing Orb Initialized.');
    } catch (e) {
      Logger.error(`[ORB ERROR] ${e.message}`);
      this.initCanvas2DOrb(canvas);
    }
  }

  setStudioEngaged(engaged) {
    this.isStudioEngaged = engaged;
  }

  onWindowResize() {
    if (!this.camera || !this.renderer) return;
    this.camera.aspect = window.innerWidth / window.innerHeight;
    this.camera.updateProjectionMatrix();
    this.renderer.setSize(window.innerWidth, window.innerHeight);
  }

  onDocumentMouseMove(event) {
    this.mouseX = (event.clientX / window.innerWidth - 0.5) * 2;
    this.mouseY = (event.clientY / window.innerHeight - 0.5) * 2;
  }

  animate() {
    requestAnimationFrame(() => this.animate());
    const elapsedTime = this.clock ? this.clock.getElapsedTime() : 0;

    if (this.orbMesh && this.originalPositions && this.simplex) {
      const geo = this.orbMesh.geometry;
      const posAttr = geo.attributes.position;

      const spikeFactor = this.isStudioEngaged ? 1.6 : (0.45 + Math.sin(elapsedTime * 1.5) * 0.3);

      for (let i = 0; i < posAttr.count; i++) {
        const px = this.originalPositions[i * 3];
        const py = this.originalPositions[i * 3 + 1];
        const pz = this.originalPositions[i * 3 + 2];

        const nx = this.originalNormals[i * 3];
        const ny = this.originalNormals[i * 3 + 1];
        const nz = this.originalNormals[i * 3 + 2];

        const n1 = this.simplex.noise3D(px * 0.9 + elapsedTime * 0.4, py * 0.9 + elapsedTime * 0.4, pz * 0.9 + elapsedTime * 0.4);
        const n2 = this.simplex.noise3D(px * 2.2 - elapsedTime * 0.3, py * 2.2 - elapsedTime * 0.3, pz * 2.2 - elapsedTime * 0.3);
        const noiseVal = n1 * 0.7 + n2 * 0.3;

        const displacement = noiseVal * spikeFactor;

        posAttr.setXYZ(i, px + nx * displacement, py + ny * displacement, pz + nz * displacement);
      }

      posAttr.needsUpdate = true;
      geo.computeVertexNormals();

      this.targetX = this.mouseX * 0.4;
      this.targetY = this.mouseY * 0.4;

      this.orbMesh.rotation.y += 0.005;
      this.orbMesh.rotation.x += (this.targetY - this.orbMesh.rotation.x) * 0.05;
      this.orbMesh.rotation.z += (this.targetX - this.orbMesh.rotation.z) * 0.05;
    }

    if (this.particleSystem) {
      this.particleSystem.rotation.y = elapsedTime * 0.012;
    }

    if (this.camera) {
      if (this.isStudioEngaged) {
        this.camera.position.z += (3.5 - this.camera.position.z) * 0.05;
      } else {
        this.camera.position.z += (6.5 - this.camera.position.z) * 0.05;
      }
    }

    if (this.renderer && this.scene && this.camera) {
      this.renderer.render(this.scene, this.camera);
    }
  }

  initCanvas2DOrb(canvas) {
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    function resize() {
      canvas.width = window.innerWidth;
      canvas.height = window.innerHeight;
    }
    window.addEventListener('resize', resize);
    resize();

    let angle = 0;
    const draw = () => {
      requestAnimationFrame(draw);
      angle += 0.012;
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      const cx = canvas.width / 2;
      const cy = canvas.height / 2;
      const radius = Math.min(canvas.width, canvas.height) * 0.18;

      ctx.save();
      ctx.translate(cx, cy);

      ctx.beginPath();
      const spikes = 64;
      for (let i = 0; i < spikes; i++) {
        const a = (i / spikes) * Math.PI * 2;
        const rOffset = Math.sin(a * 6 + angle * 2) * 22 + Math.cos(a * 9 - angle * 3) * 14;
        const r = radius + rOffset;
        const x = Math.cos(a) * r;
        const y = Math.sin(a) * r;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.closePath();

      const grad = ctx.createRadialGradient(0, 0, radius * 0.2, 0, 0, radius * 1.5);
      grad.addColorStop(0, 'rgba(220, 220, 220, 0.85)');
      grad.addColorStop(0.5, 'rgba(150, 150, 150, 0.35)');
      grad.addColorStop(1, 'rgba(0, 0, 0, 0)');
      ctx.fillStyle = grad;
      ctx.fill();
      ctx.strokeStyle = '#ffffff';
      ctx.lineWidth = 2.5;
      ctx.stroke();

      ctx.restore();
    };
    draw();
  }
}

export const OrbEngine = new ThreeOrbEngine();
