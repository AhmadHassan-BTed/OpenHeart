package com.patterns.creational.builder;

public class Director {
    public Computer constructGamingComputer(ComputerBuilder builder) {
        return builder.buildCpu("Intel i9")
                      .buildRam("32GB")
                      .buildStorage("2TB NVMe")
                      .setGraphicsCard(true)
                      .build();
    }
}
