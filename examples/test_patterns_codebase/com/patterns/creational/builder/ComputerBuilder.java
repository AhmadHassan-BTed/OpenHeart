package com.patterns.creational.builder;

public class ComputerBuilder {
    private Computer computer;

    public ComputerBuilder() {
        this.computer = new Computer();
    }

    public ComputerBuilder buildCpu(String cpu) {
        this.computer.setCpu(cpu);
        return this;
    }

    public ComputerBuilder buildRam(String ram) {
        this.computer.setRam(ram);
        return this;
    }

    public ComputerBuilder buildStorage(String storage) {
        this.computer.setStorage(storage);
        return this;
    }

    public ComputerBuilder setGraphicsCard(boolean enabled) {
        this.computer.setGraphicsCardEnabled(enabled);
        return this;
    }

    public Computer build() {
        return this.computer;
    }
}
