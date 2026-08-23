/**
 * OpenHeart Source Editor & Line Synchronization Module
 * Powered by Monaco Editor API with Modern Light theme
 * Contains real Java source files from test_patterns_codebase & enterprise benchmarks.
 */

export class SourceEditorModule {
  constructor(containerId = 'monaco-container') {
    this.containerId = containerId;
    this.editor = null;
    this.currentDecorations = [];
    this.currentFile = 'VideoConversionFacade.java';

    this.sampleSourceFiles = {
      'VideoConversionFacade.java': `package com.patterns.structural.facade;

public class VideoConversionFacade {
    private AudioMixer audioMixer = new AudioMixer();
    private BitrateReader bitrateReader = new BitrateReader();

    public String convertVideo(String fileName, String format) {
        System.out.println("VideoConversionFacade: conversion started.");
        bitrateReader.read(fileName);
        audioMixer.fix();
        System.out.println("VideoConversionFacade: conversion completed.");
        return "ConvertedVideo." + format;
    }
}
`,
      'AudioMixer.java': `package com.patterns.structural.facade;

public class AudioMixer {
    public void fix() {
        System.out.println("AudioMixer: fixing audio...");
    }
}
`,
      'BitrateReader.java': `package com.patterns.structural.facade;

public class BitrateReader {
    public void read(String fileName) {
        System.out.println("BitrateReader: reading " + fileName);
    }
}
`,
      'Logistics.java': `package com.patterns.creational.factory;

public abstract class Logistics {
    public void planDelivery() {
        Transport transport = createTransport();
        transport.deliver();
    }

    public abstract Transport createTransport();
}
`,
      'RoadLogistics.java': `package com.patterns.creational.factory;

public class RoadLogistics extends Logistics {
    @Override
    public Transport createTransport() {
        return new Truck();
    }
}
`,
      'SeaLogistics.java': `package com.patterns.creational.factory;

public class SeaLogistics extends Logistics {
    @Override
    public Transport createTransport() {
        return new Ship();
    }
}
`,
      'Transport.java': `package com.patterns.creational.factory;

public interface Transport {
    void deliver();
}
`,
      'Truck.java': `package com.patterns.creational.factory;

public class Truck implements Transport {
    @Override
    public void deliver() {
        System.out.println("Delivering cargo by land in a box truck.");
    }
}
`,
      'Ship.java': `package com.patterns.creational.factory;

public class Ship implements Transport {
    @Override
    public void deliver() {
        System.out.println("Delivering cargo by sea in a container ship.");
    }
}
`,
      'PaymentProcessor.java': `package com.example.service;

import com.example.model.Order;
import com.example.repository.OrderRepository;

public class PaymentProcessor {
    private final OrderRepository repository;
    private final AuditService auditLog;

    public PaymentProcessor(OrderRepository repository, AuditService auditLog) {
        this.repository = repository;
        this.auditLog = auditLog;
    }

    public boolean processOrder(Order order) {
        if (order.getAmount() > 0 && order.isValid()) {
            this.repository.saveOrder(order);
            this.auditLog.record("ORDER_PROCESSED");
            return true;
        }
        return false;
    }
}
`,
      'Order.java': `package com.example.model;

public class Order {
    private String id;
    private double amount;

    public Order(String id, double amount) {
        this.id = id;
        this.amount = amount;
    }

    public double getAmount() {
        return this.amount;
    }

    public boolean isValid() {
        return this.amount > 0 && this.id != null;
    }
}
`,
      'OrderRepository.java': `package com.example.repository;

import com.example.model.Order;

public interface OrderRepository {
    void saveOrder(Order order);
    Order findById(String id);
}
`,
      'SqlOrderRepository.java': `package com.example.repository;

import com.example.model.Order;
import java.sql.Connection;

public class SqlOrderRepository implements OrderRepository {
    private Connection dbConn;

    @Override
    public void saveOrder(Order order) {
        // Execute SQL INSERT transaction
    }

    @Override
    public Order findById(String id) {
        return null;
    }
}
`,
      'AuditService.java': `package com.example.service;

public class AuditService {
    public void record(String event) {
        System.out.println("[AUDIT EVENT]: " + event);
    }
}
`
    };
  }

  init() {
    const container = document.getElementById(this.containerId);
    if (!container) return;

    if (window.monaco) {
      this.createMonacoInstance(container);
    } else {
      this.renderFallbackView('VideoConversionFacade.java', [7, 8, 9, 10, 11, 12]);
    }
  }

  createMonacoInstance(container) {
    monaco.editor.defineTheme('modern-light', {
      base: 'vs',
      inherit: true,
      rules: [
        { token: 'keyword', foreground: '0F172A', fontStyle: 'bold' },
        { token: 'type', foreground: '2563EB', fontStyle: 'bold' },
        { token: 'string', foreground: '059669' },
        { token: 'comment', foreground: '94A3B8', fontStyle: 'italic' },
        { token: 'identifier', foreground: '0F172A' }
      ],
      colors: {
        'editor.background': '#FFFFFF',
        'editor.foreground': '#0F172A',
        'editorLineNumber.foreground': '#94A3B8',
        'editorLineNumber.activeForeground': '#0F172A',
        'editor.selectionBackground': '#E2E8F0',
        'editor.lineHighlightBackground': '#F8FAFC'
      }
    });

    this.editor = monaco.editor.create(container, {
      value: this.sampleSourceFiles['VideoConversionFacade.java'],
      language: 'java',
      theme: 'modern-light',
      readOnly: true,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      fontSize: 12,
      fontFamily: 'JetBrains Mono, SF Mono, Consolas, monospace',
      lineHeight: 20
    });
  }

  highlightLines(fileName, lines = []) {
    const file = fileName || 'VideoConversionFacade.java';
    const code = this.sampleSourceFiles[file] || this.sampleSourceFiles['VideoConversionFacade.java'];
    this.currentFile = file;

    if (this.editor) {
      if (this.editor.getValue() !== code) {
        this.editor.setValue(code);
      }

      const newDecorations = lines.map(line => ({
        range: new monaco.Range(line, 1, line, 1),
        options: {
          isWholeLine: true,
          className: 'monaco-line-highlight-modern',
          linesDecorationsClassName: 'monaco-gutter-modern'
        }
      }));

      this.currentDecorations = this.editor.deltaDecorations(this.currentDecorations, newDecorations);
      if (lines.length > 0) {
        this.editor.revealLineInCenter(lines[0]);
      }
    } else {
      this.renderFallbackView(file, lines);
    }
  }

  renderFallbackView(fileName, lines = []) {
    const container = document.getElementById(this.containerId);
    if (!container) return;

    const code = this.sampleSourceFiles[fileName] || this.sampleSourceFiles['VideoConversionFacade.java'];
    const linesArr = code.split('\n');

    let html = `<div style="font-family: 'JetBrains Mono', monospace; font-size: 12px; line-height: 20px; padding: 12px; background: #FFF; height: 100%; overflow-y: auto;">`;
    linesArr.forEach((text, idx) => {
      const lineNum = idx + 1;
      const isHighlighted = lines.includes(lineNum);
      const bg = isHighlighted ? 'background: #FEF2F2; color: #EF4444; font-weight: 700;' : '';
      const gutter = `<span style="display:inline-block; width: 32px; color: #94A3B8; text-align: right; margin-right: 12px; user-select: none;">${lineNum}</span>`;
      html += `<div style="${bg}">${gutter}<span>${escapeHtml(text)}</span></div>`;
    });
    html += `</div>`;

    container.innerHTML = html;
  }
}

function escapeHtml(text) {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}
