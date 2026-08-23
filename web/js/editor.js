/**
 * OpenHeart Source Editor & Line Synchronization Module
 * Powered by Monaco Editor API with Modern Light theme
 * Contains ALL 35 real Java source files from test_patterns_codebase.
 */

export class SourceEditorModule {
  constructor(containerId = "monaco-container") {
    this.containerId = containerId;
    this.editor = null;
    this.currentDecorations = [];
    this.currentFile = "VideoConversionFacade.java";

    this.sampleSourceFiles = {
      "MediaPlayer.java": "package com.patterns.structural.adapter;\n\npublic interface MediaPlayer {\n    void play(String audioType, String fileName);\n}\n",
      "AdvancedMediaPlayer.java": "package com.patterns.structural.adapter;\n\npublic interface AdvancedMediaPlayer {\n    void playVlc(String fileName);\n    void playMp4(String fileName);\n}\n",
      "AudioPlayer.java": "package com.patterns.structural.adapter;\n\npublic class AudioPlayer implements MediaPlayer {\n    private MediaAdapter mediaAdapter;\n\n    @Override\n    public void play(String audioType, String fileName) {\n        if (audioType.equalsIgnoreCase(\"mp3\")) {\n            System.out.println(\"Playing mp3 file: \" + fileName);\n        } else if (audioType.equalsIgnoreCase(\"vlc\") || audioType.equalsIgnoreCase(\"mp4\")) {\n            mediaAdapter = new MediaAdapter(audioType);\n            mediaAdapter.play(audioType, fileName);\n        }\n    }\n}\n",
      "VlcPlayer.java": "package com.patterns.structural.adapter;\n\npublic class VlcPlayer implements AdvancedMediaPlayer {\n    @Override\n    public void playVlc(String fileName) {\n        System.out.println(\"Playing vlc file: \" + fileName);\n    }\n\n    @Override\n    public void playMp4(String fileName) {}\n}\n",
      "Mp4Player.java": "package com.patterns.structural.adapter;\n\npublic class Mp4Player implements AdvancedMediaPlayer {\n    @Override\n    public void playVlc(String fileName) {}\n\n    @Override\n    public void playMp4(String fileName) {\n        System.out.println(\"Playing mp4 file: \" + fileName);\n    }\n}\n",
      "MediaAdapter.java": "package com.patterns.structural.adapter;\n\npublic class MediaAdapter implements MediaPlayer {\n    private AdvancedMediaPlayer advancedMusicPlayer;\n\n    public MediaAdapter(String audioType) {\n        if (audioType.equalsIgnoreCase(\"vlc\")) {\n            advancedMusicPlayer = new VlcPlayer();\n        } else if (audioType.equalsIgnoreCase(\"mp4\")) {\n            advancedMusicPlayer = new Mp4Player();\n        }\n    }\n\n    @Override\n    public void play(String audioType, String fileName) {\n        if (audioType.equalsIgnoreCase(\"vlc\")) {\n            advancedMusicPlayer.playVlc(fileName);\n        } else if (audioType.equalsIgnoreCase(\"mp4\")) {\n            advancedMusicPlayer.playMp4(fileName);\n        }\n    }\n}\n",
      "AudioMixer.java": "package com.patterns.structural.facade;\n\npublic class AudioMixer {\n    public void fix() {\n        System.out.println(\"AudioMixer: fixing audio track.\");\n    }\n}\n",
      "BitrateReader.java": "package com.patterns.structural.facade;\n\npublic class BitrateReader {\n    public void read(String fileName) {\n        System.out.println(\"BitrateReader: reading \" + fileName);\n    }\n}\n",
      "VideoConversionFacade.java": "package com.patterns.structural.facade;\n\npublic class VideoConversionFacade {\n    private AudioMixer audioMixer = new AudioMixer();\n    private BitrateReader bitrateReader = new BitrateReader();\n\n    public String convertVideo(String fileName, String format) {\n        System.out.println(\"VideoConversionFacade: conversion started.\");\n        bitrateReader.read(fileName);\n        audioMixer.fix();\n        System.out.println(\"VideoConversionFacade: conversion completed.\");\n        return \"ConvertedVideo.\" + format;\n    }\n}\n",
      "Beverage.java": "package com.patterns.structural.decorator;\n\npublic interface Beverage {\n    String getDescription();\n    double cost();\n}\n",
      "CondimentDecorator.java": "package com.patterns.structural.decorator;\n\npublic abstract class CondimentDecorator implements Beverage {\n    protected Beverage beverage;\n\n    public CondimentDecorator(Beverage beverage) {\n        this.beverage = beverage;\n    }\n\n    @Override\n    public abstract String getDescription();\n}\n",
      "Espresso.java": "package com.patterns.structural.decorator;\n\npublic class Espresso implements Beverage {\n    @Override\n    public String getDescription() {\n        return \"Espresso\";\n    }\n\n    @Override\n    public double cost() {\n        return 1.99;\n    }\n}\n",
      "Mocha.java": "package com.patterns.structural.decorator;\n\npublic class Mocha extends CondimentDecorator {\n    public Mocha(Beverage beverage) {\n        super(beverage);\n    }\n\n    @Override\n    public String getDescription() {\n        return beverage.getDescription() + \", Mocha\";\n    }\n\n    @Override\n    public double cost() {\n        return beverage.cost() + 0.20;\n    }\n}\n",
      "Whip.java": "package com.patterns.structural.decorator;\n\npublic class Whip extends CondimentDecorator {\n    public Whip(Beverage beverage) {\n        super(beverage);\n    }\n\n    @Override\n    public String getDescription() {\n        return beverage.getDescription() + \", Whip\";\n    }\n\n    @Override\n    public double cost() {\n        return beverage.cost() + 0.10;\n    }\n}\n",
      "NewsChannel.java": "package com.patterns.behavioral.observer;\n\npublic class NewsChannel implements Observer {\n    private String news;\n\n    @Override\n    public void update(String news) {\n        this.news = news;\n        System.out.println(\"NewsChannel received: \" + this.news);\n    }\n}\n",
      "NewsAgency.java": "package com.patterns.behavioral.observer;\n\nimport java.util.ArrayList;\nimport java.util.List;\n\npublic class NewsAgency implements Subject {\n    private String news;\n    private List<Observer> observers = new ArrayList<>();\n\n    public void setNews(String news) {\n        this.news = news;\n        notifyObservers();\n    }\n\n    @Override\n    public void attach(Observer observer) {\n        this.observers.add(observer);\n    }\n\n    @Override\n    public void detach(Observer observer) {\n        this.observers.remove(observer);\n    }\n\n    @Override\n    public void notifyObservers() {\n        for (Observer observer : this.observers) {\n            observer.update(this.news);\n        }\n    }\n}\n",
      "Observer.java": "package com.patterns.behavioral.observer;\n\npublic interface Observer {\n    void update(String news);\n}\n",
      "Subject.java": "package com.patterns.behavioral.observer;\n\npublic interface Subject {\n    void attach(Observer observer);\n    void detach(Observer observer);\n    void notifyObservers();\n}\n",
      "PdfDataMiner.java": "package com.patterns.behavioral.templatemethod;\n\npublic class PdfDataMiner extends DataMiner {\n    @Override\n    public void openFile(String path) {\n        System.out.println(\"Opening PDF: \" + path);\n    }\n\n    @Override\n    public void extractData() {\n        System.out.println(\"Extracting raw PDF bytes.\");\n    }\n\n    @Override\n    public void parseData() {\n        System.out.println(\"Parsing PDF structure.\");\n    }\n}\n",
      "DataMiner.java": "package com.patterns.behavioral.templatemethod;\n\npublic abstract class DataMiner {\n    public final void mine(String path) {\n        openFile(path);\n        extractData();\n        parseData();\n        closeFile();\n    }\n\n    public abstract void openFile(String path);\n    public abstract void extractData();\n    public abstract void parseData();\n\n    public void closeFile() {\n        System.out.println(\"File closed.\");\n    }\n}\n",
      "CsvDataMiner.java": "package com.patterns.behavioral.templatemethod;\n\npublic class CsvDataMiner extends DataMiner {\n    @Override\n    public void openFile(String path) {\n        System.out.println(\"Opening CSV: \" + path);\n    }\n\n    @Override\n    public void extractData() {\n        System.out.println(\"Extracting CSV lines.\");\n    }\n\n    @Override\n    public void parseData() {\n        System.out.println(\"Parsing CSV comma records.\");\n    }\n}\n",
      "CreditCardStrategy.java": "package com.patterns.behavioral.strategy;\n\npublic class CreditCardStrategy implements PaymentStrategy {\n    private String name;\n    private String cardNumber;\n\n    public CreditCardStrategy(String name, String cardNumber) {\n        this.name = name;\n        this.cardNumber = cardNumber;\n    }\n\n    @Override\n    public void pay(int amount) {\n        System.out.println(amount + \" paid with credit card.\");\n    }\n}\n",
      "ShoppingCart.java": "package com.patterns.behavioral.strategy;\n\npublic class ShoppingCart {\n    private PaymentStrategy paymentStrategy;\n\n    public void setPaymentStrategy(PaymentStrategy strategy) {\n        this.paymentStrategy = strategy;\n    }\n\n    public void checkout(int amount) {\n        this.paymentStrategy.pay(amount);\n    }\n}\n",
      "PaymentStrategy.java": "package com.patterns.behavioral.strategy;\n\npublic interface PaymentStrategy {\n    void pay(int amount);\n}\n",
      "PaypalStrategy.java": "package com.patterns.behavioral.strategy;\n\npublic class PaypalStrategy implements PaymentStrategy {\n    private String emailId;\n\n    public PaypalStrategy(String email) {\n        this.emailId = email;\n    }\n\n    @Override\n    public void pay(int amount) {\n        System.out.println(amount + \" paid using Paypal.\");\n    }\n}\n",
      "DatabaseConnectionPool.java": "package com.patterns.creational.singleton;\n\npublic class DatabaseConnectionPool {\n    private static DatabaseConnectionPool instance;\n    private int maxConnections;\n    private boolean isInitialized;\n\n    private DatabaseConnectionPool() {\n        this.maxConnections = 10;\n        this.isInitialized = true;\n    }\n\n    public static DatabaseConnectionPool getInstance() {\n        if (instance == null) {\n            instance = new DatabaseConnectionPool();\n        }\n        return instance;\n    }\n\n    public void executeQuery(String sql) {\n        System.out.println(\"Executing: \" + sql);\n    }\n}\n",
      "ComputerBuilder.java": "package com.patterns.creational.builder;\n\npublic class ComputerBuilder {\n    private Computer computer;\n\n    public ComputerBuilder() {\n        this.computer = new Computer();\n    }\n\n    public ComputerBuilder buildCpu(String cpu) {\n        this.computer.setCpu(cpu);\n        return this;\n    }\n\n    public ComputerBuilder buildRam(String ram) {\n        this.computer.setRam(ram);\n        return this;\n    }\n\n    public ComputerBuilder buildStorage(String storage) {\n        this.computer.setStorage(storage);\n        return this;\n    }\n\n    public ComputerBuilder setGraphicsCard(boolean enabled) {\n        this.computer.setGraphicsCardEnabled(enabled);\n        return this;\n    }\n\n    public Computer build() {\n        return this.computer;\n    }\n}\n",
      "Computer.java": "package com.patterns.creational.builder;\n\npublic class Computer {\n    private String cpu;\n    private String ram;\n    private String storage;\n    private boolean graphicsCardEnabled;\n\n    public void setCpu(String cpu) { this.cpu = cpu; }\n    public void setRam(String ram) { this.ram = ram; }\n    public void setStorage(String storage) { this.storage = storage; }\n    public void setGraphicsCardEnabled(boolean enabled) { this.graphicsCardEnabled = enabled; }\n\n    public String getCpu() { return cpu; }\n    public String getRam() { return ram; }\n    public String getStorage() { return storage; }\n    public boolean isGraphicsCardEnabled() { return graphicsCardEnabled; }\n}\n",
      "Director.java": "package com.patterns.creational.builder;\n\npublic class Director {\n    public Computer constructGamingComputer(ComputerBuilder builder) {\n        return builder.buildCpu(\"Intel i9\")\n                      .buildRam(\"32GB\")\n                      .buildStorage(\"2TB NVMe\")\n                      .setGraphicsCard(true)\n                      .build();\n    }\n}\n",
      "Transport.java": "package com.patterns.creational.factory;\n\npublic interface Transport {\n    void deliver();\n}\n",
      "Truck.java": "package com.patterns.creational.factory;\n\npublic class Truck implements Transport {\n    @Override\n    public void deliver() {\n        System.out.println(\"Deliver by land in a box.\");\n    }\n}\n",
      "Ship.java": "package com.patterns.creational.factory;\n\npublic class Ship implements Transport {\n    @Override\n    public void deliver() {\n        System.out.println(\"Deliver by sea in a container.\");\n    }\n}\n",
      "RoadLogistics.java": "package com.patterns.creational.factory;\n\npublic class RoadLogistics extends Logistics {\n    @Override\n    public Transport createTransport() {\n        return new Truck();\n    }\n}\n",
      "SeaLogistics.java": "package com.patterns.creational.factory;\n\npublic class SeaLogistics extends Logistics {\n    @Override\n    public Transport createTransport() {\n        return new Ship();\n    }\n}\n",
      "Logistics.java": "package com.patterns.creational.factory;\n\npublic abstract class Logistics {\n    public void planDelivery() {\n        Transport transport = createTransport();\n        transport.deliver();\n    }\n\n    public abstract Transport createTransport();\n}\n"
};
  }

  init() {
    const container = document.getElementById(this.containerId);
    if (!container) return;

    if (window.monaco) {
      this.createMonacoInstance(container);
    } else {
      this.renderFallbackView("VideoConversionFacade.java", [7, 8, 9, 10, 11, 12]);
    }
  }

  createMonacoInstance(container) {
    monaco.editor.defineTheme("modern-light", {
      base: "vs",
      inherit: true,
      rules: [
        { token: "keyword", foreground: "0F172A", fontStyle: "bold" },
        { token: "type", foreground: "2563EB", fontStyle: "bold" },
        { token: "string", foreground: "059669" },
        { token: "comment", foreground: "94A3B8", fontStyle: "italic" },
        { token: "identifier", foreground: "0F172A" }
      ],
      colors: {
        "editor.background": "#FFFFFF",
        "editor.foreground": "#0F172A",
        "editorLineNumber.foreground": "#94A3B8",
        "editorLineNumber.activeForeground": "#0F172A",
        "editor.selectionBackground": "#E2E8F0",
        "editor.lineHighlightBackground": "#F8FAFC"
      }
    });

    this.editor = monaco.editor.create(container, {
      value: this.sampleSourceFiles["VideoConversionFacade.java"] || Object.values(this.sampleSourceFiles)[0],
      language: "java",
      theme: "modern-light",
      readOnly: true,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      fontSize: 12,
      fontFamily: "JetBrains Mono, SF Mono, Consolas, monospace",
      lineHeight: 20
    });
  }

  highlightLines(fileName, lines = []) {
    const file = fileName || "VideoConversionFacade.java";
    const code = this.sampleSourceFiles[file] || this.sampleSourceFiles["VideoConversionFacade.java"] || Object.values(this.sampleSourceFiles)[0];
    this.currentFile = file;

    if (this.editor) {
      if (this.editor.getValue() !== code) {
        this.editor.setValue(code);
      }

      const newDecorations = lines.map(line => ({
        range: new monaco.Range(line, 1, line, 1),
        options: {
          isWholeLine: true,
          className: "monaco-line-highlight-modern",
          linesDecorationsClassName: "monaco-gutter-modern"
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

    const code = this.sampleSourceFiles[fileName] || this.sampleSourceFiles["VideoConversionFacade.java"] || Object.values(this.sampleSourceFiles)[0];
    const linesArr = code.split("\n");

    let html = `<div style="font-family: 'JetBrains Mono', monospace; font-size: 12px; line-height: 20px; padding: 12px; background: #FFF; height: 100%; overflow-y: auto;">`;
    linesArr.forEach((text, idx) => {
      const lineNum = idx + 1;
      const isHighlighted = lines.includes(lineNum);
      const bg = isHighlighted ? "background: #FEF2F2; color: #EF4444; font-weight: 700;" : "";
      const gutter = `<span style="display:inline-block; width: 32px; color: #94A3B8; text-align: right; margin-right: 12px; user-select: none;">${lineNum}</span>`;
      html += `<div style="${bg}">${gutter}<span>${escapeHtml(text)}</span></div>`;
    });
    html += `</div>`;

    container.innerHTML = html;
  }
}

function escapeHtml(text) {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}
