package com.patterns.creational.builder;

public class Computer {
    private String cpu;
    private String ram;
    private String storage;
    private boolean graphicsCardEnabled;

    public void setCpu(String cpu) { this.cpu = cpu; }
    public void setRam(String ram) { this.ram = ram; }
    public void setStorage(String storage) { this.storage = storage; }
    public void setGraphicsCardEnabled(boolean enabled) { this.graphicsCardEnabled = enabled; }

    public String getCpu() { return cpu; }
    public String getRam() { return ram; }
    public String getStorage() { return storage; }
    public boolean isGraphicsCardEnabled() { return graphicsCardEnabled; }
}
