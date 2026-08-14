/**
 * OpenHeart Web Studio — Logger Module
 * High-cohesion logging subsystem for UI pipeline terminal & browser console.
 */

export class SystemLogger {
  constructor() {
    this.logsContainer = null;
  }

  init(containerId = 'status-logs') {
    this.logsContainer = document.getElementById(containerId);
  }

  log(msg, type = 'info') {
    const timestamp = new Date().toISOString().split('T')[1].slice(0, 8);
    const prefix = type === 'error' ? '[ERROR]' : type === 'warn' ? '[WARN]' : '[INFO]';
    const formatted = `[${timestamp}] ${prefix} ${msg}`;

    if (type === 'error') {
      console.error(formatted);
    } else if (type === 'warn') {
      console.warn(formatted);
    } else {
      console.log(formatted);
    }

    if (this.logsContainer) {
      const line = document.createElement('div');
      line.className = `log-line log-${type}`;
      line.textContent = `> ${msg}`;
      this.logsContainer.appendChild(line);
      this.logsContainer.scrollTop = this.logsContainer.scrollHeight;
    }
  }

  step(title) {
    this.log(`=== ${title} ===`, 'info');
  }

  error(msg) {
    this.log(msg, 'error');
  }

  warn(msg) {
    this.log(msg, 'warn');
  }

  clear() {
    if (this.logsContainer) {
      this.logsContainer.innerHTML = '';
    }
  }
}

export const Logger = new SystemLogger();
